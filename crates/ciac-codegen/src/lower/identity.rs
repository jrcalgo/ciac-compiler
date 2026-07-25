//! The `HostSyntax` contract's own reference implementation
//! (`22UpdatePlan.md` Pillar 3, Part 3 — "a test-only 'identity'
//! `HostSyntax` (emitting s-expression-ish pseudo-code) demonstrates
//! ... that a new language implements leaf methods against a frozen
//! contract, and that identity backend's output is itself
//! snapshot-tested so the *contract* has goldens, not just its
//! consumers"). Two structs, `IdentitySyntax` (`Orientation::Expression`)
//! and `IdentitySyntaxStatement` (`Orientation::Statement`), proving
//! the contract from *both* modes against the same real HIR — see
//! `tests/tests/host_syntax_identity.rs`.
//!
//! **Not a real target**: neither struct is registered in
//! `crates/ciac/src/commands.rs::backends()`, and neither implements
//! [`crate::Backend`] at all — the exact same "real crate, always
//! compiled, never reachable through the CLI" discipline
//! `backends/skeleton-internal` already established. Their only job
//! is to render every HIR shape both bundled backends' leaves cover
//! into legible, deterministic s-expression text.

use super::host_syntax::{IndexKey, LoweredPredicate, MatchArm, Orientation, PredValue};
use super::{apply_dest, Dest, HostSyntax, Wrap};
use ciac_ir::{BinOp, HirType, NormalizedIr, PredOp, RecordId, TableId, UnOp, Verb};

fn binop_symbol(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Eq => "==",
        BinOp::NotEq => "!=",
        BinOp::Lt => "<",
        BinOp::LtEq => "<=",
        BinOp::Gt => ">",
        BinOp::GtEq => ">=",
        BinOp::And => "and",
        BinOp::Or => "or",
    }
}

fn unop_symbol(op: UnOp) -> &'static str {
    match op {
        UnOp::Neg => "neg",
        UnOp::Not => "not",
    }
}

fn predop_symbol(op: PredOp) -> &'static str {
    match op {
        PredOp::Eq => "=",
        PredOp::NotEq => "!=",
        PredOp::Lt => "<",
        PredOp::LtEq => "<=",
        PredOp::Gt => ">",
        PredOp::GtEq => ">=",
        PredOp::Contains => "contains",
    }
}

fn verb_symbol(verb: Verb) -> &'static str {
    match verb {
        Verb::DbInsert(_) => "db.insert",
        Verb::DbGet(_) => "db.get",
        Verb::DbUpdate(_) => "db.update",
        Verb::DbDelete(_) => "db.delete",
        Verb::DbQuery(_) => "db.query",
        Verb::DbCount(_) => "db.count",
        Verb::DbDeleteWhere(_) => "db.delete_where",
        Verb::CacheGet => "cache.get",
        Verb::CacheSet => "cache.set",
        Verb::CacheDelete => "cache.delete",
        Verb::ObjectStorePut => "object_store.put",
        Verb::ObjectStoreGet => "object_store.get",
        Verb::ObjectStoreDelete => "object_store.delete",
        Verb::ObjectStoreList => "object_store.list",
        Verb::EmailSend => "email.send",
        Verb::SearchIndex => "search.index",
        Verb::SearchQuery => "search.query",
        Verb::HttpCall => "http.call",
    }
}

fn predicate_sexpr(predicate: Option<&LoweredPredicate>) -> String {
    let Some(predicate) = predicate else {
        return "(no-predicate)".to_owned();
    };
    let terms: Vec<String> = predicate
        .terms
        .iter()
        .map(|term| {
            let value = match &term.value {
                PredValue::Rendered(s) => s.clone(),
                PredValue::EnumVariant(v) => format!("{v:?}"),
                PredValue::BoolLit(b) => b.to_string(),
            };
            format!("({} {} {value})", term.field, predop_symbol(term.op))
        })
        .collect();
    format!("(where {})", terms.join(" "))
}

fn record_cons_sexpr(record_name: &str, fields: &[(String, String)], base: Option<&str>) -> String {
    let field_strs: Vec<String> = fields
        .iter()
        .map(|(name, value)| format!("({name} {value})"))
        .collect();
    match base {
        None => format!("(record-cons {record_name} {})", field_strs.join(" ")),
        Some(base) => format!(
            "(record-update {record_name} {base} {})",
            field_strs.join(" ")
        ),
    }
}

fn fail_sexpr(ir: &NormalizedIr, error: RecordId, args: &[String]) -> String {
    let name = &ir.record(error).name;
    format!("(fail {name} {})", args.join(" "))
}

fn publish_sexpr(subject: &str, value: &str) -> String {
    format!("(publish {subject:?} {value})")
}

fn db_get_sexpr(ir: &NormalizedIr, table: TableId, key: &str) -> String {
    format!("(db-get {} {key})", ir.table(table).name)
}

/// The `Orientation::Expression` half of the contract's own reference
/// implementation.
#[derive(Debug)]
pub struct IdentitySyntax<'a> {
    pub ir: &'a NormalizedIr,
}

impl<'a> IdentitySyntax<'a> {
    pub fn new(ir: &'a NormalizedIr) -> Self {
        Self { ir }
    }
}

impl HostSyntax for IdentitySyntax<'_> {
    const ORIENTATION: Orientation = Orientation::Expression;

    fn int_lit(&self, n: i64) -> String {
        n.to_string()
    }
    fn float_lit(&self, f: f64) -> String {
        format!("{f}")
    }
    fn str_lit(&self, s: &str) -> String {
        format!("{s:?}")
    }
    fn bool_lit(&self, b: bool) -> String {
        b.to_string()
    }
    fn field_access(&self, base: &str, field: &str) -> String {
        format!("(field-access {base} {field})")
    }
    fn index(&self, base: &str, key: IndexKey<'_>) -> String {
        match key {
            IndexKey::StrKey(s) => format!("(index {base} {s:?})"),
            IndexKey::Expr(e) => format!("(index {base} {e})"),
        }
    }
    fn uuid_new(&self) -> String {
        "(uuid-new)".to_owned()
    }
    fn timestamp_now(&self) -> String {
        "(timestamp-now)".to_owned()
    }
    fn enum_literal(&self, enum_name: Option<&str>, variant: &str) -> String {
        match enum_name {
            Some(name) => format!("(enum {name} {variant})"),
            None => format!("(enum-lit {variant})"),
        }
    }
    fn record_cons(
        &self,
        record_name: &str,
        fields: &[(String, String)],
        base: Option<&str>,
    ) -> String {
        record_cons_sexpr(record_name, fields, base)
    }
    fn binary(
        &self,
        op: BinOp,
        lhs: &str,
        rhs: &str,
        _lhs_ty: &HirType,
        _rhs_ty: &HirType,
    ) -> String {
        format!("({} {lhs} {rhs})", binop_symbol(op))
    }
    fn unary(&self, op: UnOp, operand: &str) -> String {
        format!("({} {operand})", unop_symbol(op))
    }

    fn if_expr(&self, cond: &str, then_block: &str, else_block: &str) -> String {
        format!("(if {cond} (then {then_block}) (else {else_block}))")
    }
    fn match_expr(&self, enum_name: &str, scrutinee: &str, arms: &[MatchArm]) -> String {
        let arm_strs: Vec<String> = arms
            .iter()
            .map(|arm| {
                let pattern = arm.variant.as_deref().unwrap_or("_");
                format!("({enum_name}::{pattern} {})", arm.body)
            })
            .collect();
        format!("(match {scrutinee} {})", arm_strs.join(" "))
    }
    fn db_insert_expr(&self, table: TableId, value: &str, in_tx: bool) -> String {
        let op = if in_tx { "db-insert-tx" } else { "db-insert" };
        format!("({op} {} {value})", self.ir.table(table).name)
    }
    fn db_update_expr(&self, table: TableId, key: &str, value: &str, in_tx: bool) -> String {
        let op = if in_tx { "db-update-tx" } else { "db-update" };
        format!("({op} {} {key} {value})", self.ir.table(table).name)
    }
    fn db_delete_expr(&self, table: TableId, key: &str, in_tx: bool) -> String {
        let op = if in_tx { "db-delete-tx" } else { "db-delete" };
        format!("({op} {} {key})", self.ir.table(table).name)
    }
    fn query_expr(&self, verb: Verb, predicate: Option<&LoweredPredicate>, in_tx: bool) -> String {
        let suffix = if in_tx { "-tx" } else { "" };
        format!(
            "({}{suffix} {})",
            verb_symbol(verb),
            predicate_sexpr(predicate)
        )
    }
    fn let_binding(&self, name: &str, value: &str) -> String {
        format!("(let {name} {value})")
    }
    fn wrap_tail(&self, value: &str, wrap: Wrap) -> String {
        match wrap {
            Wrap::None => format!("(expr-stmt {value})"),
            Wrap::Plain => value.to_owned(),
            Wrap::Wrapped => format!("(ok {value})"),
        }
    }
    fn unit_literal(&self) -> String {
        "(unit)".to_owned()
    }
    fn transaction_expr(&self, world_branch: &str, real_branch: &str) -> String {
        format!("(transaction (sim {world_branch}) (real {real_branch}))")
    }

    fn return_stmt(&self, value: Option<&str>, _indent: &str) -> String {
        match value {
            Some(v) => format!("(return {v})"),
            None => "(return)".to_owned(),
        }
    }
    fn fail(&self, error: RecordId, args: &[String], _indent: &str) -> String {
        fail_sexpr(self.ir, error, args)
    }
    fn publish(&self, subject: &str, value: &str, _value_ty: &HirType, _indent: &str) -> String {
        publish_sexpr(subject, value)
    }
    fn db_get(&self, table: TableId, key: &str) -> String {
        db_get_sexpr(self.ir, table, key)
    }
    fn cache_get(&self, key: &str) -> String {
        format!("(cache-get {key})")
    }
    fn cache_set(&self, key: &str, value: &str, _value_ty: &HirType) -> String {
        format!("(cache-set {key} {value})")
    }
    fn cache_delete(&self, key: &str) -> String {
        format!("(cache-delete {key})")
    }
    fn object_store_put(&self, key: &str, value: &str, _value_ty: &HirType) -> String {
        format!("(object-store-put {key} {value})")
    }
    fn object_store_get(&self, key: &str) -> String {
        format!("(object-store-get {key})")
    }
    fn object_store_delete(&self, key: &str) -> String {
        format!("(object-store-delete {key})")
    }
    fn object_store_list(&self, prefix: &str) -> String {
        format!("(object-store-list {prefix})")
    }
    fn email_send(&self, to: &str, subject: &str, body: &str) -> String {
        format!("(email-send {to} {subject} {body})")
    }
    fn search_index(&self, doc_id: &str, document: &str, _document_ty: &HirType) -> String {
        format!("(search-index {doc_id} {document})")
    }
    fn search_query(&self, query: &str) -> String {
        format!("(search-query {query})")
    }
    fn http_call(&self, url: &str, json_body: &str, _body_ty: &HirType) -> String {
        format!("(http-call {url} {json_body})")
    }
}

/// The `Orientation::Statement` half of the contract's own reference
/// implementation — the same leaf surface as [`IdentitySyntax`] except
/// for the control-flow/db-write/dest-application leaves, proving the
/// contract is orientation-agnostic-capable: a hypothetical future
/// host could pick either mode for the same body shape.
#[derive(Debug)]
pub struct IdentitySyntaxStatement<'a> {
    pub ir: &'a NormalizedIr,
}

impl<'a> IdentitySyntaxStatement<'a> {
    pub fn new(ir: &'a NormalizedIr) -> Self {
        Self { ir }
    }
}

impl HostSyntax for IdentitySyntaxStatement<'_> {
    const ORIENTATION: Orientation = Orientation::Statement;

    fn int_lit(&self, n: i64) -> String {
        n.to_string()
    }
    fn float_lit(&self, f: f64) -> String {
        format!("{f}")
    }
    fn str_lit(&self, s: &str) -> String {
        format!("{s:?}")
    }
    fn bool_lit(&self, b: bool) -> String {
        b.to_string()
    }
    fn field_access(&self, base: &str, field: &str) -> String {
        format!("(field-access {base} {field})")
    }
    fn index(&self, base: &str, key: IndexKey<'_>) -> String {
        match key {
            IndexKey::StrKey(s) => format!("(index {base} {s:?})"),
            IndexKey::Expr(e) => format!("(index {base} {e})"),
        }
    }
    fn uuid_new(&self) -> String {
        "(uuid-new)".to_owned()
    }
    fn timestamp_now(&self) -> String {
        "(timestamp-now)".to_owned()
    }
    fn enum_literal(&self, enum_name: Option<&str>, variant: &str) -> String {
        match enum_name {
            Some(name) => format!("(enum {name} {variant})"),
            None => format!("(enum-lit {variant})"),
        }
    }
    fn record_cons(
        &self,
        record_name: &str,
        fields: &[(String, String)],
        base: Option<&str>,
    ) -> String {
        record_cons_sexpr(record_name, fields, base)
    }
    fn binary(
        &self,
        op: BinOp,
        lhs: &str,
        rhs: &str,
        _lhs_ty: &HirType,
        _rhs_ty: &HirType,
    ) -> String {
        format!("({} {lhs} {rhs})", binop_symbol(op))
    }
    fn unary(&self, op: UnOp, operand: &str) -> String {
        format!("({} {operand})", unop_symbol(op))
    }

    fn if_tail(
        &self,
        cond: &str,
        then_lines: Vec<String>,
        else_lines: Vec<String>,
        indent: &str,
    ) -> Vec<String> {
        let mut out = vec![format!("{indent}(if {cond}")];
        out.extend(then_lines);
        out.push(format!("{indent} else"));
        out.extend(else_lines);
        out.push(format!("{indent})"));
        out
    }
    fn match_tail(
        &self,
        scrutinee: &str,
        arms: &[(Option<String>, Vec<String>)],
        indent: &str,
    ) -> Vec<String> {
        let mut out = vec![format!("{indent}(match {scrutinee}")];
        for (variant, lines) in arms {
            let pattern = variant.as_deref().unwrap_or("_");
            out.push(format!("{indent} ({pattern}"));
            out.extend(lines.iter().cloned());
            out.push(format!("{indent} )"));
        }
        out.push(format!("{indent})"));
        out
    }
    fn db_insert_tail(
        &self,
        table: TableId,
        value: &str,
        dest: &Dest,
        indent: &str,
        _in_tx: bool,
    ) -> Vec<String> {
        let mut out = vec![format!(
            "{indent}(db-insert! {} {value})",
            self.ir.table(table).name
        )];
        apply_dest(self, dest, value, indent, &mut out);
        out
    }
    fn db_update_tail(
        &self,
        table: TableId,
        key: &str,
        value: &str,
        dest: &Dest,
        indent: &str,
        _in_tx: bool,
    ) -> Vec<String> {
        let mut out = vec![format!(
            "{indent}(db-update! {} {key} {value})",
            self.ir.table(table).name
        )];
        apply_dest(self, dest, value, indent, &mut out);
        out
    }
    fn db_delete_tail(
        &self,
        table: TableId,
        key: &str,
        dest: &Dest,
        indent: &str,
        _in_tx: bool,
    ) -> Vec<String> {
        let mut out = vec![format!(
            "{indent}(db-delete! {} {key})",
            self.ir.table(table).name
        )];
        apply_dest(self, dest, "true", indent, &mut out);
        out
    }
    fn query_tail(
        &self,
        verb: Verb,
        predicate: Option<&LoweredPredicate>,
        dest: &Dest,
        indent: &str,
        _in_tx: bool,
    ) -> Vec<String> {
        let value = format!("({} {})", verb_symbol(verb), predicate_sexpr(predicate));
        let mut out = Vec::new();
        apply_dest(self, dest, &value, indent, &mut out);
        out
    }
    fn assign(&self, name: &str, value: &str, indent: &str) -> String {
        format!("{indent}(assign {name} {value})")
    }
    fn discard_stmt(&self, value: &str, indent: &str) -> String {
        format!("{indent}(discard {value})")
    }
    fn empty_block_stmt(&self, indent: &str) -> Vec<String> {
        vec![format!("{indent}(empty-block)")]
    }
    fn transaction_stmt(&self, inner_lines: Vec<String>, indent: &str) -> Vec<String> {
        let mut out = vec![format!("{indent}(transaction")];
        out.extend(inner_lines);
        out.push(format!("{indent})"));
        out
    }

    fn return_stmt(&self, value: Option<&str>, indent: &str) -> String {
        match value {
            Some(v) => format!("{indent}(return {v})"),
            None => format!("{indent}(return)"),
        }
    }
    fn fail(&self, error: RecordId, args: &[String], indent: &str) -> String {
        format!("{indent}{}", fail_sexpr(self.ir, error, args))
    }
    fn publish(&self, subject: &str, value: &str, _value_ty: &HirType, indent: &str) -> String {
        format!("{indent}{}", publish_sexpr(subject, value))
    }
    fn db_get(&self, table: TableId, key: &str) -> String {
        db_get_sexpr(self.ir, table, key)
    }
    fn cache_get(&self, key: &str) -> String {
        format!("(cache-get {key})")
    }
    fn cache_set(&self, key: &str, value: &str, _value_ty: &HirType) -> String {
        format!("(cache-set {key} {value})")
    }
    fn cache_delete(&self, key: &str) -> String {
        format!("(cache-delete {key})")
    }
    fn object_store_put(&self, key: &str, value: &str, _value_ty: &HirType) -> String {
        format!("(object-store-put {key} {value})")
    }
    fn object_store_get(&self, key: &str) -> String {
        format!("(object-store-get {key})")
    }
    fn object_store_delete(&self, key: &str) -> String {
        format!("(object-store-delete {key})")
    }
    fn object_store_list(&self, prefix: &str) -> String {
        format!("(object-store-list {prefix})")
    }
    fn email_send(&self, to: &str, subject: &str, body: &str) -> String {
        format!("(email-send {to} {subject} {body})")
    }
    fn search_index(&self, doc_id: &str, document: &str, _document_ty: &HirType) -> String {
        format!("(search-index {doc_id} {document})")
    }
    fn search_query(&self, query: &str) -> String {
        format!("(search-query {query})")
    }
    fn http_call(&self, url: &str, json_body: &str, _body_ty: &HirType) -> String {
        format!("(http-call {url} {json_body})")
    }
}
