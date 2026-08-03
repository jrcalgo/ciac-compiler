//! Direct lowering of the typed HIR (`ciac_ir::hir`) into Python source.
//!
//! The walker (block/tail shaping, precedence, enum-literal use-site
//! recovery, float-literal fidelity, divergence truncation) lives in
//! `ciac_codegen::lower` (`22UpdatePlan.md` Pillar 3, Parts 2-3);
//! [`PySyntax`] supplies only the leaf constructors genuinely specific
//! to this target — ORM calls (not raw SQL), and the `Sink`-shaped
//! statement decomposition Python needs because Python statements
//! aren't expressions (control flow and `db.insert`/`update`/`delete`/
//! `query`/`count`/`delete_where` don't fit a single Python
//! expression; they're lowered as a statement sequence applied to a
//! `Dest` — see [`ciac_codegen::lower::Dest`]/[`ciac_codegen::lower::lower_tail`]).
//!
//! [`render_test`] (the generated behavioral test) and its
//! `dummy_value`/`assert_result`/`collect_record_ids` helpers are
//! untouched by this split: they depend only on the shared `Needs`
//! scanner (Part 1), never on `HostSyntax`, since they build
//! mock-assertion scaffolding from booleans and counts rather than
//! rendering lowered code text.

use ciac_codegen::lower::scan;
use ciac_codegen::lower::{
    self, Dest, HostSyntax, IndexKey, LoweredPredicate, Orientation, PredValue,
};
use ciac_codegen::model as context;
use ciac_ir::{BinOp, HandlerBody, HirType, NormalizedIr, PredOp, RecordId, TableId, UnOp, Verb};
use heck::ToSnakeCase;
use serde::Serialize;

/// Python type annotation for a HIR type — a handler *signature*
/// concern (param/return types), not part of the `HostSyntax` body
/// contract. Record types need `from app.schemas import <name>` at the
/// call site — see [`ciac_codegen::lower::Needs`].
pub fn py_type(ir: &NormalizedIr, ty: &HirType) -> String {
    match ty {
        HirType::Str | HirType::Uuid => "str".to_owned(),
        HirType::Int => "int".to_owned(),
        HirType::Float => "float".to_owned(),
        HirType::Bool => "bool".to_owned(),
        HirType::Timestamp => "datetime".to_owned(),
        HirType::Json => "dict[str, Any]".to_owned(),
        HirType::Enum { variants } => format!(
            "Literal[{}]",
            variants
                .iter()
                .map(|v| format!("{v:?}"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        HirType::Record(id) => ir.record(*id).name.clone(),
        HirType::Option(inner) => format!("{} | None", py_type(ir, inner)),
        HirType::List(inner) => format!("list[{}]", py_type(ir, inner)),
        HirType::Unit | HirType::Never => "None".to_owned(),
    }
}

fn model_class_name(ir: &NormalizedIr, table: TableId) -> String {
    ir.table(table).name.clone()
}

fn record_class_name(ir: &NormalizedIr, record: RecordId) -> String {
    ir.record(record).name.clone()
}

/// OpenSearch has no per-document collection concept in the language
/// yet (v0.14 M3/M4) — every `search.index`/`search.query` call on a
/// given instance shares this one index name.
const SEARCH_INDEX_NAME: &str = "documents";

/// The `Orientation::Statement` `HostSyntax` implementation for this
/// target: Python statements aren't expressions, so control flow and
/// the statement-shaped db verbs decompose into a line sequence
/// applied to a [`Dest`] — the shared dispatcher's `lower_tail`, not
/// this type, owns that decomposition; this type supplies only the
/// ORM-call spelling.
struct PySyntax<'a> {
    ir: &'a NormalizedIr,
}

impl PySyntax<'_> {
    /// A Python *value* (not a JSON string — for `search.index`'s
    /// `document` and `external_http.request`'s `json=` body, both of
    /// which take a real dict) for `value`, which `search.index`/
    /// `external_http.request` accept as any type per their closed
    /// verb signature (mirroring `cache.set`/`object_store.put`'s
    /// untyped payload): records become their field dict, `Json`
    /// values pass through as-is, and anything else (a bare
    /// string/number/bool) is wrapped so the body is always a JSON
    /// object.
    fn json_body(&self, value: &str, value_ty: &HirType) -> String {
        match value_ty {
            HirType::Record(_) => format!("{value}.model_dump(mode=\"json\")"),
            HirType::Json => value.to_owned(),
            _ => format!("{{\"value\": {value}}}"),
        }
    }

    /// Renders the `.where(..)` chain for a `db.query`/`count`/
    /// `delete_where` predicate (v0.14 M2) — one
    /// `.where(Model.field <op> value)` per term, SQLAlchemy ANDs
    /// chained `.where()` calls together. Empty string (no filtering)
    /// when there's no `where` clause.
    fn where_chain(&self, model: &str, predicate: Option<&LoweredPredicate>) -> String {
        let Some(predicate) = predicate else {
            return String::new();
        };
        predicate
            .terms
            .iter()
            .map(|term| {
                let field = &term.field;
                // `field == True`/`field == False` is `ruff`'s E712: a
                // bare (or negated) column already reads as a boolean
                // filter in SQLAlchemy, same as Python's own
                // preference for `if x:` over `if x == True:`.
                match (term.op, &term.value) {
                    (PredOp::Eq, PredValue::BoolLit(true))
                    | (PredOp::NotEq, PredValue::BoolLit(false)) => {
                        format!(".where({model}.{field})")
                    }
                    (PredOp::Eq, PredValue::BoolLit(false))
                    | (PredOp::NotEq, PredValue::BoolLit(true)) => {
                        format!(".where(~{model}.{field})")
                    }
                    (op, value) => {
                        let value_s = match value {
                            PredValue::EnumVariant(v) => format!("{v:?}"),
                            PredValue::BoolLit(b) => if *b { "True" } else { "False" }.to_owned(),
                            PredValue::Rendered(s) => s.clone(),
                        };
                        match op {
                            PredOp::Eq => format!(".where({model}.{field} == {value_s})"),
                            PredOp::NotEq => format!(".where({model}.{field} != {value_s})"),
                            PredOp::Lt => format!(".where({model}.{field} < {value_s})"),
                            PredOp::LtEq => format!(".where({model}.{field} <= {value_s})"),
                            PredOp::Gt => format!(".where({model}.{field} > {value_s})"),
                            PredOp::GtEq => format!(".where({model}.{field} >= {value_s})"),
                            PredOp::Contains => {
                                format!(".where({model}.{field}.contains({value_s}))")
                            }
                        }
                    }
                }
            })
            .collect()
    }
}

impl HostSyntax for PySyntax<'_> {
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
        if b { "True" } else { "False" }.to_owned()
    }
    fn field_access(&self, base: &str, field: &str) -> String {
        format!("{base}.{field}")
    }
    fn index(&self, base: &str, key: IndexKey<'_>) -> String {
        let k = match key {
            IndexKey::StrKey(s) => format!("{s:?}"),
            IndexKey::Expr(e) => e,
        };
        format!("{base}[{k}]")
    }
    fn uuid_new(&self) -> String {
        "str(uuid4())".to_owned()
    }
    fn timestamp_now(&self) -> String {
        "datetime.now(timezone.utc)".to_owned()
    }
    fn enum_literal(&self, _enum_name: Option<&str>, variant: &str) -> String {
        format!("{variant:?}")
    }
    fn record_cons(
        &self,
        record_name: &str,
        fields: &[(String, String)],
        base: Option<&str>,
    ) -> String {
        match base {
            None => {
                let args = fields
                    .iter()
                    .map(|(name, value)| format!("{name}={value}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{record_name}({args})")
            }
            Some(base) => {
                let updates = fields
                    .iter()
                    .map(|(name, value)| format!("{name:?}: {value}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{base}.model_copy(update={{{updates}}})")
            }
        }
    }
    fn binary(
        &self,
        op: BinOp,
        lhs: &str,
        rhs: &str,
        lhs_ty: &HirType,
        rhs_ty: &HirType,
    ) -> String {
        if op == BinOp::Add {
            return if *lhs_ty == HirType::Str && *rhs_ty != HirType::Str {
                format!("({lhs} + str({rhs}))")
            } else if *rhs_ty == HirType::Str && *lhs_ty != HirType::Str {
                format!("(str({lhs}) + {rhs})")
            } else {
                format!("({lhs} + {rhs})")
            };
        }
        let op_s = match op {
            BinOp::Add => unreachable!("handled above"),
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
        };
        format!("({lhs} {op_s} {rhs})")
    }
    fn unary(&self, op: UnOp, operand: &str) -> String {
        match op {
            UnOp::Neg => format!("(-{operand})"),
            UnOp::Not => format!("(not {operand})"),
        }
    }

    fn if_tail(
        &self,
        cond: &str,
        then_lines: Vec<String>,
        else_lines: Vec<String>,
        indent: &str,
    ) -> Vec<String> {
        let mut out = vec![format!("{indent}if {cond}:")];
        out.extend(then_lines);
        out.push(format!("{indent}else:"));
        out.extend(else_lines);
        out
    }
    fn match_tail(
        &self,
        scrutinee: &str,
        arms: &[(Option<String>, Vec<String>)],
        indent: &str,
    ) -> Vec<String> {
        let mut out = Vec::new();
        for (i, (variant, lines)) in arms.iter().enumerate() {
            let cond = match variant {
                Some(v) => format!("{scrutinee} == {v:?}"),
                None => "True".to_owned(),
            };
            let kw = if i == 0 { "if" } else { "elif" };
            out.push(format!("{indent}{kw} {cond}:"));
            out.extend(lines.iter().cloned());
        }
        out
    }
    fn db_insert_tail(
        &self,
        table: TableId,
        value: &str,
        dest: &Dest,
        indent: &str,
        in_tx: bool,
    ) -> Vec<String> {
        let model = model_class_name(self.ir, table);
        let mut out = vec![format!(
            "{indent}self.session.add({model}(**{value}.model_dump()))"
        )];
        if !in_tx {
            out.push(format!("{indent}await self.session.commit()"));
        }
        match dest {
            Dest::Assign(name) => out.push(format!("{indent}{name} = {value}")),
            Dest::Return => out.push(format!("{indent}return {value}")),
            Dest::Discard => {}
        }
        out
    }
    fn db_update_tail(
        &self,
        table: TableId,
        key: &str,
        value: &str,
        dest: &Dest,
        indent: &str,
        in_tx: bool,
    ) -> Vec<String> {
        let model = model_class_name(self.ir, table);
        let record = record_class_name(self.ir, self.ir.table(table).record);
        let mut out = vec![
            format!("{indent}_row = await self.session.get({model}, str({key}))"),
            format!("{indent}if _row is not None:"),
            format!("{indent}    for _k, _v in {value}.model_dump().items():"),
            format!("{indent}        setattr(_row, _k, _v)"),
        ];
        if !in_tx {
            out.push(format!("{indent}    await self.session.commit()"));
        }
        lower::apply_dest(
            self,
            dest,
            &format!("{record}.model_validate(_row, from_attributes=True)"),
            &format!("{indent}    "),
            &mut out,
        );
        out.push(format!("{indent}else:"));
        lower::apply_dest(self, dest, "None", &format!("{indent}    "), &mut out);
        out
    }
    fn db_delete_tail(
        &self,
        table: TableId,
        key: &str,
        dest: &Dest,
        indent: &str,
        in_tx: bool,
    ) -> Vec<String> {
        let model = model_class_name(self.ir, table);
        let mut out = vec![
            format!("{indent}_row = await self.session.get({model}, str({key}))"),
            format!("{indent}if _row is not None:"),
            format!("{indent}    await self.session.delete(_row)"),
        ];
        if !in_tx {
            out.push(format!("{indent}    await self.session.commit()"));
        }
        lower::apply_dest(self, dest, "True", &format!("{indent}    "), &mut out);
        out.push(format!("{indent}else:"));
        lower::apply_dest(self, dest, "False", &format!("{indent}    "), &mut out);
        out
    }
    fn query_tail(
        &self,
        verb: Verb,
        predicate: Option<&LoweredPredicate>,
        dest: &Dest,
        indent: &str,
        in_tx: bool,
    ) -> Vec<String> {
        match verb {
            Verb::DbQuery(table) => {
                let model = model_class_name(self.ir, table);
                let record = record_class_name(self.ir, self.ir.table(table).record);
                let where_chain = self.where_chain(&model, predicate);
                let mut out = vec![
                    format!("{indent}_stmt = select({model}){where_chain}"),
                    format!("{indent}_rows = (await self.session.execute(_stmt)).scalars().all()"),
                ];
                lower::apply_dest(
                    self,
                    dest,
                    &format!("[{record}.model_validate(_r, from_attributes=True) for _r in _rows]"),
                    indent,
                    &mut out,
                );
                out
            }
            Verb::DbCount(table) => {
                let model = model_class_name(self.ir, table);
                let where_chain = self.where_chain(&model, predicate);
                let mut out = vec![format!(
                    "{indent}_stmt = select(func.count()).select_from({model}){where_chain}"
                )];
                lower::apply_dest(
                    self,
                    dest,
                    "(await self.session.execute(_stmt)).scalar_one()",
                    indent,
                    &mut out,
                );
                out
            }
            Verb::DbDeleteWhere(table) => {
                let model = model_class_name(self.ir, table);
                let where_chain = self.where_chain(&model, predicate);
                let mut out = vec![
                    format!("{indent}_stmt = sql_delete({model}){where_chain}"),
                    format!("{indent}_result = await self.session.execute(_stmt)"),
                ];
                if !in_tx {
                    out.push(format!("{indent}await self.session.commit()"));
                }
                lower::apply_dest(self, dest, "_result.rowcount", indent, &mut out);
                out
            }
            _ => unreachable!("HirExpr::Query only ever carries a db query verb, found {verb:?}"),
        }
    }
    fn assign(&self, name: &str, value: &str, indent: &str) -> String {
        format!("{indent}{name} = {value}")
    }
    fn discard_stmt(&self, value: &str, indent: &str) -> String {
        format!("{indent}{value}")
    }
    fn empty_block_stmt(&self, indent: &str) -> Vec<String> {
        vec![format!("{indent}pass")]
    }
    // v0.16 M5: every `db.*` verb underneath (see the `db_*_tail`/
    // `query_tail` leaves above) skips its own commit when `in_tx` is
    // set; `try`/`except` — not a nested `session.begin()` — commits
    // once on success and rolls back on any exception (a `fail`'s
    // `raise` included), because `AsyncSession` autobegins its own
    // transaction on first use, and an *explicit* `begin()` on a
    // session that already has one active raises
    // `InvalidRequestError`.
    fn transaction_stmt(&self, inner_lines: Vec<String>, indent: &str) -> Vec<String> {
        let mut out = vec![format!("{indent}try:")];
        out.extend(inner_lines);
        out.push(format!("{indent}    await self.session.commit()"));
        out.push(format!("{indent}except Exception:"));
        out.push(format!("{indent}    await self.session.rollback()"));
        out.push(format!("{indent}    raise"));
        out
    }

    fn return_stmt(&self, value: Option<&str>, indent: &str) -> String {
        match value {
            Some(v) => format!("{indent}return {v}"),
            None => format!("{indent}return"),
        }
    }
    fn fail(&self, error: RecordId, args: &[String], indent: &str) -> String {
        let exc = record_class_name(self.ir, error);
        format!("{indent}raise {exc}({})", args.join(", "))
    }
    fn publish(&self, subject: &str, value: &str, value_ty: &HirType, indent: &str) -> String {
        let payload = if matches!(value_ty, HirType::Record(_)) {
            format!("{value}.model_dump(mode=\"json\")")
        } else {
            value.to_owned()
        };
        format!("{indent}await publish({subject:?}, {payload})")
    }
    fn db_get(&self, table: TableId, key: &str) -> String {
        let model = model_class_name(self.ir, table);
        let record = record_class_name(self.ir, self.ir.table(table).record);
        format!(
            "({record}.model_validate(_row, from_attributes=True) \
             if (_row := await self.session.get({model}, str({key}))) is not None else None)"
        )
    }
    fn cache_get(&self, key: &str) -> String {
        format!("(json.loads(_cv) if (_cv := await self.cache.get({key})) is not None else None)")
    }
    fn cache_set(&self, key: &str, value: &str, value_ty: &HirType) -> String {
        let encoded = if matches!(value_ty, HirType::Record(_)) {
            format!("{value}.model_dump_json()")
        } else {
            format!("json.dumps({value})")
        };
        format!("(await self.cache.set({key}, {encoded}))")
    }
    fn cache_delete(&self, key: &str) -> String {
        format!("(await self.cache.delete({key}))")
    }
    fn object_store_put(&self, key: &str, value: &str, value_ty: &HirType) -> String {
        let payload = if matches!(value_ty, HirType::Record(_)) {
            format!("{value}.model_dump_json().encode()")
        } else {
            format!("str({value}).encode()")
        };
        format!("(await self.object_store.put({key}, {payload}))")
    }
    fn object_store_get(&self, key: &str) -> String {
        format!("json.loads(await self.object_store.get({key}))")
    }
    fn object_store_delete(&self, key: &str) -> String {
        format!("(await self.object_store.delete({key}))")
    }
    fn object_store_list(&self, prefix: &str) -> String {
        format!("(await self.object_store.list({prefix}))")
    }
    fn email_send(&self, to: &str, subject: &str, body: &str) -> String {
        format!("(await self.email.send({to}, {subject}, {body}))")
    }
    fn search_index(&self, doc_id: &str, document: &str, document_ty: &HirType) -> String {
        let document = self.json_body(document, document_ty);
        format!("(await self.search.index({SEARCH_INDEX_NAME:?}, {doc_id}, {document}))")
    }
    fn search_query(&self, query: &str) -> String {
        format!(
            "(await self.search.search({SEARCH_INDEX_NAME:?}, {{\"query\": {{\"query_string\": {{\"query\": {query}}}}}}}))"
        )
    }
    fn http_call(&self, url: &str, json_body: &str, body_ty: &HirType) -> String {
        let json_arg = self.json_body(json_body, body_ty);
        format!("(await self.http.post({url}, json={json_arg})).json()")
    }
}

#[derive(Debug, Serialize)]
pub struct ParamCtx {
    pub name: String,
    pub ty: String,
}

/// Everything `logic.py.j2` needs to render one typed handler's file —
/// inline (compiler-owned, `app/logic/<module>.py`) or `extern` (seeded,
/// `app/services/<module>.py`, `body` is just a `NotImplementedError`).
#[derive(Debug, Serialize)]
pub struct LogicFileCtx {
    pub class_name: String,
    pub module: String,
    pub is_extern: bool,
    pub params: Vec<ParamCtx>,
    pub return_type: String,
    pub needs_db: bool,
    pub needs_cache: bool,
    pub needs_queue: bool,
    pub needs_json: bool,
    pub needs_uuid: bool,
    pub needs_datetime: bool,
    /// `select`/`func`/`delete as sql_delete`, filtered to what this
    /// handler actually uses — see `Needs::sa_select`/`sa_func`/
    /// `sa_delete`.
    pub sa_query_imports: Vec<String>,
    pub extras: Vec<context::ExtraDepCtx>,
    pub schema_imports: Vec<String>,
    pub model_imports: Vec<String>,
    pub body: Vec<String>,
}

/// Builds the render context for one typed handler node. `name` is the
/// handler's declared name (`node.component.name()`).
pub fn render(ir: &NormalizedIr, name: &str, hir: &HandlerBody) -> LogicFileCtx {
    let needs = scan(ir, hir);
    let bindings = context::hir_bindings(ir, hir);
    let access = context::access_of(&bindings);
    let extras = context::extras_of(&bindings);

    let mut schema_imports: Vec<String> = needs
        .records
        .iter()
        .map(|id| record_class_name(ir, *id))
        .collect();
    schema_imports.sort();
    let mut model_imports: Vec<String> = needs
        .tables
        .iter()
        .map(|id| model_class_name(ir, *id))
        .collect();
    model_imports.sort();

    let params = hir
        .params
        .iter()
        .map(|(n, ty)| ParamCtx {
            name: n.clone(),
            ty: py_type(ir, ty),
        })
        .collect();

    let body = match &hir.body {
        Some(_) => {
            let syntax = PySyntax { ir };
            lower::lower_body_stmt(&syntax, ir, hir, "        ")
        }
        None => vec!["        raise NotImplementedError".to_owned()],
    };

    let mut sa_query_imports = Vec::new();
    if needs.sa_select {
        sa_query_imports.push("select".to_owned());
    }
    if needs.sa_func {
        sa_query_imports.push("func".to_owned());
    }
    if needs.sa_delete {
        sa_query_imports.push("delete as sql_delete".to_owned());
    }

    LogicFileCtx {
        class_name: name.to_owned(),
        module: name.to_snake_case(),
        is_extern: hir.body.is_none(),
        params,
        return_type: py_type(ir, &hir.return_ty),
        needs_db: access.db.is_some(),
        needs_cache: access.cache_expr.is_some(),
        needs_queue: needs.queue,
        needs_json: needs.json,
        needs_uuid: needs.uuid,
        needs_datetime: needs.datetime,
        sa_query_imports,
        extras,
        schema_imports,
        model_imports,
        body,
    }
}

/// Records the generated behavioral test actually spells by name — only
/// a bare `Record` (both `dummy_value` and `assert_result` reference the
/// class name directly). `Option<Record>`/`List<Record>` deliberately
/// don't recurse: their dummy values (`None`/`[]`) and assertions
/// (`assert_result` skips `Option`; the `List` case only checks
/// `isinstance(result, list)`) never spell the inner record's name, so
/// importing it would be an unused import (`ruff` F401).
fn collect_record_ids(ty: &HirType, out: &mut Vec<RecordId>) {
    if let HirType::Record(id) = ty {
        if !out.contains(id) {
            out.push(*id);
        }
    }
}

fn dummy_field(ty: &ciac_ir::FieldType) -> String {
    use ciac_ir::FieldType;
    match ty {
        FieldType::Str => "\"test\"".to_owned(),
        FieldType::Int => "0".to_owned(),
        FieldType::Float => "0.0".to_owned(),
        FieldType::Bool => "True".to_owned(),
        FieldType::Uuid => "str(uuid4())".to_owned(),
        FieldType::Timestamp => "datetime.now(timezone.utc)".to_owned(),
        FieldType::Json => "{}".to_owned(),
        FieldType::Enum { variants } => {
            format!("{:?}", variants.first().cloned().unwrap_or_default())
        }
        // v0.16 M4: a to-one reference's generated Pydantic attribute is
        // a plain id string (`ciac_codegen::model::FieldTypeKind::Reference`'s
        // doc comment), so a dummy `str(uuid4())` matches it exactly.
        FieldType::Reference {
            cardinality: ciac_ir::Cardinality::One,
            ..
        } => "str(uuid4())".to_owned(),
        FieldType::Reference {
            cardinality: ciac_ir::Cardinality::Many,
            ..
        } => unreachable!("many-relation codegen is gated until v0.16 M5/M6 land"),
    }
}

/// A throwaway Python literal of type `ty`, for the generated behavioral
/// test's payload — not a fixture generator, just enough to construct a
/// well-typed argument for `handle()`.
fn dummy_value(ir: &NormalizedIr, ty: &HirType) -> String {
    match ty {
        HirType::Str => "\"test\"".to_owned(),
        HirType::Int => "0".to_owned(),
        HirType::Float => "0.0".to_owned(),
        HirType::Bool => "True".to_owned(),
        HirType::Uuid => "str(uuid4())".to_owned(),
        HirType::Timestamp => "datetime.now(timezone.utc)".to_owned(),
        HirType::Json => "{}".to_owned(),
        HirType::Enum { variants } => {
            format!("{:?}", variants.first().cloned().unwrap_or_default())
        }
        HirType::Record(id) => {
            let record = ir.record(*id);
            let args = record
                .fields
                .iter()
                // A to-many reference has no generated Pydantic
                // attribute yet (v0.16 M4 — see `build_record`), so it
                // isn't a constructor kwarg here either.
                .filter(|f| {
                    !matches!(
                        f.ty,
                        ciac_ir::FieldType::Reference {
                            cardinality: ciac_ir::Cardinality::Many,
                            ..
                        }
                    )
                })
                .map(|f| {
                    let name = match &f.ty {
                        ciac_ir::FieldType::Reference {
                            cardinality: ciac_ir::Cardinality::One,
                            ..
                        } => format!("{}_id", f.name),
                        _ => f.name.clone(),
                    };
                    format!("{name}={}", dummy_field(&f.ty))
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!("{}({args})", record.name)
        }
        HirType::List(_) => "[]".to_owned(),
        HirType::Option(_) | HirType::Unit | HirType::Never => "None".to_owned(),
    }
}

fn dummy_value_needs(ir: &NormalizedIr, ty: &HirType, uuid: &mut bool, datetime: &mut bool) {
    use ciac_ir::FieldType;
    match ty {
        HirType::Uuid => *uuid = true,
        HirType::Timestamp => *datetime = true,
        HirType::Record(id) => {
            for field in &ir.record(*id).fields {
                match field.ty {
                    FieldType::Uuid => *uuid = true,
                    FieldType::Timestamp => *datetime = true,
                    // A to-one reference's dummy value is `str(uuid4())`
                    // (v0.16 M4), so it needs the same import.
                    FieldType::Reference {
                        cardinality: ciac_ir::Cardinality::One,
                        ..
                    } => *uuid = true,
                    _ => {}
                }
            }
        }
        HirType::Option(inner) | HirType::List(inner) => {
            dummy_value_needs(ir, inner, uuid, datetime)
        }
        _ => {}
    }
}

fn assert_result(ir: &NormalizedIr, ty: &HirType) -> Option<String> {
    match ty {
        HirType::Record(id) => Some(format!(
            "    assert isinstance(result, {})",
            record_class_name(ir, *id)
        )),
        HirType::Str => Some("    assert isinstance(result, str)".to_owned()),
        HirType::Int => Some("    assert isinstance(result, int)".to_owned()),
        HirType::Float => Some("    assert isinstance(result, float)".to_owned()),
        HirType::Bool => Some("    assert isinstance(result, bool)".to_owned()),
        HirType::List(_) => Some("    assert isinstance(result, list)".to_owned()),
        HirType::Uuid | HirType::Timestamp | HirType::Json | HirType::Enum { .. } => None,
        HirType::Option(_) | HirType::Unit | HirType::Never => None,
    }
}

/// A generated `pytest` exercising one inline handler's lowered body
/// against mocked runtime dependencies: proves the lowering calls the
/// right runtime APIs. `None` for `extern` handlers (no body to test).
pub fn render_test(ir: &NormalizedIr, hir: &HandlerBody, ctx: &LogicFileCtx) -> Option<String> {
    hir.body.as_ref()?;
    let needs = scan(ir, hir);
    let mut lines = Vec::new();
    lines.push(format!(
        "\"\"\"Generated behavioral test for `{}`. Regenerated on every build.\n\nExercises the lowered handler body against mocked runtime dependencies;\nreal persistence round-trips are `ciac verify --live`'s job, not this test.\n\"\"\"",
        ctx.class_name
    ));
    // 27UpdatePlan.md M3: stdlib imports (`unittest.mock`/`uuid`/
    // `datetime`) must precede the third-party `pytest` import, each
    // group separated by a blank line -- `ruff`'s isort otherwise flags
    // "un-sorted or un-formatted" and `ciac verify`'s lint gate fails
    // the build. The previous ordering (`pytest` first, stdlib after)
    // never tripped this because no checked-in example had exercised a
    // handler test needing both until this arc's simulation corpus.
    let mut uuid = false;
    let mut datetime = false;
    for (_, ty) in &hir.params {
        dummy_value_needs(ir, ty, &mut uuid, &mut datetime);
    }
    // Only import what the mocked-dependency setup below actually uses —
    // a handler with no capability calls (e.g. a pure transform) needs
    // neither, and an unused import fails `ruff check`.
    let needs_async_mock =
        ctx.needs_db || ctx.needs_cache || ctx.needs_queue || !ctx.extras.is_empty();
    let needs_magic_mock = ctx.needs_db;
    let stdlib_before = lines.len();
    if datetime {
        lines.push("from datetime import datetime, timezone".to_owned());
    }
    match (needs_async_mock, needs_magic_mock) {
        (true, true) => lines.push("from unittest.mock import AsyncMock, MagicMock".to_owned()),
        (true, false) => lines.push("from unittest.mock import AsyncMock".to_owned()),
        (false, true) => lines.push("from unittest.mock import MagicMock".to_owned()),
        (false, false) => {}
    }
    if uuid {
        lines.push("from uuid import uuid4".to_owned());
    }
    if lines.len() > stdlib_before {
        lines.push(String::new());
    }
    lines.push("import pytest".to_owned());
    lines.push(String::new());
    lines.push(format!(
        "from app.logic.{} import {}",
        ctx.module, ctx.class_name
    ));
    // Only the records the test itself names (constructing the payload,
    // asserting the result) — not `ctx.schema_imports`, which also
    // covers records only ever touched inside the (mocked-out) body,
    // e.g. an error `raise`d past the mocks.
    let mut test_records: Vec<RecordId> = Vec::new();
    for (_, ty) in &hir.params {
        collect_record_ids(ty, &mut test_records);
    }
    collect_record_ids(&hir.return_ty, &mut test_records);
    let mut test_schema_imports: Vec<String> = test_records
        .iter()
        .map(|id| record_class_name(ir, *id))
        .collect();
    test_schema_imports.sort();
    test_schema_imports.dedup();
    // 27UpdatePlan.md M3: a handler whose payload and return type are
    // different records (e.g. `NotifyUser(payload: Notification) ->
    // Ack`) used to emit one `from app.schemas import X` line per
    // record -- `ruff`'s isort flags two same-module import lines as
    // "un-sorted or un-formatted" and `ciac verify`'s lint gate fails
    // the build. Nothing had generated this shape before this arc's
    // simulation corpus needed record-returning handlers throughout
    // (to dodge a separate, out-of-scope route-wrapper bug -- see
    // `examples/single-service/sim-peripherals.ciac`'s header comment).
    if !test_schema_imports.is_empty() {
        lines.push(format!(
            "from app.schemas import {}",
            test_schema_imports.join(", ")
        ));
    }
    lines.push(String::new());
    lines.push(String::new());
    lines.push("@pytest.mark.anyio".to_owned());
    let test_params = if ctx.needs_queue { "monkeypatch" } else { "" };
    lines.push(format!(
        "async def test_{}_handle({test_params}) -> None:",
        ctx.module
    ));

    // `publish` is a bare module-level function (`from app.queue import
    // publish`), not a constructor-injected dependency like
    // `session`/`cache`/the extras — a real connection attempt would
    // need a live broker, so it's patched at the handler module's own
    // name binding instead of passed as a kwarg.
    if ctx.needs_queue {
        lines.push("    mock_publish = AsyncMock()".to_owned());
        lines.push(format!(
            "    monkeypatch.setattr(\"app.logic.{}.publish\", mock_publish)",
            ctx.module
        ));
    }

    let mut kwargs = Vec::new();
    if ctx.needs_db {
        lines.push("    session = AsyncMock()".to_owned());
        lines.push("    session.add = MagicMock()".to_owned());
        if needs.db_get {
            lines.push("    session.get = AsyncMock(return_value=None)".to_owned());
        }
        if needs.sa_query {
            // `db.query`/`count`/`delete_where` go through
            // `session.execute(..)`, not `session.get(..)` — its return
            // value needs `.scalars().all()`/`.scalar_one()`/`.rowcount`
            // configured so the lowered body's isinstance assertions
            // below (list/int) actually hold; MagicMock's `__iter__`
            // already defaults to `iter([])`, but `.scalar_one()` and
            // `.rowcount` default to a bare `MagicMock`, not an `int`.
            lines.push("    exec_result = MagicMock()".to_owned());
            lines.push("    exec_result.scalar_one.return_value = 0".to_owned());
            lines.push("    exec_result.rowcount = 0".to_owned());
            lines.push("    session.execute = AsyncMock(return_value=exec_result)".to_owned());
        }
        kwargs.push("session=session".to_owned());
    }
    if ctx.needs_cache {
        lines.push("    cache = AsyncMock()".to_owned());
        if needs.cache_get {
            lines.push("    cache.get = AsyncMock(return_value=None)".to_owned());
        }
        kwargs.push("cache=cache".to_owned());
    }
    for extra in &ctx.extras {
        let var = &extra.param;
        lines.push(format!("    {var} = AsyncMock()"));
        if extra.kind == "object_store" && needs.object_store_get {
            lines.push(format!("    {var}.get = AsyncMock(return_value=b\"null\")"));
        }
        if extra.kind == "object_store" && needs.object_store_list {
            lines.push(format!("    {var}.list = AsyncMock(return_value=[])"));
        }
        kwargs.push(format!("{var}={var}"));
    }
    lines.push(format!(
        "    handler = {}({})",
        ctx.class_name,
        kwargs.join(", ")
    ));
    lines.push(String::new());

    let args = hir
        .params
        .iter()
        .map(|(_, ty)| dummy_value(ir, ty))
        .collect::<Vec<_>>()
        .join(", ");
    lines.push(format!("    result = await handler.handle({args})"));
    lines.push(String::new());
    if let Some(assertion) = assert_result(ir, &hir.return_ty) {
        lines.push(assertion);
    }
    if needs.db_insert == 1 {
        lines.push("    session.add.assert_called_once()".to_owned());
        lines.push("    session.commit.assert_awaited_once()".to_owned());
    } else if needs.db_insert > 1 {
        // v0.16: a `transaction` block may legitimately insert into more
        // than one table, so the call count isn't always 1 — but it's
        // still exactly `needs.db_insert`, known at codegen time.
        lines.push(format!(
            "    assert session.add.call_count == {}",
            needs.db_insert
        ));
        lines.push("    session.commit.assert_awaited()".to_owned());
    }
    if needs.db_get {
        lines.push("    session.get.assert_awaited_once()".to_owned());
    }
    if needs.sa_query {
        lines.push("    session.execute.assert_awaited_once()".to_owned());
    }
    if needs.cache_set {
        lines.push("    cache.set.assert_awaited_once()".to_owned());
    }
    if needs.cache_get {
        lines.push("    cache.get.assert_awaited_once()".to_owned());
    }
    if ctx.needs_queue {
        lines.push("    mock_publish.assert_awaited_once()".to_owned());
    }
    for extra in &ctx.extras {
        if extra.kind == "object_store" {
            let var = &extra.param;
            if needs.object_store_put {
                lines.push(format!("    {var}.put.assert_awaited_once()"));
            }
            if needs.object_store_get {
                lines.push(format!("    {var}.get.assert_awaited_once()"));
            }
            if needs.object_store_list {
                lines.push(format!("    {var}.list.assert_awaited_once()"));
            }
        }
    }

    Some(lines.join("\n") + "\n")
}
