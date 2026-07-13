//! v0.7 M2: the handler-body expression type checker.
//!
//! Walks `ciac_syntax::ast::{Expr, Stmt}` (pure syntax, per that module's
//! own contract: no name resolution, no types) and lowers it into
//! `ciac_ir::hir` — every name resolved to a local slot, every verb
//! resolved to `(capability instance, operation, table)`, every
//! expression annotated with its type.
//!
//! Implemented as a second `impl Builder` block (see `build.rs`) so it
//! reuses `resolve_record`/`resolve_stream`/`default_capability`/
//! `check_match_labels`/etc. exactly as pipeline-step resolution does,
//! instead of re-deriving capability and type resolution behind a
//! narrower free-function API.
//!
//! Closed verb set for this milestone (07UpdatePlan.md's own stated
//! risk: "scope creep is the failure mode"): `db.insert`/`db.get`,
//! `cache.get`/`cache.set`, `object_store.put`/`object_store.get`, plus
//! the niladic builtins `Uuid.new()`/`Timestamp.now()`. `email`/`search`/
//! `external_http`/`query` are not implemented; adding one is a new arm
//! in `check_verb_call`, not a redesign.
//!
//! Error recovery is deliberately simple: the first error inside a
//! handler body aborts type checking for that handler (via `?`
//! propagation) rather than attempting statement-level resynchronization
//! like the parser does. Independent handlers/declarations are unaffected
//! by one broken handler.

use crate::build::levenshtein;
use crate::build::Builder;
use ciac_diagnostics::{Diagnostic, Edit, ErrorCode, Fix, Span};
use ciac_ir::{
    Builtin, EdgeKind, FieldType, HandlerBody, HirArm, HirExpr, HirPredTerm, HirPredicate, HirStmt,
    HirType, NodeKind, RecordField, RecordId, RecordKind, Verb,
};
use ciac_syntax::ast::{self, ArmLabel, Expr, FieldInit, HandlerDecl, Ident, Stmt, TypeExpr};
use std::collections::HashMap;

/// Per-handler-body name resolution: a flat, monotonically-growing local
/// table (indexed by slot, matching `HandlerBody::locals`) with a stack
/// of block scopes for `let`'s block-scoping.
struct Scope {
    return_ty: HirType,
    locals: Vec<HirType>,
    names: Vec<String>,
    spans: Vec<Span>,
    is_let: Vec<bool>,
    used: Vec<bool>,
    frames: Vec<HashMap<String, u32>>,
}

impl Scope {
    fn new(return_ty: HirType) -> Self {
        Self {
            return_ty,
            locals: Vec::new(),
            names: Vec::new(),
            spans: Vec::new(),
            is_let: Vec::new(),
            used: Vec::new(),
            frames: vec![HashMap::new()],
        }
    }

    fn declare(&mut self, name: &str, ty: HirType, is_let: bool, span: Span) -> u32 {
        let slot = self.locals.len() as u32;
        self.locals.push(ty);
        self.names.push(name.to_owned());
        self.spans.push(span);
        self.is_let.push(is_let);
        self.used.push(false);
        self.frames
            .last_mut()
            .expect("at least one frame")
            .insert(name.to_owned(), slot);
        slot
    }

    fn lookup(&mut self, name: &str) -> Option<(u32, HirType)> {
        for frame in self.frames.iter().rev() {
            if let Some(&slot) = frame.get(name) {
                self.used[slot as usize] = true;
                return Some((slot, self.locals[slot as usize].clone()));
            }
        }
        None
    }

    fn push_frame(&mut self) {
        self.frames.push(HashMap::new());
    }

    fn pop_frame(&mut self) {
        self.frames.pop();
    }
}

/// A value's type, as `FieldType` (the surface/record-field type set)
/// mapped onto the HIR's superset.
/// The closest field name to `field` among `fields` by Levenshtein
/// distance (v0.15 M7 "did you mean" fix) -- `None` when nothing is
/// close enough to be a plausible typo rather than a coincidence.
fn nearest_field<'a>(fields: &'a [RecordField], field: &str) -> Option<&'a str> {
    fields
        .iter()
        .map(|f| {
            (
                f.name.as_str(),
                levenshtein(&f.name.to_lowercase(), &field.to_lowercase()),
            )
        })
        .min_by_key(|&(_, dist)| dist)
        .and_then(|(name, dist)| (dist <= 3).then_some(name))
}

fn field_type_to_hir(ty: &FieldType) -> HirType {
    match ty {
        FieldType::Str => HirType::Str,
        FieldType::Int => HirType::Int,
        FieldType::Float => HirType::Float,
        FieldType::Bool => HirType::Bool,
        FieldType::Uuid => HirType::Uuid,
        FieldType::Timestamp => HirType::Timestamp,
        FieldType::Json => HirType::Json,
        FieldType::Enum { variants } => HirType::Enum {
            variants: variants.clone(),
        },
        // v0.16 M2: a resolved `Reference<T>` field type-checks as a
        // plain value of the target record's type inside handler-body
        // expressions (`RecordCons`, `FieldAccess`, ...) for now — the
        // eventual wire contract (a field carries the target's `id`,
        // never an embedded object) is v0.16 M5/M6 codegen, not a
        // typeck-level distinction this milestone needs to make.
        FieldType::Reference { target, .. } => HirType::Record(*target),
    }
}

/// Whether a type can appear on either side of `+` when the other side is
/// `Str` (implicit stringification, e.g. `"videos/" + v.id`).
fn is_stringifiable(ty: &HirType) -> bool {
    matches!(
        ty,
        HirType::Str
            | HirType::Int
            | HirType::Float
            | HirType::Bool
            | HirType::Uuid
            | HirType::Timestamp
            | HirType::Enum { .. }
    )
}

/// Whether executing `stmt` never falls through to whatever follows it —
/// directly (`return`/`fail`) or because its own value already diverges
/// (e.g. `let x = <match where every arm returns>;`).
fn stmt_diverges(stmt: &HirStmt) -> bool {
    match stmt {
        HirStmt::Return(_) | HirStmt::Fail { .. } => true,
        HirStmt::Let { value, .. } | HirStmt::Expr(value) => value.ty() == HirType::Never,
        HirStmt::Publish { .. } | HirStmt::Transaction { .. } => false,
    }
}

/// Recursively finds `return`, nested `transaction`, and `publish`
/// statements anywhere inside a `transaction` body — including inside
/// `if`/`match` branches — over the surface AST, before any type
/// information exists (v0.16 M1/M2). Each would be legal syntax outside
/// a transaction but breaks its atomicity guarantee: `return` could
/// bypass the generated commit/rollback epilogue, a nested `transaction`
/// has no meaning, and `publish` doesn't roll back with the database.
fn scan_transaction_body(stmts: &[Stmt], violations: &mut Vec<(ErrorCode, String, Span)>) {
    for stmt in stmts {
        match stmt {
            Stmt::Return { span, .. } => violations.push((
                ErrorCode::InvalidTransactionBlock,
                "`return` is not allowed inside `transaction`".to_owned(),
                *span,
            )),
            Stmt::Transaction { span, .. } => violations.push((
                ErrorCode::InvalidTransactionBlock,
                "`transaction` blocks cannot nest".to_owned(),
                *span,
            )),
            Stmt::Publish { span, .. } => violations.push((
                ErrorCode::NonTransactionalEffect,
                "`publish` does not roll back with a database transaction".to_owned(),
                *span,
            )),
            Stmt::Let { value, .. } | Stmt::Expr(value) => scan_transaction_expr(value, violations),
            Stmt::Fail { .. } => {}
        }
    }
}

fn scan_transaction_expr(expr: &Expr, violations: &mut Vec<(ErrorCode, String, Span)>) {
    match expr {
        Expr::If {
            then_branch,
            else_branch,
            ..
        } => {
            scan_transaction_body(then_branch, violations);
            if let Some(else_branch) = else_branch {
                scan_transaction_body(else_branch, violations);
            }
        }
        Expr::Match { arms, .. } => {
            for arm in arms {
                scan_transaction_body(&arm.body, violations);
            }
        }
        _ => {}
    }
}

/// Walks an already-lowered transaction body's HIR looking for verb
/// calls, classifying each as the single database capability (allowed)
/// or one of the other capabilities (rejected — see
/// `ErrorCode::NonTransactionalEffect`). `Return`/`Publish`/`Transaction`
/// never appear here: `scan_transaction_body` already rejected them
/// before this body was lowered.
fn collect_transaction_verbs(
    stmts: &[HirStmt],
    has_db_verb: &mut bool,
    non_db_verb: &mut Option<&'static str>,
) {
    for stmt in stmts {
        match stmt {
            HirStmt::Let { value, .. } | HirStmt::Expr(value) => {
                collect_transaction_verbs_expr(value, has_db_verb, non_db_verb)
            }
            HirStmt::Fail { args, .. } => {
                for arg in args {
                    collect_transaction_verbs_expr(arg, has_db_verb, non_db_verb);
                }
            }
            HirStmt::Return(_) | HirStmt::Publish { .. } | HirStmt::Transaction { .. } => {}
        }
    }
}

fn collect_transaction_verbs_expr(
    expr: &HirExpr,
    has_db_verb: &mut bool,
    non_db_verb: &mut Option<&'static str>,
) {
    match expr {
        HirExpr::VerbCall { verb, args, .. } => {
            classify_transaction_verb(*verb, has_db_verb, non_db_verb);
            for arg in args {
                collect_transaction_verbs_expr(arg, has_db_verb, non_db_verb);
            }
        }
        HirExpr::Query { verb, .. } => {
            classify_transaction_verb(*verb, has_db_verb, non_db_verb);
        }
        HirExpr::FieldAccess { base, .. } => {
            collect_transaction_verbs_expr(base, has_db_verb, non_db_verb)
        }
        HirExpr::Unary { expr, .. } => {
            collect_transaction_verbs_expr(expr, has_db_verb, non_db_verb)
        }
        HirExpr::Index { base, index } => {
            collect_transaction_verbs_expr(base, has_db_verb, non_db_verb);
            collect_transaction_verbs_expr(index, has_db_verb, non_db_verb);
        }
        HirExpr::RecordCons {
            base_value, fields, ..
        } => {
            if let Some(base) = base_value {
                collect_transaction_verbs_expr(base, has_db_verb, non_db_verb);
            }
            for (_, value) in fields {
                collect_transaction_verbs_expr(value, has_db_verb, non_db_verb);
            }
        }
        HirExpr::Binary { lhs, rhs, .. } => {
            collect_transaction_verbs_expr(lhs, has_db_verb, non_db_verb);
            collect_transaction_verbs_expr(rhs, has_db_verb, non_db_verb);
        }
        HirExpr::If {
            cond,
            then_branch,
            else_branch,
            ..
        } => {
            collect_transaction_verbs_expr(cond, has_db_verb, non_db_verb);
            collect_transaction_verbs(then_branch, has_db_verb, non_db_verb);
            collect_transaction_verbs(else_branch, has_db_verb, non_db_verb);
        }
        HirExpr::Match {
            scrutinee, arms, ..
        } => {
            collect_transaction_verbs_expr(scrutinee, has_db_verb, non_db_verb);
            for arm in arms {
                collect_transaction_verbs(&arm.body, has_db_verb, non_db_verb);
            }
        }
        HirExpr::Local { .. }
        | HirExpr::IntLit(_)
        | HirExpr::FloatLit(_)
        | HirExpr::StrLit(_)
        | HirExpr::BoolLit(_)
        | HirExpr::BuiltinCall(_)
        | HirExpr::EnumLit { .. } => {}
    }
}

fn classify_transaction_verb(
    verb: Verb,
    has_db_verb: &mut bool,
    non_db_verb: &mut Option<&'static str>,
) {
    match verb {
        Verb::DbInsert(_)
        | Verb::DbGet(_)
        | Verb::DbUpdate(_)
        | Verb::DbDelete(_)
        | Verb::DbQuery(_)
        | Verb::DbCount(_)
        | Verb::DbDeleteWhere(_) => *has_db_verb = true,
        Verb::CacheGet | Verb::CacheSet | Verb::CacheDelete => {
            non_db_verb.get_or_insert("cache");
        }
        Verb::ObjectStorePut
        | Verb::ObjectStoreGet
        | Verb::ObjectStoreDelete
        | Verb::ObjectStoreList => {
            non_db_verb.get_or_insert("object_store");
        }
        Verb::EmailSend => {
            non_db_verb.get_or_insert("email");
        }
        Verb::SearchIndex | Verb::SearchQuery => {
            non_db_verb.get_or_insert("search");
        }
        Verb::HttpCall => {
            non_db_verb.get_or_insert("external_http");
        }
    }
}

impl Builder<'_> {
    /// Type-checks a v0.7 handler declaration (inline body or `extern`)
    /// into a [`HandlerBody`]. `decl.body.is_none() && !decl.is_extern`
    /// (the classic binding-only form) must not reach this function.
    pub(crate) fn check_handler_body(&mut self, decl: &HandlerDecl) -> Option<HandlerBody> {
        let return_ty = match &decl.return_ty {
            Some(ty) => self.resolve_hir_type(ty)?,
            None => HirType::Unit,
        };
        let mut scope = Scope::new(return_ty.clone());
        let mut params = Vec::with_capacity(decl.params.len());
        for param in &decl.params {
            let ty = self.resolve_hir_type(&param.ty)?;
            params.push((param.name.text.clone(), ty.clone()));
            scope.declare(&param.name.text, ty, false, param.span);
        }
        let body = match &decl.body {
            None => None,
            Some(stmts) => {
                let (hir_stmts, _tail_ty) = self.check_block(stmts, &mut scope)?;
                Some(hir_stmts)
            }
        };
        for i in 0..scope.locals.len() {
            if scope.is_let[i] && !scope.used[i] {
                self.diags.push(
                    Diagnostic::new(
                        ErrorCode::UnusedLet,
                        format!("`{}` is never read", scope.names[i]),
                    )
                    .with_label(scope.spans[i], "unused binding"),
                );
            }
        }
        Some(HandlerBody {
            params,
            return_ty,
            locals: scope.locals,
            body,
        })
    }

    /// Resolves a surface `TypeExpr` (a param/return type) to a
    /// [`HirType`]: a primitive, an inline enum, or a declared record.
    fn resolve_hir_type(&mut self, ty: &TypeExpr) -> Option<HirType> {
        match ty {
            TypeExpr::Named(ident) => {
                if let Some(ft) = FieldType::parse(&ident.text) {
                    return Some(field_type_to_hir(&ft));
                }
                match self.graph.find_record(&ident.text) {
                    Some(rid) => Some(HirType::Record(rid)),
                    None => {
                        self.diags.push(
                            Diagnostic::new(
                                ErrorCode::UnknownType,
                                format!("unknown type `{}`", ident.text),
                            )
                            .with_label(ident.span, "not a known type or record"),
                        );
                        None
                    }
                }
            }
            TypeExpr::Enum { variants, .. } => Some(HirType::Enum {
                variants: variants.iter().map(|v| v.text.clone()).collect(),
            }),
            TypeExpr::List { inner, .. } => {
                Some(HirType::List(Box::new(self.resolve_hir_type(inner)?)))
            }
            TypeExpr::Reference { target, span } => {
                self.diags.push(
                    Diagnostic::new(
                        ErrorCode::UnknownType,
                        format!(
                            "`Reference<{}>` is only valid as a `record` field type",
                            target.text
                        ),
                    )
                    .with_label(*span, "not valid as a handler parameter/return type"),
                );
                None
            }
        }
    }

    /// Type-checks a `{ <stmts> }` block. Returns the block's own value
    /// type: `HirType::Never` means every path through it diverges
    /// (`return`/`fail`, or a `let`/expression statement whose value
    /// itself already diverges) — once any statement diverges, the rest
    /// of the block is unreachable and doesn't affect the block's type.
    fn check_block(
        &mut self,
        stmts: &[Stmt],
        scope: &mut Scope,
    ) -> Option<(Vec<HirStmt>, HirType)> {
        let mut hir_stmts = Vec::with_capacity(stmts.len());
        let mut diverges = false;
        for stmt in stmts {
            let hir_stmt = self.check_stmt(stmt, scope)?;
            if stmt_diverges(&hir_stmt) {
                diverges = true;
            }
            hir_stmts.push(hir_stmt);
        }
        let tail = if diverges {
            HirType::Never
        } else {
            match hir_stmts.last() {
                Some(HirStmt::Expr(e)) => e.ty(),
                _ => HirType::Unit,
            }
        };
        Some((hir_stmts, tail))
    }

    fn check_stmt(&mut self, stmt: &Stmt, scope: &mut Scope) -> Option<HirStmt> {
        match stmt {
            Stmt::Let { name, value, span } => {
                let value = self.check_expr(value, scope)?;
                let ty = value.ty();
                let slot = scope.declare(&name.text, ty, true, *span);
                Some(HirStmt::Let { slot, value })
            }
            Stmt::Expr(expr) => Some(HirStmt::Expr(self.check_expr(expr, scope)?)),
            Stmt::Return { value: None, span } => {
                if HirType::unify(scope.return_ty.clone(), HirType::Unit).is_err() {
                    self.diags.push(
                        Diagnostic::new(
                            ErrorCode::HandlerExprTypeMismatch,
                            format!(
                                "handler returns `{:?}` but this `return;` yields nothing",
                                scope.return_ty
                            ),
                        )
                        .with_label(*span, "bare return here"),
                    );
                    return None;
                }
                Some(HirStmt::Return(None))
            }
            Stmt::Return {
                value: Some(expr),
                span,
            } => {
                let hir = self.check_expr(expr, scope)?;
                if HirType::unify(hir.ty(), scope.return_ty.clone()).is_err() {
                    self.diags.push(
                        Diagnostic::new(
                            ErrorCode::HandlerExprTypeMismatch,
                            format!(
                                "`return` yields `{:?}` but the handler declares `{:?}`",
                                hir.ty(),
                                scope.return_ty
                            ),
                        )
                        .with_label(*span, "mismatched return type"),
                    );
                    return None;
                }
                Some(HirStmt::Return(Some(hir)))
            }
            Stmt::Fail { error, args, span } => {
                let record_id = self.resolve_record(error)?;
                let record = self.graph.record(record_id);
                if record.kind != RecordKind::Error {
                    self.diags.push(
                        Diagnostic::new(
                            ErrorCode::HandlerExprTypeMismatch,
                            format!(
                                "`{}` is a `record`, not an `error` — `fail` requires an \
                                 `error {{ .. }}` declaration",
                                error.text
                            ),
                        )
                        .with_label(error.span, "not an error record"),
                    );
                    return None;
                }
                let fields = record.fields.clone();
                if args.len() != fields.len() {
                    self.diags.push(
                        Diagnostic::new(
                            ErrorCode::HandlerExprTypeMismatch,
                            format!(
                                "`fail {}` takes {} argument(s), found {}",
                                error.text,
                                fields.len(),
                                args.len()
                            ),
                        )
                        .with_label(*span, "argument count mismatch"),
                    );
                    return None;
                }
                let mut hir_args = Vec::with_capacity(args.len());
                for (arg, field) in args.iter().zip(&fields) {
                    let hir_arg = self.check_expr(arg, scope)?;
                    let expected = field_type_to_hir(&field.ty);
                    if hir_arg.ty() != expected {
                        self.diags.push(
                            Diagnostic::new(
                                ErrorCode::HandlerExprTypeMismatch,
                                format!(
                                    "`fail {}` field `{}` expects `{expected:?}`, found `{:?}`",
                                    error.text,
                                    field.name,
                                    hir_arg.ty()
                                ),
                            )
                            .with_label(arg.span(), "mismatched argument type"),
                        );
                        return None;
                    }
                    hir_args.push(hir_arg);
                }
                Some(HirStmt::Fail {
                    error: record_id,
                    args: hir_args,
                })
            }
            Stmt::Publish {
                stream,
                value,
                span,
            } => {
                let stream_node = self.resolve_stream(stream)?;
                if let Some(queue) =
                    self.require_queue(&format!("publish `{}`", stream.text), *span)
                {
                    self.graph.add_edge(stream_node, queue, EdgeKind::DependsOn);
                }
                let hir_value = self.check_expr(value, scope)?;
                let payload = match hir_value.ty() {
                    HirType::Record(rid) => Some(rid),
                    _ => None,
                };
                self.check_publish_type(stream_node, payload, *span);
                Some(HirStmt::Publish {
                    stream: stream_node,
                    value: hir_value,
                })
            }
            Stmt::Transaction { body, span } => self.check_transaction(body, *span, scope),
        }
    }

    /// Type-checks a `transaction { .. }` block (v0.16 M1/M2). Structural
    /// rules (`return`/nested `transaction`/`publish` forbidden, empty
    /// block forbidden) are checked over the surface AST first — they
    /// need no type information — before the block is even lowered;
    /// finding one aborts immediately, matching this module's "first
    /// error aborts the handler" convention. Only after that does the
    /// block get type-checked and its verb calls classified: at least
    /// one `db.*` verb is required, and any `cache`/`object_store`/
    /// `email`/`search`/`external_http` verb is rejected, since none of
    /// those roll back with the database transaction.
    fn check_transaction(
        &mut self,
        body: &[Stmt],
        span: Span,
        scope: &mut Scope,
    ) -> Option<HirStmt> {
        if body.is_empty() {
            self.diags.push(
                Diagnostic::new(
                    ErrorCode::InvalidTransactionBlock,
                    "`transaction` block is empty",
                )
                .with_label(span, "add at least one `db.*` verb"),
            );
            return None;
        }
        let mut violations = Vec::new();
        scan_transaction_body(body, &mut violations);
        if !violations.is_empty() {
            for (code, message, vspan) in violations {
                self.diags
                    .push(Diagnostic::new(code, message).with_label(vspan, "not allowed here"));
            }
            return None;
        }
        scope.push_frame();
        let result = self.check_block(body, scope);
        scope.pop_frame();
        let (hir_body, _tail_ty) = result?;
        let mut has_db_verb = false;
        let mut non_db_verb: Option<&'static str> = None;
        collect_transaction_verbs(&hir_body, &mut has_db_verb, &mut non_db_verb);
        if let Some(name) = non_db_verb {
            self.diags.push(
                Diagnostic::new(
                    ErrorCode::NonTransactionalEffect,
                    format!("`{name}` does not roll back with a database transaction"),
                )
                .with_label(span, "not allowed inside `transaction`"),
            );
            return None;
        }
        if !has_db_verb {
            self.diags.push(
                Diagnostic::new(
                    ErrorCode::InvalidTransactionBlock,
                    "`transaction` block contains no database verb",
                )
                .with_label(span, "add at least one `db.*` verb"),
            );
            return None;
        }
        Some(HirStmt::Transaction { body: hir_body })
    }

    /// Checks `expr`, resolving a bare identifier as an enum-variant
    /// literal when `expected` names an enum type and the identifier
    /// matches one of its variants and isn't a bound local — otherwise
    /// falls back to [`Self::check_expr`]. The only two call sites that
    /// need this (comparison RHS, record field values) pass `expected`;
    /// everywhere else a bare enum variant has no type to resolve against
    /// and stays an `UnknownName` error, matching the doc's examples.
    fn check_expr_expecting(
        &mut self,
        expr: &Expr,
        expected: Option<&HirType>,
        scope: &mut Scope,
    ) -> Option<HirExpr> {
        if let (Expr::Ident(ident), Some(HirType::Enum { variants })) = (expr, expected) {
            if scope.lookup(&ident.text).is_none() && variants.contains(&ident.text) {
                return Some(HirExpr::EnumLit {
                    variants: variants.clone(),
                    variant: ident.text.clone(),
                });
            }
        }
        self.check_expr(expr, scope)
    }

    fn check_expr(&mut self, expr: &Expr, scope: &mut Scope) -> Option<HirExpr> {
        match expr {
            Expr::Ident(ident) => match scope.lookup(&ident.text) {
                Some((slot, ty)) => Some(HirExpr::Local { slot, ty }),
                None => {
                    self.diags.push(
                        Diagnostic::new(
                            ErrorCode::UnknownName,
                            format!("unknown name `{}`", ident.text),
                        )
                        .with_label(ident.span, "not a parameter or `let` binding in scope"),
                    );
                    None
                }
            },
            Expr::Number { text, span } => {
                if text.contains('.') {
                    match text.parse::<f64>() {
                        Ok(v) => Some(HirExpr::FloatLit(v)),
                        Err(_) => {
                            self.diags.push(
                                Diagnostic::new(
                                    ErrorCode::HandlerExprTypeMismatch,
                                    format!("`{text}` is not a valid float literal"),
                                )
                                .with_label(*span, "invalid number"),
                            );
                            None
                        }
                    }
                } else {
                    match text.parse::<i64>() {
                        Ok(v) => Some(HirExpr::IntLit(v)),
                        Err(_) => {
                            self.diags.push(
                                Diagnostic::new(
                                    ErrorCode::HandlerExprTypeMismatch,
                                    format!("`{text}` is not a valid integer literal"),
                                )
                                .with_label(*span, "invalid number"),
                            );
                            None
                        }
                    }
                }
            }
            Expr::Str { value, .. } => Some(HirExpr::StrLit(value.clone())),
            Expr::Bool { value, .. } => Some(HirExpr::BoolLit(*value)),
            Expr::FieldAccess { base, field, span } => {
                let base_hir = self.check_expr(base, scope)?;
                let HirType::Record(rid) = base_hir.ty() else {
                    self.diags.push(
                        Diagnostic::new(
                            ErrorCode::HandlerExprTypeMismatch,
                            format!("field access on a non-record value (`{:?}`)", base_hir.ty()),
                        )
                        .with_label(*span, "not a record"),
                    );
                    return None;
                };
                let record = self.graph.record(rid);
                let Some(f) = record.fields.iter().find(|f| f.name == field.text) else {
                    let mut diag = Diagnostic::new(
                        ErrorCode::UnknownRecordField,
                        format!("record `{}` has no field `{}`", record.name, field.text),
                    )
                    .with_label(field.span, "unknown field");
                    if let Some(nearest) = nearest_field(&record.fields, &field.text) {
                        diag = diag.with_fix(Fix {
                            title: format!("Rename to `{nearest}`"),
                            edits: vec![Edit {
                                span: field.span,
                                replacement: nearest.to_owned(),
                            }],
                        });
                    }
                    self.diags.push(diag);
                    return None;
                };
                let ty = field_type_to_hir(&f.ty);
                Some(HirExpr::FieldAccess {
                    base: Box::new(base_hir),
                    field: field.text.clone(),
                    ty,
                })
            }
            Expr::Index { base, index, span } => {
                let base_hir = self.check_expr(base, scope)?;
                if base_hir.ty() != HirType::Json {
                    self.diags.push(
                        Diagnostic::new(
                            ErrorCode::HandlerExprTypeMismatch,
                            "`[..]` indexing is only valid on `Json` values",
                        )
                        .with_label(*span, "not a Json value"),
                    );
                    return None;
                }
                let index_hir = self.check_expr(index, scope)?;
                if index_hir.ty() != HirType::Str {
                    self.diags.push(
                        Diagnostic::new(
                            ErrorCode::HandlerExprTypeMismatch,
                            "a `Json` index must be a string key",
                        )
                        .with_label(*span, "expected a string"),
                    );
                    return None;
                }
                Some(HirExpr::Index {
                    base: Box::new(base_hir),
                    index: Box::new(index_hir),
                })
            }
            Expr::Call { callee, args, span } => self.check_call(callee, args, None, *span, scope),
            Expr::RecordCons { base, fields, span } => {
                self.check_record_cons(base, fields, *span, scope)
            }
            Expr::Binary { op, lhs, rhs, span } => self.check_binary(*op, lhs, rhs, *span, scope),
            Expr::Unary { op, expr, span } => self.check_unary(*op, expr, *span, scope),
            Expr::If {
                cond,
                then_branch,
                else_branch,
                span,
            } => self.check_if(cond, then_branch, else_branch.as_deref(), *span, scope),
            Expr::Match {
                scrutinee,
                arms,
                span,
            } => self.check_match_expr(scrutinee, arms, *span, scope),
            Expr::Query {
                call,
                predicate,
                span,
            } => {
                let Expr::Call { callee, args, .. } = call.as_ref() else {
                    self.diags.push(
                        Diagnostic::new(
                            ErrorCode::InvalidVerbCall,
                            "a `where` clause may only follow a capability verb call",
                        )
                        .with_label(*span, "not a call"),
                    );
                    return None;
                };
                self.check_call(callee, args, Some(predicate), *span, scope)
            }
        }
    }

    /// `<callee>(<args>)`: either a capability verb call
    /// (`db.insert(..)`) or a niladic builtin (`Uuid.new()`). `predicate`
    /// is `Some` when the call was wrapped in a `where` clause (v0.14
    /// M1) — only `db.query`/`db.count`/`db.delete_where` accept one.
    fn check_call(
        &mut self,
        callee: &Expr,
        args: &[Expr],
        predicate: Option<&ast::Predicate>,
        span: Span,
        scope: &mut Scope,
    ) -> Option<HirExpr> {
        let Expr::FieldAccess { base, field, .. } = callee else {
            self.diags.push(
                Diagnostic::new(
                    ErrorCode::InvalidVerbCall,
                    "calls must be `receiver.method(..)`",
                )
                .with_label(span, "not a capability verb or builtin call"),
            );
            return None;
        };
        let Expr::Ident(recv) = base.as_ref() else {
            self.diags.push(
                Diagnostic::new(
                    ErrorCode::InvalidVerbCall,
                    "calls must be `receiver.method(..)`",
                )
                .with_label(span, "not a capability verb or builtin call"),
            );
            return None;
        };
        match (recv.text.as_str(), field.text.as_str()) {
            ("Uuid", "new") if args.is_empty() && predicate.is_none() => {
                return Some(HirExpr::BuiltinCall(Builtin::UuidNew))
            }
            ("Timestamp", "now") if args.is_empty() && predicate.is_none() => {
                return Some(HirExpr::BuiltinCall(Builtin::TimestampNow))
            }
            ("Uuid", "new") | ("Timestamp", "now") => {
                self.diags.push(
                    Diagnostic::new(
                        ErrorCode::InvalidVerbCall,
                        format!("`{}.{}` takes no arguments", recv.text, field.text),
                    )
                    .with_label(span, "unexpected arguments"),
                );
                return None;
            }
            _ => {}
        }
        let Some(kind) = crate::build::binding_kind(&recv.text) else {
            self.diags.push(
                Diagnostic::new(
                    ErrorCode::UnknownName,
                    format!("`{}` is not a capability or builtin", recv.text),
                )
                .with_label(recv.span, "unknown receiver"),
            );
            return None;
        };
        self.check_verb_call(kind, recv, field, args, predicate, span, scope)
    }

    /// Resolves and type-checks a capability verb call against this
    /// milestone's closed verb set. `predicate` is `Some` when the
    /// source wrapped the call in a `where` clause (v0.14 M1) — only
    /// `db.query`/`db.count`/`db.delete_where` accept one.
    #[allow(clippy::too_many_arguments)]
    fn check_verb_call(
        &mut self,
        kind: NodeKind,
        recv: &Ident,
        verb_ident: &Ident,
        args: &[Expr],
        predicate: Option<&ast::Predicate>,
        span: Span,
        scope: &mut Scope,
    ) -> Option<HirExpr> {
        let Some(capability) =
            self.default_capability(kind, &format!("`{}.{}`", recv.text, verb_ident.text), span)
        else {
            self.diags.push(
                Diagnostic::new(
                    ErrorCode::VerbOnUnboundCapability,
                    format!(
                        "`{}.{}` has no bound `{}` instance in this service",
                        recv.text, verb_ident.text, recv.text
                    ),
                )
                .with_label(span, "unbound capability")
                .with_help(format!(
                    "add `{} <Provider>;` to the `use {{ .. }}` block",
                    recv.text
                )),
            );
            return None;
        };
        let is_query_verb = matches!(
            (kind, verb_ident.text.as_str()),
            (NodeKind::Database, "query" | "count" | "delete_where")
        );
        if predicate.is_some() && !is_query_verb {
            self.diags.push(
                Diagnostic::new(
                    ErrorCode::QueryModifierNotSupported,
                    format!(
                        "`{}.{}` does not accept a `where` clause",
                        recv.text, verb_ident.text
                    ),
                )
                .with_label(span, "unsupported `where` clause"),
            );
            return None;
        }
        let arity_error = |this: &mut Self, expected: usize| {
            this.diags.push(
                Diagnostic::new(
                    ErrorCode::InvalidVerbCall,
                    format!(
                        "`{}.{}` takes {expected} argument(s), found {}",
                        recv.text,
                        verb_ident.text,
                        args.len()
                    ),
                )
                .with_label(span, "wrong argument count"),
            );
        };
        match (kind, verb_ident.text.as_str()) {
            (NodeKind::Database, "insert") => {
                if args.len() != 2 {
                    arity_error(self, 2);
                    return None;
                }
                let Expr::Ident(table_ident) = &args[0] else {
                    self.diags.push(
                        Diagnostic::new(
                            ErrorCode::InvalidVerbCall,
                            "`db.insert` expects a table name",
                        )
                        .with_label(args[0].span(), "not a table name"),
                    );
                    return None;
                };
                let table_id = self.resolve_table(table_ident)?;
                let record_id = self.graph.table(table_id).record;
                let value = self.check_expr(&args[1], scope)?;
                if value.ty() != HirType::Record(record_id) {
                    self.diags.push(
                        Diagnostic::new(
                            ErrorCode::HandlerExprTypeMismatch,
                            format!(
                                "`db.insert` expects `{:?}`, found `{:?}`",
                                HirType::Record(record_id),
                                value.ty()
                            ),
                        )
                        .with_label(args[1].span(), "mismatched value type"),
                    );
                    return None;
                }
                Some(HirExpr::VerbCall {
                    capability,
                    verb: Verb::DbInsert(table_id),
                    args: vec![value],
                    ty: HirType::Record(record_id),
                })
            }
            (NodeKind::Database, "get") => {
                if args.len() != 2 {
                    arity_error(self, 2);
                    return None;
                }
                let Expr::Ident(table_ident) = &args[0] else {
                    self.diags.push(
                        Diagnostic::new(
                            ErrorCode::InvalidVerbCall,
                            "`db.get` expects a table name",
                        )
                        .with_label(args[0].span(), "not a table name"),
                    );
                    return None;
                };
                let table_id = self.resolve_table(table_ident)?;
                let record_id = self.graph.table(table_id).record;
                let key = self.check_expr(&args[1], scope)?;
                if key.ty() != HirType::Uuid {
                    self.diags.push(
                        Diagnostic::new(
                            ErrorCode::HandlerExprTypeMismatch,
                            format!("`db.get` key must be `Uuid`, found `{:?}`", key.ty()),
                        )
                        .with_label(args[1].span(), "mismatched key type"),
                    );
                    return None;
                }
                Some(HirExpr::VerbCall {
                    capability,
                    verb: Verb::DbGet(table_id),
                    args: vec![key],
                    ty: HirType::Option(Box::new(HirType::Record(record_id))),
                })
            }
            (NodeKind::Database, "update") => {
                if args.len() != 3 {
                    arity_error(self, 3);
                    return None;
                }
                let Expr::Ident(table_ident) = &args[0] else {
                    self.diags.push(
                        Diagnostic::new(
                            ErrorCode::InvalidVerbCall,
                            "`db.update` expects a table name",
                        )
                        .with_label(args[0].span(), "not a table name"),
                    );
                    return None;
                };
                let table_id = self.resolve_table(table_ident)?;
                let record_id = self.graph.table(table_id).record;
                let key = self.check_expr(&args[1], scope)?;
                if key.ty() != HirType::Uuid {
                    self.diags.push(
                        Diagnostic::new(
                            ErrorCode::HandlerExprTypeMismatch,
                            format!("`db.update` key must be `Uuid`, found `{:?}`", key.ty()),
                        )
                        .with_label(args[1].span(), "mismatched key type"),
                    );
                    return None;
                }
                let value = self.check_expr(&args[2], scope)?;
                if value.ty() != HirType::Record(record_id) {
                    self.diags.push(
                        Diagnostic::new(
                            ErrorCode::HandlerExprTypeMismatch,
                            format!(
                                "`db.update` expects `{:?}`, found `{:?}`",
                                HirType::Record(record_id),
                                value.ty()
                            ),
                        )
                        .with_label(args[2].span(), "mismatched value type"),
                    );
                    return None;
                }
                Some(HirExpr::VerbCall {
                    capability,
                    verb: Verb::DbUpdate(table_id),
                    args: vec![key, value],
                    ty: HirType::Option(Box::new(HirType::Record(record_id))),
                })
            }
            (NodeKind::Database, "delete") => {
                if args.len() != 2 {
                    arity_error(self, 2);
                    return None;
                }
                let Expr::Ident(table_ident) = &args[0] else {
                    self.diags.push(
                        Diagnostic::new(
                            ErrorCode::InvalidVerbCall,
                            "`db.delete` expects a table name",
                        )
                        .with_label(args[0].span(), "not a table name"),
                    );
                    return None;
                };
                let table_id = self.resolve_table(table_ident)?;
                let key = self.check_expr(&args[1], scope)?;
                if key.ty() != HirType::Uuid {
                    self.diags.push(
                        Diagnostic::new(
                            ErrorCode::HandlerExprTypeMismatch,
                            format!("`db.delete` key must be `Uuid`, found `{:?}`", key.ty()),
                        )
                        .with_label(args[1].span(), "mismatched key type"),
                    );
                    return None;
                }
                Some(HirExpr::VerbCall {
                    capability,
                    verb: Verb::DbDelete(table_id),
                    args: vec![key],
                    ty: HirType::Bool,
                })
            }
            (NodeKind::Database, verb @ ("query" | "count" | "delete_where")) => {
                if args.len() != 1 {
                    arity_error(self, 1);
                    return None;
                }
                let Expr::Ident(table_ident) = &args[0] else {
                    self.diags.push(
                        Diagnostic::new(
                            ErrorCode::InvalidVerbCall,
                            format!("`db.{verb}` expects a table name"),
                        )
                        .with_label(args[0].span(), "not a table name"),
                    );
                    return None;
                };
                let table_id = self.resolve_table(table_ident)?;
                let record_id = self.graph.table(table_id).record;
                let predicate = self.check_predicate(predicate, record_id, scope)?;
                let (hir_verb, ty) = match verb {
                    "query" => (
                        Verb::DbQuery(table_id),
                        HirType::List(Box::new(HirType::Record(record_id))),
                    ),
                    "count" => (Verb::DbCount(table_id), HirType::Int),
                    "delete_where" => (Verb::DbDeleteWhere(table_id), HirType::Int),
                    _ => unreachable!(),
                };
                Some(HirExpr::Query {
                    capability,
                    verb: hir_verb,
                    predicate,
                    ty,
                })
            }
            (NodeKind::Cache, "get") => {
                if args.len() != 1 {
                    arity_error(self, 1);
                    return None;
                }
                let key = self.check_expr(&args[0], scope)?;
                if key.ty() != HirType::Str {
                    self.diags.push(
                        Diagnostic::new(
                            ErrorCode::HandlerExprTypeMismatch,
                            format!("`cache.get` key must be a string, found `{:?}`", key.ty()),
                        )
                        .with_label(args[0].span(), "mismatched key type"),
                    );
                    return None;
                }
                Some(HirExpr::VerbCall {
                    capability,
                    verb: Verb::CacheGet,
                    args: vec![key],
                    ty: HirType::Option(Box::new(HirType::Json)),
                })
            }
            (NodeKind::Cache, "set") => {
                if args.len() != 2 {
                    arity_error(self, 2);
                    return None;
                }
                let key = self.check_expr(&args[0], scope)?;
                if key.ty() != HirType::Str {
                    self.diags.push(
                        Diagnostic::new(
                            ErrorCode::HandlerExprTypeMismatch,
                            format!("`cache.set` key must be a string, found `{:?}`", key.ty()),
                        )
                        .with_label(args[0].span(), "mismatched key type"),
                    );
                    return None;
                }
                let value = self.check_expr(&args[1], scope)?;
                Some(HirExpr::VerbCall {
                    capability,
                    verb: Verb::CacheSet,
                    args: vec![key, value],
                    ty: HirType::Unit,
                })
            }
            (NodeKind::Cache, "delete") => {
                if args.len() != 1 {
                    arity_error(self, 1);
                    return None;
                }
                let key = self.check_expr(&args[0], scope)?;
                if key.ty() != HirType::Str {
                    self.diags.push(
                        Diagnostic::new(
                            ErrorCode::HandlerExprTypeMismatch,
                            format!(
                                "`cache.delete` key must be a string, found `{:?}`",
                                key.ty()
                            ),
                        )
                        .with_label(args[0].span(), "mismatched key type"),
                    );
                    return None;
                }
                Some(HirExpr::VerbCall {
                    capability,
                    verb: Verb::CacheDelete,
                    args: vec![key],
                    ty: HirType::Unit,
                })
            }
            (NodeKind::ObjectStore, "put") => {
                if args.len() != 2 {
                    arity_error(self, 2);
                    return None;
                }
                let key = self.check_expr(&args[0], scope)?;
                if key.ty() != HirType::Str {
                    self.diags.push(
                        Diagnostic::new(
                            ErrorCode::HandlerExprTypeMismatch,
                            format!(
                                "`object_store.put` key must be a string, found `{:?}`",
                                key.ty()
                            ),
                        )
                        .with_label(args[0].span(), "mismatched key type"),
                    );
                    return None;
                }
                let value = self.check_expr(&args[1], scope)?;
                Some(HirExpr::VerbCall {
                    capability,
                    verb: Verb::ObjectStorePut,
                    args: vec![key, value],
                    ty: HirType::Unit,
                })
            }
            (NodeKind::ObjectStore, "get") => {
                if args.len() != 1 {
                    arity_error(self, 1);
                    return None;
                }
                let key = self.check_expr(&args[0], scope)?;
                if key.ty() != HirType::Str {
                    self.diags.push(
                        Diagnostic::new(
                            ErrorCode::HandlerExprTypeMismatch,
                            format!(
                                "`object_store.get` key must be a string, found `{:?}`",
                                key.ty()
                            ),
                        )
                        .with_label(args[0].span(), "mismatched key type"),
                    );
                    return None;
                }
                Some(HirExpr::VerbCall {
                    capability,
                    verb: Verb::ObjectStoreGet,
                    args: vec![key],
                    ty: HirType::Option(Box::new(HirType::Json)),
                })
            }
            (NodeKind::ObjectStore, "delete") => {
                if args.len() != 1 {
                    arity_error(self, 1);
                    return None;
                }
                let key = self.check_expr(&args[0], scope)?;
                if key.ty() != HirType::Str {
                    self.diags.push(
                        Diagnostic::new(
                            ErrorCode::HandlerExprTypeMismatch,
                            format!(
                                "`object_store.delete` key must be a string, found `{:?}`",
                                key.ty()
                            ),
                        )
                        .with_label(args[0].span(), "mismatched key type"),
                    );
                    return None;
                }
                Some(HirExpr::VerbCall {
                    capability,
                    verb: Verb::ObjectStoreDelete,
                    args: vec![key],
                    ty: HirType::Unit,
                })
            }
            (NodeKind::ObjectStore, "list") => {
                if args.len() != 1 {
                    arity_error(self, 1);
                    return None;
                }
                let prefix = self.check_expr(&args[0], scope)?;
                if prefix.ty() != HirType::Str {
                    self.diags.push(
                        Diagnostic::new(
                            ErrorCode::HandlerExprTypeMismatch,
                            format!(
                                "`object_store.list` prefix must be a string, found `{:?}`",
                                prefix.ty()
                            ),
                        )
                        .with_label(args[0].span(), "mismatched prefix type"),
                    );
                    return None;
                }
                Some(HirExpr::VerbCall {
                    capability,
                    verb: Verb::ObjectStoreList,
                    args: vec![prefix],
                    ty: HirType::List(Box::new(HirType::Str)),
                })
            }
            (NodeKind::Email, "send") => {
                if args.len() != 3 {
                    arity_error(self, 3);
                    return None;
                }
                let mut hir_args = Vec::with_capacity(3);
                for (arg, what) in args.iter().zip(["to", "subject", "body"]) {
                    let hir = self.check_expr(arg, scope)?;
                    if hir.ty() != HirType::Str {
                        self.diags.push(
                            Diagnostic::new(
                                ErrorCode::HandlerExprTypeMismatch,
                                format!(
                                    "`email.send` {what} must be a string, found `{:?}`",
                                    hir.ty()
                                ),
                            )
                            .with_label(arg.span(), "mismatched argument type"),
                        );
                        return None;
                    }
                    hir_args.push(hir);
                }
                Some(HirExpr::VerbCall {
                    capability,
                    verb: Verb::EmailSend,
                    args: hir_args,
                    ty: HirType::Unit,
                })
            }
            (NodeKind::Search, "index") => {
                if args.len() != 2 {
                    arity_error(self, 2);
                    return None;
                }
                let doc_id = self.check_expr(&args[0], scope)?;
                if doc_id.ty() != HirType::Str {
                    self.diags.push(
                        Diagnostic::new(
                            ErrorCode::HandlerExprTypeMismatch,
                            format!(
                                "`search.index` doc id must be a string, found `{:?}`",
                                doc_id.ty()
                            ),
                        )
                        .with_label(args[0].span(), "mismatched doc id type"),
                    );
                    return None;
                }
                // Any type is acceptable, like `cache.set`/`object_store.put`'s
                // payload — indexing serializes whatever value is given.
                let value = self.check_expr(&args[1], scope)?;
                Some(HirExpr::VerbCall {
                    capability,
                    verb: Verb::SearchIndex,
                    args: vec![doc_id, value],
                    ty: HirType::Unit,
                })
            }
            (NodeKind::Search, "query") => {
                if args.len() != 1 {
                    arity_error(self, 1);
                    return None;
                }
                let query = self.check_expr(&args[0], scope)?;
                if query.ty() != HirType::Str {
                    self.diags.push(
                        Diagnostic::new(
                            ErrorCode::HandlerExprTypeMismatch,
                            format!(
                                "`search.query` query must be a string, found `{:?}`",
                                query.ty()
                            ),
                        )
                        .with_label(args[0].span(), "mismatched query type"),
                    );
                    return None;
                }
                Some(HirExpr::VerbCall {
                    capability,
                    verb: Verb::SearchQuery,
                    args: vec![query],
                    ty: HirType::List(Box::new(HirType::Json)),
                })
            }
            (NodeKind::ExternalHttp, "request") => {
                if args.len() != 2 {
                    arity_error(self, 2);
                    return None;
                }
                let url = self.check_expr(&args[0], scope)?;
                if url.ty() != HirType::Str {
                    self.diags.push(
                        Diagnostic::new(
                            ErrorCode::HandlerExprTypeMismatch,
                            format!(
                                "`external_http.request` url must be a string, found `{:?}`",
                                url.ty()
                            ),
                        )
                        .with_label(args[0].span(), "mismatched url type"),
                    );
                    return None;
                }
                // Any type is acceptable, like `cache.set`/`object_store.put`'s
                // payload — the call serializes whatever value is given.
                let body = self.check_expr(&args[1], scope)?;
                Some(HirExpr::VerbCall {
                    capability,
                    verb: Verb::HttpCall,
                    args: vec![url, body],
                    ty: HirType::Json,
                })
            }
            _ => {
                self.diags.push(
                    Diagnostic::new(
                        ErrorCode::InvalidVerbCall,
                        format!(
                            "`{}` has no `{}` verb in this milestone's closed set",
                            recv.text, verb_ident.text
                        ),
                    )
                    .with_label(span, "unsupported verb"),
                );
                None
            }
        }
    }

    /// Type-checks an optional `where` clause against `record_id`'s
    /// fields (v0.14 M1). `None` in means `None` out (no clause was
    /// given — the query matches every row); the outer `Option` is
    /// `None` on error (already reported).
    fn check_predicate(
        &mut self,
        predicate: Option<&ast::Predicate>,
        record_id: RecordId,
        scope: &mut Scope,
    ) -> Option<Option<HirPredicate>> {
        let Some(predicate) = predicate else {
            return Some(None);
        };
        let record_fields = self.graph.record(record_id).fields.clone();
        let record_name = self.graph.record(record_id).name.clone();
        let mut terms = Vec::with_capacity(predicate.terms.len());
        for term in &predicate.terms {
            let Some(rf) = record_fields.iter().find(|f| f.name == term.field.text) else {
                self.diags.push(
                    Diagnostic::new(
                        ErrorCode::UnknownRecordField,
                        format!("record `{record_name}` has no field `{}`", term.field.text),
                    )
                    .with_label(term.field.span, "unknown field"),
                );
                return None;
            };
            let field_ty = field_type_to_hir(&rf.ty);
            let value = self.check_expr_expecting(&term.value, Some(&field_ty), scope)?;
            let op = if term.op == ast::PredOp::Contains {
                if field_ty != HirType::Str || value.ty() != HirType::Str {
                    self.diags.push(
                        Diagnostic::new(
                            ErrorCode::HandlerExprTypeMismatch,
                            format!(
                                "`contains` needs a `Str` field and a `Str` value, found \
                                 field `{field_ty:?}` and value `{:?}`",
                                value.ty()
                            ),
                        )
                        .with_label(term.span, "mismatched `contains`"),
                    );
                    return None;
                }
                ciac_ir::PredOp::Contains
            } else {
                if value.ty() != field_ty {
                    self.diags.push(
                        Diagnostic::new(
                            ErrorCode::HandlerExprTypeMismatch,
                            format!(
                                "field `{}` is `{field_ty:?}`, but this comparison uses `{:?}`",
                                term.field.text,
                                value.ty()
                            ),
                        )
                        .with_label(term.span, "mismatched comparison"),
                    );
                    return None;
                }
                match term.op {
                    ast::PredOp::Eq => ciac_ir::PredOp::Eq,
                    ast::PredOp::NotEq => ciac_ir::PredOp::NotEq,
                    ast::PredOp::Lt => ciac_ir::PredOp::Lt,
                    ast::PredOp::LtEq => ciac_ir::PredOp::LtEq,
                    ast::PredOp::Gt => ciac_ir::PredOp::Gt,
                    ast::PredOp::GtEq => ciac_ir::PredOp::GtEq,
                    ast::PredOp::Contains => unreachable!("handled above"),
                }
            };
            terms.push(HirPredTerm {
                field: term.field.text.clone(),
                field_ty,
                op,
                value,
            });
        }
        Some(Some(HirPredicate { terms }))
    }

    /// `<base> { field: value, .. }` — full construction if `base` names
    /// a declared record type, or a functional update if `base` is a
    /// record-typed local. See `Expr::RecordCons`'s doc comment: the
    /// grammar can't tell these apart, so this is where it's decided.
    fn check_record_cons(
        &mut self,
        base: &Expr,
        fields: &[FieldInit],
        span: Span,
        scope: &mut Scope,
    ) -> Option<HirExpr> {
        let Expr::Ident(base_ident) = base else {
            self.diags.push(
                Diagnostic::new(
                    ErrorCode::HandlerExprTypeMismatch,
                    "record construction/update must start with a plain name",
                )
                .with_label(span, "expected a record type or variable name"),
            );
            return None;
        };
        if let Some((slot, ty)) = scope.lookup(&base_ident.text) {
            let HirType::Record(rid) = ty else {
                self.diags.push(
                    Diagnostic::new(
                        ErrorCode::HandlerExprTypeMismatch,
                        format!(
                            "functional update requires a record-typed variable, found `{ty:?}`"
                        ),
                    )
                    .with_label(base_ident.span, "not a record value"),
                );
                return None;
            };
            let hir_fields = self.check_field_inits(rid, fields, scope, false, span)?;
            return Some(HirExpr::RecordCons {
                record: rid,
                base_value: Some(Box::new(HirExpr::Local {
                    slot,
                    ty: HirType::Record(rid),
                })),
                fields: hir_fields,
            });
        }
        let Some(rid) = self.graph.find_record(&base_ident.text) else {
            self.diags.push(
                Diagnostic::new(
                    ErrorCode::UnknownName,
                    format!(
                        "`{}` is neither a declared record type nor a variable in scope",
                        base_ident.text
                    ),
                )
                .with_label(base_ident.span, "unknown name"),
            );
            return None;
        };
        let hir_fields = self.check_field_inits(rid, fields, scope, true, span)?;
        Some(HirExpr::RecordCons {
            record: rid,
            base_value: None,
            fields: hir_fields,
        })
    }

    fn check_field_inits(
        &mut self,
        record: RecordId,
        fields: &[FieldInit],
        scope: &mut Scope,
        require_all: bool,
        span: Span,
    ) -> Option<Vec<(String, HirExpr)>> {
        let record_fields = self.graph.record(record).fields.clone();
        let record_name = self.graph.record(record).name.clone();
        let mut seen: HashMap<String, Span> = HashMap::new();
        let mut hir_fields = Vec::with_capacity(fields.len());
        for field in fields {
            let Some(rf) = record_fields.iter().find(|f| f.name == field.name.text) else {
                self.diags.push(
                    Diagnostic::new(
                        ErrorCode::UnknownRecordField,
                        format!("record `{record_name}` has no field `{}`", field.name.text),
                    )
                    .with_label(field.name.span, "unknown field"),
                );
                return None;
            };
            if let Some(first) = seen.get(&field.name.text) {
                self.diags.push(
                    Diagnostic::new(
                        ErrorCode::DuplicateDeclaration,
                        format!("field `{}` is initialized more than once", field.name.text),
                    )
                    .with_label(field.name.span, "duplicate here")
                    .with_label(*first, "first here"),
                );
                return None;
            }
            seen.insert(field.name.text.clone(), field.name.span);
            let expected = field_type_to_hir(&rf.ty);
            let value = self.check_expr_expecting(&field.value, Some(&expected), scope)?;
            if value.ty() != expected {
                self.diags.push(
                    Diagnostic::new(
                        ErrorCode::HandlerExprTypeMismatch,
                        format!(
                            "field `{}` expects `{expected:?}`, found `{:?}`",
                            field.name.text,
                            value.ty()
                        ),
                    )
                    .with_label(field.value.span(), "mismatched field value"),
                );
                return None;
            }
            hir_fields.push((field.name.text.clone(), value));
        }
        if require_all {
            let missing: Vec<&str> = record_fields
                .iter()
                .map(|f| f.name.as_str())
                .filter(|name| !seen.contains_key(*name))
                .collect();
            if !missing.is_empty() {
                self.diags.push(
                    Diagnostic::new(
                        ErrorCode::HandlerExprTypeMismatch,
                        format!(
                            "record construction for `{record_name}` is missing field(s): {}",
                            missing.join(", ")
                        ),
                    )
                    .with_label(span, "incomplete record construction"),
                );
                return None;
            }
        }
        Some(hir_fields)
    }

    fn check_binary(
        &mut self,
        op: ast::BinOp,
        lhs: &Expr,
        rhs: &Expr,
        span: Span,
        scope: &mut Scope,
    ) -> Option<HirExpr> {
        let lhs_hir = self.check_expr(lhs, scope)?;
        let lty = lhs_hir.ty();
        // `==`/`!=` against a bare enum variant (`v.status == Ready`) is
        // the only place a literal needs its type from context — resolve
        // it against the other side's type before falling back to the
        // generic checker (which has no way to know which enum a bare
        // name belongs to).
        let expect_rhs = matches!(op, ast::BinOp::Eq | ast::BinOp::NotEq).then_some(&lty);
        let rhs_hir = self.check_expr_expecting(rhs, expect_rhs, scope)?;
        let rty = rhs_hir.ty();
        let mismatch = |this: &mut Self| {
            this.diags.push(
                Diagnostic::new(
                    ErrorCode::HandlerExprTypeMismatch,
                    format!("`{op:?}` is not defined for `{lty:?}` and `{rty:?}`"),
                )
                .with_label(span, "mismatched operand types"),
            );
        };
        use ciac_ir::BinOp as H;
        let (hir_op, ty) = match op {
            ast::BinOp::Add => {
                if lty == HirType::Str || rty == HirType::Str {
                    if is_stringifiable(&lty) && is_stringifiable(&rty) {
                        (H::Add, HirType::Str)
                    } else {
                        mismatch(self);
                        return None;
                    }
                } else if lty == HirType::Int && rty == HirType::Int {
                    (H::Add, HirType::Int)
                } else if lty == HirType::Float && rty == HirType::Float {
                    (H::Add, HirType::Float)
                } else {
                    mismatch(self);
                    return None;
                }
            }
            ast::BinOp::Sub | ast::BinOp::Mul | ast::BinOp::Div => {
                let hir_op = match op {
                    ast::BinOp::Sub => H::Sub,
                    ast::BinOp::Mul => H::Mul,
                    ast::BinOp::Div => H::Div,
                    _ => unreachable!(),
                };
                if lty == HirType::Int && rty == HirType::Int {
                    (hir_op, HirType::Int)
                } else if lty == HirType::Float && rty == HirType::Float {
                    (hir_op, HirType::Float)
                } else {
                    mismatch(self);
                    return None;
                }
            }
            ast::BinOp::Eq | ast::BinOp::NotEq => {
                if lty != rty {
                    mismatch(self);
                    return None;
                }
                (
                    if op == ast::BinOp::Eq {
                        H::Eq
                    } else {
                        H::NotEq
                    },
                    HirType::Bool,
                )
            }
            ast::BinOp::Lt | ast::BinOp::LtEq | ast::BinOp::Gt | ast::BinOp::GtEq => {
                let ok = (lty == HirType::Int && rty == HirType::Int)
                    || (lty == HirType::Float && rty == HirType::Float);
                if !ok {
                    mismatch(self);
                    return None;
                }
                let hir_op = match op {
                    ast::BinOp::Lt => H::Lt,
                    ast::BinOp::LtEq => H::LtEq,
                    ast::BinOp::Gt => H::Gt,
                    ast::BinOp::GtEq => H::GtEq,
                    _ => unreachable!(),
                };
                (hir_op, HirType::Bool)
            }
            ast::BinOp::And | ast::BinOp::Or => {
                if lty != HirType::Bool || rty != HirType::Bool {
                    mismatch(self);
                    return None;
                }
                (
                    if op == ast::BinOp::And { H::And } else { H::Or },
                    HirType::Bool,
                )
            }
        };
        Some(HirExpr::Binary {
            op: hir_op,
            lhs: Box::new(lhs_hir),
            rhs: Box::new(rhs_hir),
            ty,
        })
    }

    fn check_unary(
        &mut self,
        op: ast::UnOp,
        expr: &Expr,
        span: Span,
        scope: &mut Scope,
    ) -> Option<HirExpr> {
        let hir = self.check_expr(expr, scope)?;
        let ty = hir.ty();
        use ciac_ir::UnOp as H;
        let (hir_op, result_ty) = match op {
            ast::UnOp::Neg if ty == HirType::Int => (H::Neg, HirType::Int),
            ast::UnOp::Neg if ty == HirType::Float => (H::Neg, HirType::Float),
            ast::UnOp::Not if ty == HirType::Bool => (H::Not, HirType::Bool),
            _ => {
                self.diags.push(
                    Diagnostic::new(
                        ErrorCode::HandlerExprTypeMismatch,
                        format!("`{op:?}` is not defined for `{ty:?}`"),
                    )
                    .with_label(span, "mismatched operand type"),
                );
                return None;
            }
        };
        Some(HirExpr::Unary {
            op: hir_op,
            expr: Box::new(hir),
            ty: result_ty,
        })
    }

    fn check_if(
        &mut self,
        cond: &Expr,
        then_branch: &[Stmt],
        else_branch: Option<&[Stmt]>,
        span: Span,
        scope: &mut Scope,
    ) -> Option<HirExpr> {
        let cond_hir = self.check_expr(cond, scope)?;
        if cond_hir.ty() != HirType::Bool {
            self.diags.push(
                Diagnostic::new(
                    ErrorCode::HandlerExprTypeMismatch,
                    format!("`if` condition must be `Bool`, found `{:?}`", cond_hir.ty()),
                )
                .with_label(cond.span(), "not a boolean"),
            );
            return None;
        }
        scope.push_frame();
        let then_result = self.check_block(then_branch, scope);
        scope.pop_frame();
        let (then_stmts, then_ty) = then_result?;
        let (else_stmts, else_ty) = match else_branch {
            Some(stmts) => {
                scope.push_frame();
                let result = self.check_block(stmts, scope);
                scope.pop_frame();
                result?
            }
            None => (Vec::new(), HirType::Unit),
        };
        let ty = match HirType::unify(then_ty, else_ty) {
            Ok(ty) => ty,
            Err((t1, t2)) => {
                self.diags.push(
                    Diagnostic::new(
                        ErrorCode::HandlerExprTypeMismatch,
                        format!("`if`/`else` branches disagree: `{t1:?}` vs `{t2:?}`"),
                    )
                    .with_label(span, "mismatched branch types"),
                );
                return None;
            }
        };
        Some(HirExpr::If {
            cond: Box::new(cond_hir),
            then_branch: then_stmts,
            else_branch: else_stmts,
            ty,
        })
    }

    fn check_match_expr(
        &mut self,
        scrutinee: &Expr,
        arms: &[ast::ExprArm],
        span: Span,
        scope: &mut Scope,
    ) -> Option<HirExpr> {
        let scrutinee_hir = self.check_expr(scrutinee, scope)?;
        let HirType::Enum { variants } = scrutinee_hir.ty() else {
            self.diags.push(
                Diagnostic::new(
                    ErrorCode::HandlerExprTypeMismatch,
                    format!(
                        "`match` requires an enum-typed scrutinee, found `{:?}`",
                        scrutinee_hir.ty()
                    ),
                )
                .with_label(scrutinee.span(), "not an enum value"),
            );
            return None;
        };
        let labels: Vec<&ArmLabel> = arms.iter().map(|arm| &arm.label).collect();
        let resolved_labels = self.check_match_labels(&variants, &labels, "match expression", span);
        let mut hir_arms = Vec::with_capacity(arms.len());
        let mut result_ty = HirType::Never;
        for (arm, label) in arms.iter().zip(resolved_labels) {
            scope.push_frame();
            let result = self.check_block(&arm.body, scope);
            scope.pop_frame();
            let (body, arm_ty) = result?;
            result_ty = match HirType::unify(result_ty, arm_ty) {
                Ok(ty) => ty,
                Err((t1, t2)) => {
                    self.diags.push(
                        Diagnostic::new(
                            ErrorCode::HandlerExprTypeMismatch,
                            format!("match arms disagree: `{t1:?}` vs `{t2:?}`"),
                        )
                        .with_label(span, "mismatched arm types"),
                    );
                    return None;
                }
            };
            hir_arms.push(HirArm {
                variant: label,
                body,
            });
        }
        let ty = result_ty;
        Some(HirExpr::Match {
            scrutinee: Box::new(scrutinee_hir),
            arms: hir_arms,
            ty,
        })
    }
}

#[cfg(test)]
mod fix_tests {
    use super::*;

    fn field(name: &str) -> RecordField {
        RecordField {
            name: name.to_owned(),
            ty: FieldType::Str,
        }
    }

    #[test]
    fn nearest_field_finds_a_close_typo_but_not_a_stranger() {
        let fields = vec![field("id"), field("name"), field("email")];
        assert_eq!(nearest_field(&fields, "nam"), Some("name"));
        assert_eq!(nearest_field(&fields, "emial"), Some("email"));
        assert_eq!(nearest_field(&fields, "totally_unrelated_xyz"), None);
    }
}
