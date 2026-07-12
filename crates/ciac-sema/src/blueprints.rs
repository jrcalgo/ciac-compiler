//! v0.8 M2: blueprint expansion — parameterized templates instantiated
//! per `expand` site into ordinary items, resolved *before*
//! [`crate::build::build_graph`] ever runs. By the time `build_graph`
//! sees the returned [`Program`], every `blueprint`/`expand` item is
//! gone, replaced by the same `use`/`crud`/`stream`/`handler` items a
//! hand-written program would contain — every existing pass needs zero
//! awareness that blueprints exist, exactly like `Item::Import`
//! resolution one layer up in `ciac_syntax::module`.
//!
//! Scope (deliberately narrow — see 08UpdatePlan.md's own risk
//! warnings about this feature): one generic type parameter per
//! blueprint, constrained to `record` only; hygiene is a single rule
//! (every name a blueprint body declares is suffixed with the
//! concrete type argument's name, unless a caller-supplied `params`
//! value overrides it). v0.8 M2 shipped `use`/`crud`/`stream`/
//! `handler` bodies; v0.14 M5 grows this to `record`/`table`/`api`/
//! `worker`/`pipeline`, so a body can declare a self-contained
//! api-plus-pipeline shape — a `pipeline` in the body attaches to an
//! `api`/`worker` in the *same* body because both share the same
//! hygiene rename (see `instantiate`'s renames map).

use ciac_diagnostics::{Diagnostic, Diagnostics, ErrorCode};
use ciac_syntax::ast::{
    ApiDecl, Arm, ArmLabel, Attr, AttrValue, BlueprintDecl, BlueprintItem, CrudDecl, ExpandStmt,
    Expr, ExprArm, Field, FieldInit, HandlerDecl, Ident, Item, Param, PipelineDecl, PredTerm,
    Predicate, Program, RecordDecl, ServiceBlock, ServiceItem, StepExpr, Stmt, StreamDecl,
    TableDecl, TypeExpr, WorkerDecl,
};
use std::collections::HashMap;

/// Expands every `blueprint`/`expand` item in `program` into ordinary
/// items, returning a new [`Program`] with none left. Errors (unknown
/// blueprint, arity mismatch, constraint violation) are pushed into
/// `diags`; the `expand` site producing one contributes nothing to the
/// output, same as any other invalid declaration.
pub fn expand(program: &Program, diags: &mut Diagnostics) -> Program {
    let mut blueprints: HashMap<&str, &BlueprintDecl> = HashMap::new();
    for item in &program.items {
        if let Item::Blueprint(decl) = item {
            if let Some(existing) = blueprints.get(decl.name.text.as_str()) {
                diags.push(
                    Diagnostic::new(
                        ErrorCode::DuplicateDeclaration,
                        format!("blueprint `{}` is declared more than once", decl.name.text),
                    )
                    .with_label(decl.span, "duplicate declaration here")
                    .with_label(existing.span, "first declared here"),
                );
                continue;
            }
            blueprints.insert(&decl.name.text, decl);
        }
    }

    let record_names: Vec<&str> = program
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Record(decl) => Some(decl.name.text.as_str()),
            _ => None,
        })
        .collect();

    let mut items = Vec::new();
    let mut hoisted = Vec::new();
    for item in &program.items {
        match item {
            Item::Blueprint(_) => {}
            Item::Expand(stmt) => {
                items.extend(expand_at_top_level(stmt, &blueprints, &record_names, diags));
            }
            Item::ServiceBlock(block) => {
                let (expanded_block, block_hoisted) =
                    expand_service_block(block, &blueprints, &record_names, diags);
                items.push(Item::ServiceBlock(expanded_block));
                hoisted.extend(block_hoisted);
            }
            other => items.push(other.clone()),
        }
    }
    items.extend(hoisted);
    Program { items }
}

fn expand_at_top_level(
    stmt: &ExpandStmt,
    blueprints: &HashMap<&str, &BlueprintDecl>,
    record_names: &[&str],
    diags: &mut Diagnostics,
) -> Vec<Item> {
    let Some(instantiated) = instantiate(stmt, blueprints, record_names, diags) else {
        return Vec::new();
    };
    instantiated
        .into_iter()
        .map(|item| match item {
            BlueprintItem::Use(u) => Item::Use(u),
            BlueprintItem::Crud(c) => Item::Crud(c),
            BlueprintItem::Stream(s) => Item::Stream(s),
            BlueprintItem::Handler(h) => Item::Handler(h),
            BlueprintItem::Record(r) => Item::Record(r),
            BlueprintItem::Table(t) => Item::Table(t),
            BlueprintItem::Api(a) => Item::Api(a),
            BlueprintItem::Worker(w) => Item::Worker(w),
            BlueprintItem::Pipeline(p) => Item::Pipeline(p),
        })
        .collect()
}

/// Expands every `ServiceItem::Expand` inside `block`, returning the
/// block with them replaced by ordinary `ServiceItem`s, plus any
/// `stream` items the expansion produced — those always hoist to the
/// enclosing [`Program`]'s top level, since `ServiceItem` (unlike
/// `Item`) has no `Stream` variant, even hand-written.
fn expand_service_block(
    block: &ServiceBlock,
    blueprints: &HashMap<&str, &BlueprintDecl>,
    record_names: &[&str],
    diags: &mut Diagnostics,
) -> (ServiceBlock, Vec<Item>) {
    let mut items = Vec::new();
    let mut hoisted = Vec::new();
    for item in &block.items {
        match item {
            ServiceItem::Expand(stmt) => {
                let Some(instantiated) = instantiate(stmt, blueprints, record_names, diags) else {
                    continue;
                };
                for item in instantiated {
                    match item {
                        BlueprintItem::Use(u) => items.push(ServiceItem::Use(u)),
                        BlueprintItem::Crud(c) => items.push(ServiceItem::Crud(c)),
                        BlueprintItem::Handler(h) => items.push(ServiceItem::Handler(h)),
                        BlueprintItem::Api(a) => items.push(ServiceItem::Api(a)),
                        BlueprintItem::Worker(w) => items.push(ServiceItem::Worker(w)),
                        BlueprintItem::Pipeline(p) => items.push(ServiceItem::Pipeline(p)),
                        // `ServiceItem` has no `Stream`/`Record`/`Table`
                        // variant — hand-written programs declare these
                        // at the top level even inside a `service { .. }`
                        // block, so a blueprint-produced one hoists the
                        // same way.
                        BlueprintItem::Stream(s) => hoisted.push(Item::Stream(s)),
                        BlueprintItem::Record(r) => hoisted.push(Item::Record(r)),
                        BlueprintItem::Table(t) => hoisted.push(Item::Table(t)),
                    }
                }
            }
            other => items.push(other.clone()),
        }
    }
    (
        ServiceBlock {
            name: block.name.clone(),
            items,
            span: block.span,
        },
        hoisted,
    )
}

/// Validates one `expand` site (unknown blueprint: `CIAC0048`; arity/
/// param mismatch: `CIAC0049`; type argument isn't a record: `CIAC0050`)
/// and, if valid, returns the blueprint's body with the generic type
/// parameter and scalar params substituted and every declared name
/// hygienically renamed. `None` means a diagnostic was already pushed.
fn instantiate(
    stmt: &ExpandStmt,
    blueprints: &HashMap<&str, &BlueprintDecl>,
    record_names: &[&str],
    diags: &mut Diagnostics,
) -> Option<Vec<BlueprintItem>> {
    let Some(blueprint) = blueprints.get(stmt.blueprint.text.as_str()) else {
        diags.push(
            Diagnostic::new(
                ErrorCode::UnknownBlueprint,
                format!("unknown blueprint `{}`", stmt.blueprint.text),
            )
            .with_label(stmt.blueprint.span, "no `blueprint` declares this name"),
        );
        return None;
    };

    if !record_names.contains(&stmt.type_arg.text.as_str()) {
        diags.push(
            Diagnostic::new(
                ErrorCode::BlueprintConstraintViolation,
                format!("`{}` is not a declared `record`", stmt.type_arg.text),
            )
            .with_label(stmt.type_arg.span, "expected a record name here")
            .with_help(format!(
                "`{}`'s type parameter `{}` is constrained to `record`",
                blueprint.name.text, blueprint.type_param.text
            )),
        );
        return None;
    }

    let mut values: HashMap<&str, &AttrValue> = HashMap::new();
    for arg in &stmt.args {
        if !blueprint
            .params
            .iter()
            .any(|p| p.name.text == arg.name.text)
        {
            diags.push(
                Diagnostic::new(
                    ErrorCode::BlueprintArityMismatch,
                    format!(
                        "`{}` has no param named `{}`",
                        blueprint.name.text, arg.name.text
                    ),
                )
                .with_label(arg.span, "unknown param"),
            );
            return None;
        }
        values.insert(arg.name.text.as_str(), &arg.value);
    }
    for param in &blueprint.params {
        let Some(value) = values.get(param.name.text.as_str()) else {
            diags.push(
                Diagnostic::new(
                    ErrorCode::BlueprintArityMismatch,
                    format!(
                        "missing required param `{}` for `{}`",
                        param.name.text, blueprint.name.text
                    ),
                )
                .with_label(stmt.span, "expand site is missing this param"),
            );
            return None;
        };
        if !param_value_matches(&param.ty, value) {
            diags.push(
                Diagnostic::new(
                    ErrorCode::BlueprintArityMismatch,
                    format!(
                        "param `{}` expects a value matching `{}`",
                        param.name.text,
                        type_name(&param.ty)
                    ),
                )
                .with_label(value.span(), "wrong value type"),
            );
            return None;
        }
    }
    let params: HashMap<String, AttrValue> = values
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value.clone()))
        .collect();

    // Hygiene: a declared name that's also a declared `params` entry
    // (e.g. `crud name: R;` alongside `params { name: String; }`) takes
    // its literal param value verbatim — the caller asked for that
    // exact name (this is what lets a blueprint faithfully wrap an
    // existing primitive, e.g. `std/crud.ciac`, matching hand-written
    // output exactly). Every other declared name gets suffixed with
    // the concrete type argument's name instead, so two expansions of
    // the same blueprint with different type args in the same scope
    // never collide (the same blueprint expanded twice with the *same*
    // type arg — or the same explicit param name twice — still
    // collides, falling through to the ordinary duplicate-declaration
    // check downstream, same as two hand-written declarations with the
    // same name).
    let mut renames: HashMap<String, String> = HashMap::new();
    for item in &blueprint.body {
        let name = match item {
            BlueprintItem::Crud(d) => &d.name,
            BlueprintItem::Stream(d) => &d.name,
            BlueprintItem::Handler(d) => &d.name,
            BlueprintItem::Record(d) => &d.name,
            BlueprintItem::Table(d) => &d.name,
            BlueprintItem::Api(d) => &d.name,
            BlueprintItem::Worker(d) => &d.name,
            // A `pipeline`'s own name isn't a fresh declaration — it
            // names the `api`/`worker` it attaches to, which already
            // contributes (the same original name maps to the same
            // renamed name either way, so this loop iteration is a
            // no-op duplicate insert, not a conflict).
            BlueprintItem::Pipeline(d) => &d.name,
            BlueprintItem::Use(_) => continue,
        };
        let replacement = match params.get(name.text.as_str()) {
            Some(AttrValue::Str { value, .. }) => value.clone(),
            _ => format!("{}{}", name.text, stmt.type_arg.text),
        };
        renames.insert(name.text.clone(), replacement);
    }

    Some(
        blueprint
            .body
            .iter()
            .map(|item| substitute_item(item, blueprint, stmt, &renames, &params))
            .collect(),
    )
}

fn param_value_matches(ty: &TypeExpr, value: &AttrValue) -> bool {
    match (ty, value) {
        (TypeExpr::Named(t), AttrValue::Str { .. }) => t.text == "String",
        (TypeExpr::Named(t), AttrValue::Number { .. }) => t.text == "Int",
        _ => false,
    }
}

fn type_name(ty: &TypeExpr) -> String {
    match ty {
        TypeExpr::Named(ident) => ident.text.clone(),
        TypeExpr::Enum { .. } => "enum".to_owned(),
        TypeExpr::List { .. } => "list".to_owned(),
    }
}

fn substitute_item(
    item: &BlueprintItem,
    blueprint: &BlueprintDecl,
    stmt: &ExpandStmt,
    renames: &HashMap<String, String>,
    params: &HashMap<String, AttrValue>,
) -> BlueprintItem {
    match item {
        BlueprintItem::Use(u) => BlueprintItem::Use(u.clone()),
        BlueprintItem::Crud(d) => BlueprintItem::Crud(CrudDecl {
            name: renamed_ident(&d.name, renames),
            record: d
                .record
                .as_ref()
                .map(|r| substitute_reference(r, blueprint, stmt, renames)),
            attrs: substitute_attrs(&d.attrs, params),
            span: d.span,
        }),
        BlueprintItem::Stream(d) => BlueprintItem::Stream(StreamDecl {
            name: renamed_ident(&d.name, renames),
            record: substitute_reference(&d.record, blueprint, stmt, renames),
            attrs: substitute_attrs(&d.attrs, params),
            span: d.span,
        }),
        BlueprintItem::Handler(d) => BlueprintItem::Handler(HandlerDecl {
            name: renamed_ident(&d.name, renames),
            bindings: d.bindings.clone(),
            params: d
                .params
                .iter()
                .map(|p| Param {
                    name: p.name.clone(),
                    ty: substitute_type(&p.ty, blueprint, stmt, renames),
                    span: p.span,
                })
                .collect(),
            return_ty: d
                .return_ty
                .as_ref()
                .map(|ty| substitute_type(ty, blueprint, stmt, renames)),
            body: d
                .body
                .as_ref()
                .map(|stmts| rewrite_stmts(stmts, renames, params)),
            is_extern: d.is_extern,
            span: d.span,
        }),
        BlueprintItem::Record(d) => BlueprintItem::Record(RecordDecl {
            name: renamed_ident(&d.name, renames),
            fields: d
                .fields
                .iter()
                .map(|f| Field {
                    name: f.name.clone(),
                    ty: substitute_type(&f.ty, blueprint, stmt, renames),
                    span: f.span,
                })
                .collect(),
            kind: d.kind,
            span: d.span,
        }),
        BlueprintItem::Table(d) => BlueprintItem::Table(TableDecl {
            name: renamed_ident(&d.name, renames),
            record: substitute_reference(&d.record, blueprint, stmt, renames),
            span: d.span,
        }),
        BlueprintItem::Api(d) => BlueprintItem::Api(ApiDecl {
            name: renamed_ident(&d.name, renames),
            request: d
                .request
                .as_ref()
                .map(|r| substitute_reference(r, blueprint, stmt, renames)),
            attrs: substitute_attrs(&d.attrs, params),
            span: d.span,
        }),
        BlueprintItem::Worker(d) => BlueprintItem::Worker(WorkerDecl {
            name: renamed_ident(&d.name, renames),
            stream: d
                .stream
                .as_ref()
                .map(|s| substitute_reference(s, blueprint, stmt, renames)),
            attrs: substitute_attrs(&d.attrs, params),
            span: d.span,
        }),
        BlueprintItem::Pipeline(d) => BlueprintItem::Pipeline(PipelineDecl {
            name: renamed_ident(&d.name, renames),
            steps: d.steps.iter().map(|s| rewrite_step(s, renames)).collect(),
            span: d.span,
        }),
    }
}

fn renamed_ident(ident: &Ident, renames: &HashMap<String, String>) -> Ident {
    match renames.get(&ident.text) {
        Some(new_name) => Ident {
            text: new_name.clone(),
            span: ident.span,
        },
        None => ident.clone(),
    }
}

/// Resolves a name a body item refers to *by name* rather than
/// declaring itself: the generic type parameter (e.g. `crud
/// Resource: R;`'s `R`) substitutes to the concrete type argument;
/// any other name falls back to `renames` (a body-local declaration,
/// e.g. a `table`'s `record` naming a `record` declared earlier in
/// the same body) and otherwise passes through unchanged (e.g.
/// `stream Audited: AuditEvent;`'s unrelated, non-body-local
/// `AuditEvent`).
fn substitute_reference(
    ident: &Ident,
    blueprint: &BlueprintDecl,
    stmt: &ExpandStmt,
    renames: &HashMap<String, String>,
) -> Ident {
    if ident.text == blueprint.type_param.text {
        Ident {
            text: stmt.type_arg.text.clone(),
            span: ident.span,
        }
    } else {
        renamed_ident(ident, renames)
    }
}

fn substitute_type(
    ty: &TypeExpr,
    blueprint: &BlueprintDecl,
    stmt: &ExpandStmt,
    renames: &HashMap<String, String>,
) -> TypeExpr {
    match ty {
        TypeExpr::Named(ident) => {
            TypeExpr::Named(substitute_reference(ident, blueprint, stmt, renames))
        }
        TypeExpr::List { inner, span } => TypeExpr::List {
            inner: Box::new(substitute_type(inner, blueprint, stmt, renames)),
            span: *span,
        },
        other => other.clone(),
    }
}

fn substitute_attrs(attrs: &[Attr], params: &HashMap<String, AttrValue>) -> Vec<Attr> {
    attrs
        .iter()
        .map(|attr| {
            if let AttrValue::Ident(ident) = &attr.value {
                if let Some(value) = params.get(&ident.text) {
                    return Attr {
                        name: attr.name.clone(),
                        value: value.clone(),
                        span: attr.span,
                    };
                }
            }
            attr.clone()
        })
        .collect()
}

/// Rewrites every `publish <Stream>(..)` inside a blueprint-owned
/// handler body whose `<Stream>` names one of the blueprint's *own*
/// (hygienically renamed) streams, so the body stays internally
/// consistent with the rest of the expansion; also substitutes any
/// bare identifier matching a `params` name with the caller-supplied
/// literal (v0.14 M5 — lets a body compare against/pass along a
/// scalar the `expand` site configured, e.g. a rate limiter's request
/// cap, the same way an attribute value already could). Nothing else
/// in a body needs rewriting — generic-type substitution never
/// reaches into statements/expressions beyond this, only
/// declaration-level type positions. See `rewrite_step` below for the
/// equivalent pass over pipeline steps.
fn rewrite_stmts(
    stmts: &[Stmt],
    renames: &HashMap<String, String>,
    params: &HashMap<String, AttrValue>,
) -> Vec<Stmt> {
    stmts
        .iter()
        .map(|s| rewrite_stmt(s, renames, params))
        .collect()
}

fn rewrite_stmt(
    stmt: &Stmt,
    renames: &HashMap<String, String>,
    params: &HashMap<String, AttrValue>,
) -> Stmt {
    match stmt {
        Stmt::Let { name, value, span } => Stmt::Let {
            name: name.clone(),
            value: rewrite_expr(value, renames, params),
            span: *span,
        },
        Stmt::Expr(e) => Stmt::Expr(rewrite_expr(e, renames, params)),
        Stmt::Return { value, span } => Stmt::Return {
            value: value.as_ref().map(|v| rewrite_expr(v, renames, params)),
            span: *span,
        },
        Stmt::Fail { error, args, span } => Stmt::Fail {
            error: renamed_ident(error, renames),
            args: args
                .iter()
                .map(|a| rewrite_expr(a, renames, params))
                .collect(),
            span: *span,
        },
        Stmt::Publish {
            stream,
            value,
            span,
        } => Stmt::Publish {
            stream: renamed_ident(stream, renames),
            value: rewrite_expr(value, renames, params),
            span: *span,
        },
    }
}

fn rewrite_expr(
    expr: &Expr,
    renames: &HashMap<String, String>,
    params: &HashMap<String, AttrValue>,
) -> Expr {
    match expr {
        // A bare ident in a handler body is a local var/param name (not
        // hygiene-tracked — lexically scoped to the handler, never
        // collides across expansions), a `params` scalar, or a
        // reference to one of the body's *own* declared names (a
        // `table`/`record` named directly, e.g. `db.count(Tickets)`'s
        // `Tickets` or a `RecordCons`'s `base`) — `renamed_ident`
        // passes through anything not in `renames`, so this is safe to
        // apply unconditionally once `params` has had first refusal.
        Expr::Ident(ident) => match params.get(&ident.text) {
            Some(value) => param_literal_expr(value, ident.span),
            None => Expr::Ident(renamed_ident(ident, renames)),
        },
        Expr::Number { .. } | Expr::Str { .. } | Expr::Bool { .. } => expr.clone(),
        Expr::FieldAccess { base, field, span } => Expr::FieldAccess {
            base: Box::new(rewrite_expr(base, renames, params)),
            field: field.clone(),
            span: *span,
        },
        Expr::Index { base, index, span } => Expr::Index {
            base: Box::new(rewrite_expr(base, renames, params)),
            index: Box::new(rewrite_expr(index, renames, params)),
            span: *span,
        },
        Expr::Call { callee, args, span } => Expr::Call {
            callee: Box::new(rewrite_expr(callee, renames, params)),
            args: args
                .iter()
                .map(|a| rewrite_expr(a, renames, params))
                .collect(),
            span: *span,
        },
        Expr::RecordCons { base, fields, span } => Expr::RecordCons {
            base: Box::new(rewrite_expr(base, renames, params)),
            fields: fields
                .iter()
                .map(|f| FieldInit {
                    name: f.name.clone(),
                    value: rewrite_expr(&f.value, renames, params),
                    span: f.span,
                })
                .collect(),
            span: *span,
        },
        Expr::Binary { op, lhs, rhs, span } => Expr::Binary {
            op: *op,
            lhs: Box::new(rewrite_expr(lhs, renames, params)),
            rhs: Box::new(rewrite_expr(rhs, renames, params)),
            span: *span,
        },
        Expr::Unary {
            op,
            expr: inner,
            span,
        } => Expr::Unary {
            op: *op,
            expr: Box::new(rewrite_expr(inner, renames, params)),
            span: *span,
        },
        Expr::If {
            cond,
            then_branch,
            else_branch,
            span,
        } => Expr::If {
            cond: Box::new(rewrite_expr(cond, renames, params)),
            then_branch: rewrite_stmts(then_branch, renames, params),
            else_branch: else_branch
                .as_ref()
                .map(|b| rewrite_stmts(b, renames, params)),
            span: *span,
        },
        Expr::Match {
            scrutinee,
            arms,
            span,
        } => Expr::Match {
            scrutinee: Box::new(rewrite_expr(scrutinee, renames, params)),
            arms: arms
                .iter()
                .map(|arm| ExprArm {
                    label: arm.label.clone(),
                    body: rewrite_stmts(&arm.body, renames, params),
                    span: arm.span,
                })
                .collect(),
            span: *span,
        },
        Expr::Query {
            call,
            predicate,
            span,
        } => Expr::Query {
            call: Box::new(rewrite_expr(call, renames, params)),
            predicate: Predicate {
                terms: predicate
                    .terms
                    .iter()
                    .map(|t| PredTerm {
                        field: t.field.clone(),
                        op: t.op,
                        value: rewrite_expr(&t.value, renames, params),
                        span: t.span,
                    })
                    .collect(),
                span: predicate.span,
            },
            span: *span,
        },
    }
}

/// Renders a `params` value as the literal expression it stands for,
/// at the span of the identifier it's replacing (so a type error in
/// the substituted body still points at the reference site, not the
/// distant `expand` statement).
fn param_literal_expr(value: &AttrValue, span: ciac_diagnostics::Span) -> Expr {
    match value {
        AttrValue::Str { value, .. } => Expr::Str {
            value: value.clone(),
            span,
        },
        AttrValue::Number { value, .. } => Expr::Number {
            text: value.to_string(),
            span,
        },
        AttrValue::Ident(ident) => Expr::Ident(Ident {
            text: ident.text.clone(),
            span,
        }),
    }
}

/// Rewrites one pipeline step so it keeps pointing at the same
/// (hygienically renamed) declaration after expansion. `Name` covers
/// both a body-local handler reference and the three builtin step
/// names (`Auth`/`Queue`/`Return`) — builtins are never in `renames`,
/// so `renamed_ident` passes them through unchanged, same as any
/// other name the body didn't itself declare. `Call` (`call
/// <Service>.<Api>`) always names something outside the blueprint by
/// construction — a *different* service's api — so it's never
/// rewritten.
fn rewrite_step(step: &StepExpr, renames: &HashMap<String, String>) -> StepExpr {
    match step {
        StepExpr::Name(ident) => StepExpr::Name(renamed_ident(ident, renames)),
        StepExpr::Publish(ident) => StepExpr::Publish(renamed_ident(ident, renames)),
        StepExpr::Call(target) => StepExpr::Call(target.clone()),
        StepExpr::Match { field, arms, span } => StepExpr::Match {
            field: field.clone(),
            arms: arms.iter().map(|a| rewrite_arm(a, renames)).collect(),
            span: *span,
        },
    }
}

fn rewrite_arm(arm: &Arm, renames: &HashMap<String, String>) -> Arm {
    Arm {
        label: match &arm.label {
            ArmLabel::Variant(ident) => ArmLabel::Variant(ident.clone()),
            ArmLabel::Default(span) => ArmLabel::Default(*span),
        },
        steps: arm.steps.iter().map(|s| rewrite_step(s, renames)).collect(),
        span: arm.span,
    }
}
