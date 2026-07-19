//! The shared statement/expression dispatcher (`22UpdatePlan.md`
//! Pillar 3, Parts 2-3 — the deferred half of the backend factory,
//! executed as a follow-on to that plan's own M3, which shipped only
//! Part 1, the `scan.rs` scanner).
//!
//! The single structural fact this module turns on, read directly off
//! both bundled backends' pre-existing `lower.rs` files: most
//! [`HirExpr`] shapes are already lowerable as a nested value in
//! either target language — locals, literals, builtins, field access,
//! indexing, record construction, unary/binary operators, and every
//! "simple" verb call (`cache.*`, `object_store.*`, `email.send`,
//! `search.*`, `http.call`, plus `db.get`, which yields a value in
//! both hosts without any statement decomposition). Exactly four
//! shapes are not: `If`, `Match`, a `VerbCall` naming
//! `DbInsert`/`DbUpdate`/`DbDelete`, and `Query`
//! (`DbQuery`/`DbCount`/`DbDeleteWhere`). Rust lowers all four as
//! plain nested expressions (real `if`/`match`/block-expressions);
//! Python cannot (Python statements aren't expressions) and instead
//! decomposes them into a statement sequence ending in an
//! assignment/return/discard — see [`Dest`].
//!
//! This module owns exactly that split, generically over
//! [`super::HostSyntax`]:
//!
//! - [`lower_scalar`] — the ~90%-shared recursive walk over every
//!   non-block-shaped [`HirExpr`], calling one [`super::HostSyntax`]
//!   leaf per node. Both orientations funnel through this for
//!   anything that isn't one of the four special shapes.
//! - [`lower_expr_any`] — `Expression`-orientation only: handles the
//!   four special shapes as plain nested values, else delegates to
//!   [`lower_scalar`]. This is a generic-ized `rust_expr`.
//! - [`lower_tail`] — `Statement`-orientation only: handles the four
//!   special shapes by decomposing into lines applied to a [`Dest`],
//!   else lowers via [`lower_scalar`] and applies the `Dest`. This is
//!   a generic-ized Python `lower_tail`.
//! - [`lower_block_expr`]/[`lower_stmt_expr`] and
//!   [`lower_block_stmt`]/[`lower_stmt`] — the two orientations'
//!   statement-sequencing (`Let`/`Expr`/`Return`/`Fail`/`Publish`/
//!   `Transaction`), each walking `stmts`, truncating after a
//!   diverging statement exactly as both backends already did
//!   independently.
//! - [`lower_body_expr`]/[`lower_body_stmt`] — the two entry points a
//!   backend's own `render()` calls instead of a private `lower_body`.
//!
//! Every leaf receives already-lowered child strings, plus whatever
//! stable IR ids ([`TableId`]/[`RecordId`]/[`NodeId`]) it needs to
//! resolve target-specific detail (SQL text vs. ORM calls, a record's
//! field list, ...) — see `host_syntax.rs`'s own doc comment for why
//! this context-carrying shape was chosen over forcing everything
//! through pure strings.
//!
//! Byte-identical-golden discipline governs this whole module: every
//! shape below was derived by reading each backend's *current* code
//! line by line, not by designing a "clean" contract first and hoping
//! it matched. Several real asymmetries surfaced doing that (kept
//! exactly, not "fixed"): a bare `EnumLit` renders as a plain quoted
//! string for Python everywhere, but only resolves for Rust at three
//! use sites (record-field value, enum comparison, predicate term)
//! where the *dispatcher* — not the leaf — can recover a named type or
//! a raw bind value; `db.get` is a scalar in both hosts today, never
//! statement-decomposed; `cache.set`/`object_store.put` branch 2-way
//! on a Record-vs-not payload type, while `search.index`/`http.call`
//! branch 3-way (Record/Json/else) — a real, pre-existing divergence
//! this refactor must not quietly unify.

use super::host_syntax::{stream_subject, HostSyntax, IndexKey, LoweredPredTerm, MatchArm};
use super::{field_access_enum_name, LoweredPredicate, PredValue};
use ciac_ir::{
    BinOp, Builtin, HandlerBody, HirExpr, HirPredicate, HirStmt, HirType, NormalizedIr, RecordId,
    Verb,
};
use heck::ToPascalCase;

/// How an `Expression`-oriented block's own trailing bare-`Expr`
/// statement's value is shaped — `rust_block`/`rust_stmt`'s `Tail`,
/// generic-ized. Every non-last statement in a block is always lowered
/// with `None` regardless of what the *block's own* tail needs — see
/// [`lower_block_expr`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wrap {
    /// A non-tail (mid-block) statement, or a block a caller wants
    /// treated as a plain statement sequence with no tail value at all
    /// — `transaction { }`'s inner block, which lowers as ordinary
    /// statements regardless of position (its own last statement is
    /// still `;`-terminated and discarded, exactly like every other
    /// statement in it).
    None,
    /// A nested `if`/`match` branch's own tail: the bare value,
    /// feeding the enclosing expression.
    Plain,
    /// The handler function body's own tail: needs `Ok(..)` wrapping
    /// (`handle()` returns a `Result`).
    Wrapped,
}

/// Where a `Statement`-oriented block's tail value is consumed — the
/// direct generalization of the pre-factory Python-only `Sink`.
#[derive(Debug, Clone)]
pub enum Dest {
    Assign(String),
    Return,
    Discard,
}

/// Applies `dest` to an already-lowered scalar `value`, pushing
/// exactly one line — the generalized `apply_sink`. Exposed so a
/// host's own statement-shaped leaves (`db_update_tail`, `query_tail`,
/// ...) can reuse it instead of re-deriving the three-way dispatch —
/// `?Sized` so `HostSyntax`'s own default `..._tail` methods (M4's
/// error-idiom amendment) can call it on `&self` without adding a
/// `Self: Sized` bound to the trait itself.
pub fn apply_dest<H: HostSyntax + ?Sized>(
    host: &H,
    dest: &Dest,
    value: &str,
    indent: &str,
    out: &mut Vec<String>,
) {
    match dest {
        Dest::Assign(name) => out.push(host.assign(name, value, indent)),
        Dest::Return => out.push(host.return_stmt(Some(value), indent)),
        Dest::Discard => out.push(host.discard_stmt(value, indent)),
    }
}

/// `rust_binary`/`rust_expr`'s depth-scan paren-stripper, moved
/// verbatim: every composite value wraps in `(..)` so it composes
/// safely when nested, which makes the outermost pair redundant in a
/// condition position — and redundant parens are `unused_parens`,
/// promoted to a hard error by `-D warnings`. A plain function, not a
/// leaf or an automatic dispatcher-applied wrap: Rust's `if_expr`/
/// `match_expr` call this on the condition/scrutinee string they
/// receive; Python's equivalents simply don't (Python tolerates
/// redundant parens, and "improving" that here would violate the
/// byte-identical bar this refactor is held to).
pub fn strip_outer_parens(s: String) -> String {
    if !s.starts_with('(') || !s.ends_with(')') {
        return s;
    }
    let mut depth = 0i32;
    for (i, ch) in s.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 && i != s.len() - 1 {
                    // The first `(` closes before the string ends, so
                    // the leading/trailing chars aren't a single
                    // wrapping pair (e.g. `(a) + (b)`) — leave it alone.
                    return s;
                }
            }
            _ => {}
        }
    }
    s[1..s.len() - 1].to_owned()
}

/// `rust_float_lit`, moved verbatim: `format!("{f}")` on `1.0_f64`
/// prints `"1"`, which Rust (unlike Python) parses back as an integer
/// literal, not a float — so a Rust float literal must always contain
/// a `.`/`e`/`E` (or be a named special value).
pub fn fidelity_checked_float(f: f64) -> String {
    let s = format!("{f}");
    if s.contains(['.', 'e', 'E']) || s == "inf" || s == "-inf" || s == "NaN" {
        s
    } else {
        format!("{s}.0")
    }
}

/// `rust_expr`'s local `indent` helper, moved verbatim: prefixes every
/// line of `text` with `pad`. A plain shared utility (not a leaf) —
/// both existing backends indent lines the same textual way, they
/// only disagree on *when* to call it (Rust does so inside `if_expr`/
/// `match_expr`; Python threads an `indent: &str` parameter through
/// its whole statement walk instead).
pub fn indent_lines(text: &str, pad: &str) -> String {
    text.lines()
        .map(|line| format!("{pad}{line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Identical in both pre-factory backends: a statement diverges (every
/// path past it is unreachable) if it's a `return`/`fail`, or a
/// `let`/bare-expr whose value's type is [`HirType::Never`] (every
/// path through it already returned/failed).
fn stmt_diverges(stmt: &HirStmt) -> bool {
    match stmt {
        HirStmt::Return(_) | HirStmt::Fail { .. } => true,
        HirStmt::Let { value, .. } | HirStmt::Expr(value) => value.ty() == HirType::Never,
        HirStmt::Publish { .. } | HirStmt::Transaction { .. } => false,
    }
}

/// `slot_name`, moved verbatim (duplicated identically in both
/// pre-factory backends): a [`HirExpr::Local`]'s declared name when it
/// names a parameter, else a synthesized `v<slot>` for a `let` local.
fn local_name(body: &HandlerBody, slot: u32) -> String {
    let slot = slot as usize;
    if slot < body.params.len() {
        body.params[slot].0.clone()
    } else {
        format!("v{slot}")
    }
}

/// Every [`HirExpr`] shape both orientations can render as a plain
/// nested value. A bare [`HirExpr::EnumLit`] is handed to
/// [`HostSyntax::enum_literal`] with `enum_name: None` — valid for
/// Python (which ignores it), a documented panic for Rust (which
/// needs a name it can only get from an enclosing use-site the
/// *caller* must have already special-cased before recursing here;
/// see [`lower_record_cons`]/[`lower_binary`]/[`lower_predicate`]).
/// Panics (mirroring both backends' existing `unreachable!` arms) on
/// the four block-shaped forms — an `Expression`-oriented caller must
/// reach those through [`lower_expr_any`], a `Statement`-oriented one
/// through [`lower_tail`], never here.
pub fn lower_scalar<H: HostSyntax>(
    host: &H,
    ir: &NormalizedIr,
    body: &HandlerBody,
    expr: &HirExpr,
) -> String {
    match expr {
        HirExpr::If { .. } | HirExpr::Match { .. } => {
            unreachable!(
                "control flow must be lowered via lower_expr_any/lower_tail, not nested as a scalar"
            )
        }
        HirExpr::VerbCall {
            verb: Verb::DbInsert(_) | Verb::DbUpdate(_) | Verb::DbDelete(_),
            ..
        } => unreachable!(
            "db.insert/update/delete must be lowered via lower_expr_any/lower_tail, not nested as a scalar"
        ),
        HirExpr::Query { .. } => unreachable!(
            "db.query/count/delete_where must be lowered via lower_expr_any/lower_tail, not nested as a scalar"
        ),
        HirExpr::Local { slot, .. } => host.local(&local_name(body, *slot)),
        HirExpr::IntLit(n) => host.int_lit(*n),
        HirExpr::FloatLit(f) => host.float_lit(*f),
        HirExpr::StrLit(s) => host.str_lit(s),
        HirExpr::BoolLit(b) => host.bool_lit(*b),
        HirExpr::BuiltinCall(Builtin::UuidNew) => host.uuid_new(),
        HirExpr::BuiltinCall(Builtin::TimestampNow) => host.timestamp_now(),
        HirExpr::EnumLit { variant, .. } => host.enum_literal(None, variant),
        HirExpr::FieldAccess { base, field, .. } => {
            let base_s = lower_scalar(host, ir, body, base);
            host.field_access(&base_s, field)
        }
        HirExpr::Index { base, index } => {
            let base_s = lower_scalar(host, ir, body, base);
            let key = match index.as_ref() {
                HirExpr::StrLit(s) => IndexKey::StrKey(s),
                other => IndexKey::Expr(lower_scalar(host, ir, body, other)),
            };
            host.index(&base_s, key)
        }
        HirExpr::RecordCons {
            record,
            base_value,
            fields,
        } => lower_record_cons(host, ir, body, *record, base_value.as_deref(), fields),
        HirExpr::Binary { op, lhs, rhs, .. } => lower_binary(host, ir, body, *op, lhs, rhs),
        HirExpr::Unary { op, expr, .. } => {
            let inner = lower_scalar(host, ir, body, expr);
            host.unary(*op, &inner)
        }
        HirExpr::VerbCall { verb, args, .. } => lower_simple_verb(host, ir, body, *verb, args),
    }
}

/// Record construction (fresh or functional-update): resolves a bare
/// `EnumLit` field value to its named enum type (`RecordName` +
/// `PascalCase(field)`, exactly as `build_record`/the scanner already
/// compute it) *before* recursing, then applies
/// [`HostSyntax::value_for_record_field`] to every non-enum field
/// value and to `base` — matching Rust's existing structure, where the
/// clone-discipline hook only ever wraps the generic recursive path,
/// never the enum-literal special case.
fn lower_record_cons<H: HostSyntax>(
    host: &H,
    ir: &NormalizedIr,
    body: &HandlerBody,
    record: RecordId,
    base_value: Option<&HirExpr>,
    fields: &[(String, HirExpr)],
) -> String {
    let record_name = ir.record(record).name.clone();
    let rendered_fields: Vec<(String, String)> = fields
        .iter()
        .map(|(name, value)| {
            let rendered = if let HirExpr::EnumLit { variant, .. } = value {
                let enum_name = format!("{record_name}{}", name.to_pascal_case());
                host.enum_literal(Some(&enum_name), variant)
            } else {
                let scalar = lower_scalar(host, ir, body, value);
                host.value_for_record_field(scalar, value)
            };
            (name.clone(), rendered)
        })
        .collect();
    let rendered_base = base_value.map(|base| {
        let scalar = lower_scalar(host, ir, body, base);
        host.value_for_record_field(scalar, base)
    });
    host.record_cons(&record_name, &rendered_fields, rendered_base.as_deref())
}

/// `rust_binary`'s enum-comparison special case, generalized: an
/// `Eq`/`NotEq` comparison whose RHS is a bare `EnumLit` resolves the
/// LHS's named enum type *before* recursing (Rust needs it; Python
/// ignores it, and — since `enum_literal` renders it the exact same
/// way `py_expr`'s own generic `EnumLit` arm always did — this stays
/// byte-identical for Python whether or not the special case fires).
fn lower_binary<H: HostSyntax>(
    host: &H,
    ir: &NormalizedIr,
    body: &HandlerBody,
    op: BinOp,
    lhs: &HirExpr,
    rhs: &HirExpr,
) -> String {
    if matches!(op, BinOp::Eq | BinOp::NotEq) {
        if let HirExpr::EnumLit { variant, .. } = rhs {
            let enum_name = field_access_enum_name(ir, lhs);
            let lhs_s = lower_scalar(host, ir, body, lhs);
            let rhs_s = host.enum_literal(enum_name.as_deref(), variant);
            return host.binary(op, &lhs_s, &rhs_s, &lhs.ty(), &rhs.ty());
        }
    }
    let lhs_s = lower_scalar(host, ir, body, lhs);
    let rhs_s = lower_scalar(host, ir, body, rhs);
    host.binary(op, &lhs_s, &rhs_s, &lhs.ty(), &rhs.ty())
}

/// Every verb call that fits a single value in both orientations —
/// everything except `db.insert`/`update`/`delete`/`query`/`count`/
/// `delete_where` (see [`lower_expr_any`]/[`lower_tail`]).
fn lower_simple_verb<H: HostSyntax>(
    host: &H,
    ir: &NormalizedIr,
    body: &HandlerBody,
    verb: Verb,
    args: &[HirExpr],
) -> String {
    match verb {
        Verb::DbInsert(_) | Verb::DbUpdate(_) | Verb::DbDelete(_) => {
            unreachable!("db.insert/update/delete must be lowered via lower_expr_any/lower_tail")
        }
        Verb::DbGet(table) => {
            let key = lower_scalar(host, ir, body, &args[0]);
            host.db_get(table, &key)
        }
        Verb::CacheGet => {
            let key = lower_scalar(host, ir, body, &args[0]);
            host.cache_get(&key)
        }
        Verb::CacheSet => {
            let key = lower_scalar(host, ir, body, &args[0]);
            let value = lower_scalar(host, ir, body, &args[1]);
            host.cache_set(&key, &value, &args[1].ty())
        }
        Verb::CacheDelete => {
            let key = lower_scalar(host, ir, body, &args[0]);
            host.cache_delete(&key)
        }
        Verb::ObjectStorePut => {
            let key = lower_scalar(host, ir, body, &args[0]);
            let value = lower_scalar(host, ir, body, &args[1]);
            host.object_store_put(&key, &value, &args[1].ty())
        }
        Verb::ObjectStoreGet => {
            let key = lower_scalar(host, ir, body, &args[0]);
            host.object_store_get(&key)
        }
        Verb::ObjectStoreDelete => {
            let key = lower_scalar(host, ir, body, &args[0]);
            host.object_store_delete(&key)
        }
        Verb::ObjectStoreList => {
            let prefix = lower_scalar(host, ir, body, &args[0]);
            host.object_store_list(&prefix)
        }
        Verb::EmailSend => {
            let to = lower_scalar(host, ir, body, &args[0]);
            let subject = lower_scalar(host, ir, body, &args[1]);
            let body_arg = lower_scalar(host, ir, body, &args[2]);
            host.email_send(&to, &subject, &body_arg)
        }
        Verb::SearchIndex => {
            let doc_id = lower_scalar(host, ir, body, &args[0]);
            let document = lower_scalar(host, ir, body, &args[1]);
            host.search_index(&doc_id, &document, &args[1].ty())
        }
        Verb::SearchQuery => {
            let query = lower_scalar(host, ir, body, &args[0]);
            host.search_query(&query)
        }
        Verb::HttpCall => {
            let url = lower_scalar(host, ir, body, &args[0]);
            let json_val = lower_scalar(host, ir, body, &args[1]);
            host.http_call(&url, &json_val, &args[1].ty())
        }
        Verb::DbQuery(_) | Verb::DbCount(_) | Verb::DbDeleteWhere(_) => {
            unreachable!("typeck only ever constructs these via HirExpr::Query")
        }
    }
}

/// Lowers a `db.query`/`db.count`/`db.delete_where` predicate's terms,
/// pre-lowering each term's *value* only — see [`LoweredPredicate`]'s
/// own doc for why the WHERE-clause-vs-chain text itself stays a leaf
/// concern.
fn lower_predicate<H: HostSyntax>(
    host: &H,
    ir: &NormalizedIr,
    body: &HandlerBody,
    predicate: &Option<HirPredicate>,
) -> Option<LoweredPredicate> {
    let predicate = predicate.as_ref()?;
    let terms = predicate
        .terms
        .iter()
        .map(|term| {
            let value = match &term.value {
                HirExpr::EnumLit { variant, .. } => PredValue::EnumVariant(variant.clone()),
                HirExpr::BoolLit(b) => PredValue::BoolLit(*b),
                other => PredValue::Rendered(lower_scalar(host, ir, body, other)),
            };
            LoweredPredTerm {
                field: term.field.clone(),
                field_ty: term.field_ty.clone(),
                op: term.op,
                value,
            }
        })
        .collect();
    Some(LoweredPredicate { terms })
}

/// `Expression`-orientation only (Rust today): the four block-shaped
/// [`HirExpr`] forms lower directly as nested values here; everything
/// else delegates to [`lower_scalar`]. A generic-ized `rust_expr`.
pub fn lower_expr_any<H: HostSyntax>(
    host: &H,
    ir: &NormalizedIr,
    body: &HandlerBody,
    expr: &HirExpr,
) -> String {
    match expr {
        HirExpr::If {
            cond,
            then_branch,
            else_branch,
            ..
        } => {
            let cond_s = lower_scalar(host, ir, body, cond);
            let then_s = lower_block_expr(host, ir, body, then_branch, Wrap::Plain);
            let else_s = lower_block_expr(host, ir, body, else_branch, Wrap::Plain);
            host.if_expr(&cond_s, &then_s, &else_s)
        }
        HirExpr::Match {
            scrutinee, arms, ..
        } => {
            let enum_name = field_access_enum_name(ir, scrutinee)
                .expect("match scrutinee must be a record field access");
            let scrut_s = lower_scalar(host, ir, body, scrutinee);
            let rendered_arms: Vec<MatchArm> = arms
                .iter()
                .map(|arm| MatchArm {
                    variant: arm.variant.clone(),
                    body: lower_block_expr(host, ir, body, &arm.body, Wrap::Plain),
                })
                .collect();
            host.match_expr(&enum_name, &scrut_s, &rendered_arms)
        }
        HirExpr::VerbCall {
            verb: Verb::DbInsert(table),
            args,
            ..
        } => {
            let value_s = lower_scalar(host, ir, body, &args[0]);
            host.db_insert_expr(*table, &value_s)
        }
        HirExpr::VerbCall {
            verb: Verb::DbUpdate(table),
            args,
            ..
        } => {
            let key_s = lower_scalar(host, ir, body, &args[0]);
            let value_s = lower_scalar(host, ir, body, &args[1]);
            host.db_update_expr(*table, &key_s, &value_s)
        }
        HirExpr::VerbCall {
            verb: Verb::DbDelete(table),
            args,
            ..
        } => {
            let key_s = lower_scalar(host, ir, body, &args[0]);
            host.db_delete_expr(*table, &key_s)
        }
        HirExpr::Query {
            verb, predicate, ..
        } => {
            let lowered = lower_predicate(host, ir, body, predicate);
            host.query_expr(*verb, lowered.as_ref())
        }
        _ => lower_scalar(host, ir, body, expr),
    }
}

/// `Expression`-orientation block lowering (`rust_block`, generic-ized):
/// walks `stmts`, joins each lowered statement with `\n`, truncates
/// after a diverging statement (Rust's `-D warnings` promotes
/// `unreachable_code` to a hard failure), and shapes an empty block
/// via [`HostSyntax::wrap_tail`]/[`HostSyntax::unit_literal`].
pub fn lower_block_expr<H: HostSyntax>(
    host: &H,
    ir: &NormalizedIr,
    body: &HandlerBody,
    stmts: &[HirStmt],
    wrap: Wrap,
) -> String {
    if stmts.is_empty() {
        // Original `rust_block`'s empty-block short-circuit collapses
        // `None`/`Plain` to the same bare `"()"` (never `"();"` ,
        // even though a *non-empty* `None`-wrapped block's own last
        // statement *does* get `;`-terminated via `wrap_tail` below) —
        // preserved exactly, not "fixed" into false symmetry.
        let effective = if wrap == Wrap::Wrapped {
            Wrap::Wrapped
        } else {
            Wrap::Plain
        };
        let unit = host.unit_literal();
        return host.wrap_tail(&unit, effective);
    }
    let mut lines = Vec::new();
    for (i, stmt) in stmts.iter().enumerate() {
        let is_last = i + 1 == stmts.len();
        lines.push(lower_stmt_expr(
            host,
            ir,
            body,
            stmt,
            if is_last { wrap } else { Wrap::None },
        ));
        if stmt_diverges(stmt) {
            break;
        }
    }
    lines.join("\n")
}

/// `rust_stmt`, generic-ized.
fn lower_stmt_expr<H: HostSyntax>(
    host: &H,
    ir: &NormalizedIr,
    body: &HandlerBody,
    stmt: &HirStmt,
    wrap: Wrap,
) -> String {
    match stmt {
        HirStmt::Let { slot, value } => {
            if value.ty() == HirType::Never {
                // Every path through `value` returns/fails, so it
                // never actually produces anything to bind — a `let`
                // here would be an unused-variable warning, promoted
                // to a hard error by `-D warnings`. Just run it.
                format!("{};", lower_expr_any(host, ir, body, value))
            } else {
                host.let_binding(
                    &local_name(body, *slot),
                    &lower_expr_any(host, ir, body, value),
                )
            }
        }
        HirStmt::Expr(e) => {
            let e_s = lower_expr_any(host, ir, body, e);
            host.wrap_tail(&e_s, wrap)
        }
        HirStmt::Return(None) => host.return_stmt(None, ""),
        HirStmt::Return(Some(e)) => {
            let e_s = lower_expr_any(host, ir, body, e);
            host.return_stmt(Some(&e_s), "")
        }
        HirStmt::Fail { error, args } => {
            let arg_strs: Vec<String> = args
                .iter()
                .map(|a| lower_expr_any(host, ir, body, a))
                .collect();
            host.fail(*error, &arg_strs, "")
        }
        HirStmt::Publish { stream, value } => {
            let subject = stream_subject(ir, *stream);
            let value_s = lower_expr_any(host, ir, body, value);
            host.publish(&subject, &value_s, &value.ty(), "")
        }
        HirStmt::Transaction { body: inner } => {
            let inner_s = lower_block_expr(host, ir, body, inner, Wrap::None);
            host.transaction_expr(&inner_s)
        }
    }
}

/// `Statement`-orientation only (Python today): the four block-shaped
/// [`HirExpr`] forms decompose into a statement sequence applied to
/// `dest` here; everything else lowers via [`lower_scalar`] and the
/// result is applied to `dest` directly. A generic-ized Python
/// `lower_tail`.
pub fn lower_tail<H: HostSyntax>(
    host: &H,
    ir: &NormalizedIr,
    body: &HandlerBody,
    expr: &HirExpr,
    indent: &str,
    dest: &Dest,
    in_tx: bool,
) -> Vec<String> {
    match expr {
        HirExpr::If {
            cond,
            then_branch,
            else_branch,
            ..
        } => {
            let cond_s = lower_scalar(host, ir, body, cond);
            let inner_indent = format!("{indent}    ");
            let then_lines =
                lower_block_stmt(host, ir, body, then_branch, &inner_indent, dest, in_tx);
            let else_lines =
                lower_block_stmt(host, ir, body, else_branch, &inner_indent, dest, in_tx);
            host.if_tail(&cond_s, then_lines, else_lines, indent)
        }
        HirExpr::Match {
            scrutinee, arms, ..
        } => {
            let scrut_s = lower_scalar(host, ir, body, scrutinee);
            let inner_indent = format!("{indent}    ");
            let rendered_arms: Vec<(Option<String>, Vec<String>)> = arms
                .iter()
                .map(|arm| {
                    let lines =
                        lower_block_stmt(host, ir, body, &arm.body, &inner_indent, dest, in_tx);
                    (arm.variant.clone(), lines)
                })
                .collect();
            host.match_tail(&scrut_s, &rendered_arms, indent)
        }
        HirExpr::VerbCall {
            verb: Verb::DbInsert(table),
            args,
            ..
        } => {
            let value_s = lower_scalar(host, ir, body, &args[0]);
            host.db_insert_tail(*table, &value_s, dest, indent, in_tx)
        }
        HirExpr::VerbCall {
            verb: Verb::DbUpdate(table),
            args,
            ..
        } => {
            let key_s = lower_scalar(host, ir, body, &args[0]);
            let value_s = lower_scalar(host, ir, body, &args[1]);
            host.db_update_tail(*table, &key_s, &value_s, dest, indent, in_tx)
        }
        HirExpr::VerbCall {
            verb: Verb::DbDelete(table),
            args,
            ..
        } => {
            let key_s = lower_scalar(host, ir, body, &args[0]);
            host.db_delete_tail(*table, &key_s, dest, indent, in_tx)
        }
        HirExpr::Query {
            verb, predicate, ..
        } => {
            let lowered = lower_predicate(host, ir, body, predicate);
            host.query_tail(*verb, lowered.as_ref(), dest, indent, in_tx)
        }
        // The error-idiom amendment (`24UpdatePlan.md` M4): every
        // "simple verb" gets its own tail-dispatch arm here instead of
        // falling to the generic `_ =>` scalar-then-apply_dest path
        // below, mirroring how `DbInsert`/`DbUpdate`/`DbDelete`/`Query`
        // already do. `HostSyntax`'s own default for each new
        // `..._tail` method reproduces the old fallback computation
        // exactly (see that trait's doc comment), so this is a pure
        // routing change for every host that doesn't override one —
        // in_tx is not threaded here: none of these are database-
        // transactional operations, matching `db_get`'s own existing
        // no-`in_tx` signature above.
        HirExpr::VerbCall {
            verb: Verb::DbGet(table),
            args,
            ..
        } => {
            let key_s = lower_scalar(host, ir, body, &args[0]);
            host.db_get_tail(*table, &key_s, dest, indent)
        }
        HirExpr::VerbCall {
            verb: Verb::CacheGet,
            args,
            ..
        } => {
            let key_s = lower_scalar(host, ir, body, &args[0]);
            host.cache_get_tail(&key_s, dest, indent)
        }
        HirExpr::VerbCall {
            verb: Verb::CacheSet,
            args,
            ..
        } => {
            let key_s = lower_scalar(host, ir, body, &args[0]);
            let value_s = lower_scalar(host, ir, body, &args[1]);
            host.cache_set_tail(&key_s, &value_s, &args[1].ty(), dest, indent)
        }
        HirExpr::VerbCall {
            verb: Verb::CacheDelete,
            args,
            ..
        } => {
            let key_s = lower_scalar(host, ir, body, &args[0]);
            host.cache_delete_tail(&key_s, dest, indent)
        }
        HirExpr::VerbCall {
            verb: Verb::ObjectStorePut,
            args,
            ..
        } => {
            let key_s = lower_scalar(host, ir, body, &args[0]);
            let value_s = lower_scalar(host, ir, body, &args[1]);
            host.object_store_put_tail(&key_s, &value_s, &args[1].ty(), dest, indent)
        }
        HirExpr::VerbCall {
            verb: Verb::ObjectStoreGet,
            args,
            ..
        } => {
            let key_s = lower_scalar(host, ir, body, &args[0]);
            host.object_store_get_tail(&key_s, dest, indent)
        }
        HirExpr::VerbCall {
            verb: Verb::ObjectStoreDelete,
            args,
            ..
        } => {
            let key_s = lower_scalar(host, ir, body, &args[0]);
            host.object_store_delete_tail(&key_s, dest, indent)
        }
        HirExpr::VerbCall {
            verb: Verb::ObjectStoreList,
            args,
            ..
        } => {
            let prefix_s = lower_scalar(host, ir, body, &args[0]);
            host.object_store_list_tail(&prefix_s, dest, indent)
        }
        HirExpr::VerbCall {
            verb: Verb::EmailSend,
            args,
            ..
        } => {
            let to_s = lower_scalar(host, ir, body, &args[0]);
            let subject_s = lower_scalar(host, ir, body, &args[1]);
            let body_s = lower_scalar(host, ir, body, &args[2]);
            host.email_send_tail(&to_s, &subject_s, &body_s, dest, indent)
        }
        HirExpr::VerbCall {
            verb: Verb::SearchIndex,
            args,
            ..
        } => {
            let doc_id_s = lower_scalar(host, ir, body, &args[0]);
            let document_s = lower_scalar(host, ir, body, &args[1]);
            host.search_index_tail(&doc_id_s, &document_s, &args[1].ty(), dest, indent)
        }
        HirExpr::VerbCall {
            verb: Verb::SearchQuery,
            args,
            ..
        } => {
            let query_s = lower_scalar(host, ir, body, &args[0]);
            host.search_query_tail(&query_s, dest, indent)
        }
        HirExpr::VerbCall {
            verb: Verb::HttpCall,
            args,
            ..
        } => {
            let url_s = lower_scalar(host, ir, body, &args[0]);
            let json_s = lower_scalar(host, ir, body, &args[1]);
            host.http_call_tail(&url_s, &json_s, &args[1].ty(), dest, indent)
        }
        _ => {
            let e = lower_scalar(host, ir, body, expr);
            let mut out = Vec::new();
            apply_dest(host, dest, &e, indent, &mut out);
            out
        }
    }
}

/// `Statement`-orientation block lowering (Python's `lower_block`,
/// generic-ized): walks `stmts`, routes the last non-`Never` bare
/// `Expr` through [`lower_tail`] (so a trailing `if`/`match`/db-write
/// value lands in `dest`), truncates after a diverging statement, and
/// shapes an empty block via [`HostSyntax::empty_block_stmt`].
pub fn lower_block_stmt<H: HostSyntax>(
    host: &H,
    ir: &NormalizedIr,
    body: &HandlerBody,
    stmts: &[HirStmt],
    indent: &str,
    dest: &Dest,
    in_tx: bool,
) -> Vec<String> {
    if stmts.is_empty() {
        return host.empty_block_stmt(indent);
    }
    let mut out = Vec::new();
    for (i, stmt) in stmts.iter().enumerate() {
        let is_last = i + 1 == stmts.len();
        if is_last {
            if let HirStmt::Expr(e) = stmt {
                if e.ty() != HirType::Never {
                    out.extend(lower_tail(host, ir, body, e, indent, dest, in_tx));
                    continue;
                }
            }
        }
        out.extend(lower_stmt(host, ir, body, stmt, indent, in_tx));
        // The type checker still type-checks (and the HIR still
        // contains) statements after one that diverges — lowering one
        // would reference a name that was never bound. Those
        // statements are unreachable at runtime either way, so stop
        // emitting once a statement diverges.
        if stmt_diverges(stmt) {
            return out;
        }
    }
    out
}

/// Python's `lower_stmt`, generic-ized: `Let`/`Expr`/`Return` route
/// through [`lower_tail`] (their value may be one of the four
/// block-shaped forms); `Fail`/`Publish` lower their operands via
/// [`lower_scalar`] only, matching Python's own `py_expr`-only calls
/// there today; `Transaction` recurses into its own inner block.
fn lower_stmt<H: HostSyntax>(
    host: &H,
    ir: &NormalizedIr,
    body: &HandlerBody,
    stmt: &HirStmt,
    indent: &str,
    in_tx: bool,
) -> Vec<String> {
    match stmt {
        HirStmt::Let { slot, value } => {
            let name = local_name(body, *slot);
            lower_tail(host, ir, body, value, indent, &Dest::Assign(name), in_tx)
        }
        HirStmt::Expr(e) => lower_tail(host, ir, body, e, indent, &Dest::Discard, in_tx),
        HirStmt::Return(None) => vec![host.return_stmt(None, indent)],
        HirStmt::Return(Some(e)) => lower_tail(host, ir, body, e, indent, &Dest::Return, in_tx),
        HirStmt::Fail { error, args } => {
            let arg_strs: Vec<String> = args
                .iter()
                .map(|a| lower_scalar(host, ir, body, a))
                .collect();
            vec![host.fail(*error, &arg_strs, indent)]
        }
        HirStmt::Publish { stream, value } => {
            let subject = stream_subject(ir, *stream);
            let value_s = lower_scalar(host, ir, body, value);
            vec![host.publish(&subject, &value_s, &value.ty(), indent)]
        }
        HirStmt::Transaction { body: inner } => {
            let block_indent = format!("{indent}    ");
            let inner_lines =
                lower_block_stmt(host, ir, body, inner, &block_indent, &Dest::Discard, true);
            host.transaction_stmt(inner_lines, indent)
        }
    }
}

/// Entry point for `Expression`-oriented backends (replaces a private
/// `lower_body`): the function body's own tail gets `Ok(..)`-wrapped.
pub fn lower_body_expr<H: HostSyntax>(host: &H, ir: &NormalizedIr, body: &HandlerBody) -> String {
    let stmts = body.body.as_deref().unwrap_or(&[]);
    lower_block_expr(host, ir, body, stmts, Wrap::Wrapped)
}

/// Entry point for `Statement`-oriented backends (replaces a private
/// `lower_body`): a final defensive check mirrors Python's own
/// pre-factory `lower_body`, which pushed a synthesized empty-block
/// line if the whole walk somehow produced nothing despite non-empty
/// `stmts` — never observed to fire, kept for exact parity.
pub fn lower_body_stmt<H: HostSyntax>(
    host: &H,
    ir: &NormalizedIr,
    body: &HandlerBody,
    indent: &str,
) -> Vec<String> {
    let stmts = body.body.as_deref().unwrap_or(&[]);
    let mut out = lower_block_stmt(host, ir, body, stmts, indent, &Dest::Discard, false);
    if out.is_empty() {
        out = host.empty_block_stmt(indent);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_outer_parens_unwraps_a_single_wrapping_pair() {
        assert_eq!(strip_outer_parens("(a && b)".to_owned()), "a && b");
    }

    #[test]
    fn strip_outer_parens_leaves_non_wrapping_parens_alone() {
        assert_eq!(strip_outer_parens("(a) + (b)".to_owned()), "(a) + (b)");
    }

    #[test]
    fn strip_outer_parens_leaves_unparenthesized_text_alone() {
        assert_eq!(strip_outer_parens("a && b".to_owned()), "a && b");
    }

    #[test]
    fn fidelity_checked_float_always_contains_a_dot() {
        assert_eq!(fidelity_checked_float(1.0), "1.0");
        assert_eq!(fidelity_checked_float(0.0), "0.0");
    }

    #[test]
    fn fidelity_checked_float_leaves_already_fractional_values_alone() {
        assert_eq!(fidelity_checked_float(1.5), "1.5");
    }

    #[test]
    fn fidelity_checked_float_leaves_named_special_values_alone() {
        assert_eq!(fidelity_checked_float(f64::INFINITY), "inf");
    }

    #[test]
    fn indent_lines_prefixes_every_line() {
        assert_eq!(indent_lines("a\nb", "  "), "  a\n  b");
    }
}
