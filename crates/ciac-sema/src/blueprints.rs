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
//! blueprint, constrained to `record` only; blueprint bodies may
//! contain only `use`/`crud`/`stream`/`handler`; hygiene is a single
//! rule (every name a blueprint body declares is suffixed with the
//! concrete type argument's name).

use ciac_diagnostics::{Diagnostic, Diagnostics, ErrorCode};
use ciac_syntax::ast::{
    Attr, AttrValue, BlueprintDecl, BlueprintItem, CrudDecl, ExpandStmt, Expr, ExprArm, FieldInit,
    HandlerDecl, Ident, Item, Param, Program, ServiceBlock, ServiceItem, Stmt, StreamDecl,
    TypeExpr,
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
                        BlueprintItem::Stream(s) => hoisted.push(Item::Stream(s)),
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
                .map(|r| substitute_type_ident(r, blueprint, stmt)),
            attrs: substitute_attrs(&d.attrs, params),
            span: d.span,
        }),
        BlueprintItem::Stream(d) => BlueprintItem::Stream(StreamDecl {
            name: renamed_ident(&d.name, renames),
            record: substitute_type_ident(&d.record, blueprint, stmt),
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
                    ty: substitute_type(&p.ty, blueprint, stmt),
                    span: p.span,
                })
                .collect(),
            return_ty: d
                .return_ty
                .as_ref()
                .map(|ty| substitute_type(ty, blueprint, stmt)),
            body: d.body.as_ref().map(|stmts| rewrite_stmts(stmts, renames)),
            is_extern: d.is_extern,
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

/// Substitutes the generic type parameter when `ident` names it
/// exactly (e.g. `crud Resource: R;`'s `R`), otherwise leaves it as-is
/// (e.g. `stream Audited: AuditEvent;`'s unrelated `AuditEvent`).
fn substitute_type_ident(ident: &Ident, blueprint: &BlueprintDecl, stmt: &ExpandStmt) -> Ident {
    if ident.text == blueprint.type_param.text {
        Ident {
            text: stmt.type_arg.text.clone(),
            span: ident.span,
        }
    } else {
        ident.clone()
    }
}

fn substitute_type(ty: &TypeExpr, blueprint: &BlueprintDecl, stmt: &ExpandStmt) -> TypeExpr {
    match ty {
        TypeExpr::Named(ident) => TypeExpr::Named(substitute_type_ident(ident, blueprint, stmt)),
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
/// consistent with the rest of the expansion. Nothing else in a body
/// needs rewriting — see the module doc's scope note: generic-type
/// substitution never reaches into statements/expressions, only
/// declaration-level type positions.
fn rewrite_stmts(stmts: &[Stmt], renames: &HashMap<String, String>) -> Vec<Stmt> {
    stmts.iter().map(|s| rewrite_stmt(s, renames)).collect()
}

fn rewrite_stmt(stmt: &Stmt, renames: &HashMap<String, String>) -> Stmt {
    match stmt {
        Stmt::Let { name, value, span } => Stmt::Let {
            name: name.clone(),
            value: rewrite_expr(value, renames),
            span: *span,
        },
        Stmt::Expr(e) => Stmt::Expr(rewrite_expr(e, renames)),
        Stmt::Return { value, span } => Stmt::Return {
            value: value.as_ref().map(|v| rewrite_expr(v, renames)),
            span: *span,
        },
        Stmt::Fail { error, args, span } => Stmt::Fail {
            error: error.clone(),
            args: args.iter().map(|a| rewrite_expr(a, renames)).collect(),
            span: *span,
        },
        Stmt::Publish {
            stream,
            value,
            span,
        } => Stmt::Publish {
            stream: renamed_ident(stream, renames),
            value: rewrite_expr(value, renames),
            span: *span,
        },
    }
}

fn rewrite_expr(expr: &Expr, renames: &HashMap<String, String>) -> Expr {
    match expr {
        Expr::Ident(_) | Expr::Number { .. } | Expr::Str { .. } | Expr::Bool { .. } => expr.clone(),
        Expr::FieldAccess { base, field, span } => Expr::FieldAccess {
            base: Box::new(rewrite_expr(base, renames)),
            field: field.clone(),
            span: *span,
        },
        Expr::Index { base, index, span } => Expr::Index {
            base: Box::new(rewrite_expr(base, renames)),
            index: Box::new(rewrite_expr(index, renames)),
            span: *span,
        },
        Expr::Call { callee, args, span } => Expr::Call {
            callee: Box::new(rewrite_expr(callee, renames)),
            args: args.iter().map(|a| rewrite_expr(a, renames)).collect(),
            span: *span,
        },
        Expr::RecordCons { base, fields, span } => Expr::RecordCons {
            base: Box::new(rewrite_expr(base, renames)),
            fields: fields
                .iter()
                .map(|f| FieldInit {
                    name: f.name.clone(),
                    value: rewrite_expr(&f.value, renames),
                    span: f.span,
                })
                .collect(),
            span: *span,
        },
        Expr::Binary { op, lhs, rhs, span } => Expr::Binary {
            op: *op,
            lhs: Box::new(rewrite_expr(lhs, renames)),
            rhs: Box::new(rewrite_expr(rhs, renames)),
            span: *span,
        },
        Expr::Unary {
            op,
            expr: inner,
            span,
        } => Expr::Unary {
            op: *op,
            expr: Box::new(rewrite_expr(inner, renames)),
            span: *span,
        },
        Expr::If {
            cond,
            then_branch,
            else_branch,
            span,
        } => Expr::If {
            cond: Box::new(rewrite_expr(cond, renames)),
            then_branch: rewrite_stmts(then_branch, renames),
            else_branch: else_branch.as_ref().map(|b| rewrite_stmts(b, renames)),
            span: *span,
        },
        Expr::Match {
            scrutinee,
            arms,
            span,
        } => Expr::Match {
            scrutinee: Box::new(rewrite_expr(scrutinee, renames)),
            arms: arms
                .iter()
                .map(|arm| ExprArm {
                    label: arm.label.clone(),
                    body: rewrite_stmts(&arm.body, renames),
                    span: arm.span,
                })
                .collect(),
            span: *span,
        },
    }
}
