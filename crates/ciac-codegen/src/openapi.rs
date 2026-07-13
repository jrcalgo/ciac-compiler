//! OpenAPI 3.0 document generation (v0.15 M1).
//!
//! One serializer, driven entirely by the same [`crate::model::Ctx`]
//! every backend already renders from — so the document a client sees
//! is provably the same routes/schemas the generated app actually
//! serves, not a hand-maintained approximation. Both backends embed
//! this output verbatim (Python mounts it as FastAPI's `openapi()`
//! source instead of letting FastAPI derive its own; Rust serves it at
//! `/openapi.json` via `include_str!`), so there is exactly one
//! specification per service, not two near-duplicates.
//!
//! Scope, deliberately: every `api` and every `crud`-expanded route,
//! `/health`, request/response records as component schemas (enums
//! included), and scope requirements on secured routes. Realtime
//! `channel`s (WebSocket/SSE) have no OpenAPI representation and are
//! not emitted here — that's AsyncAPI's problem, out of scope for a
//! REST document.

use crate::model::{
    ApiCtx, Ctx, EnumCtx, FieldCtx, FieldTypeKind, RecordCtx, ResourceCtx, SystemModel,
};
use serde_json::{json, Map, Value};

const OPENAPI_VERSION: &str = "3.0.3";
/// Fixed dev version for the generated app's own spec — matches the
/// `0.1.0` every generated `pyproject.toml`/`Cargo.toml` already ships.
const DOC_VERSION: &str = "0.1.0";
const SECURITY_SCHEME: &str = "bearerAuth";

/// Builds one service's OpenAPI document.
pub fn build_document(ctx: &Ctx) -> Value {
    let mut paths: Map<String, Value> = Map::new();
    paths.insert("/health".to_owned(), health_path_item());

    for api in &ctx.apis {
        let item = paths
            .entry(api.route.clone())
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .expect("path item is an object");
        item.insert(api.method_lower.clone(), api_operation(api));
    }

    for resource in &ctx.resources {
        let base = format!("/{}", resource.plural);
        let collection = paths
            .entry(base.clone())
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .expect("path item is an object");
        collection.insert(
            "post".to_owned(),
            resource_operation(resource, "create", true, true, 201),
        );
        collection.insert("get".to_owned(), resource_list_operation(resource));

        let by_id = format!("{base}/{{id}}");
        let item = paths
            .entry(by_id)
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .expect("path item is an object");
        item.insert(
            "get".to_owned(),
            resource_operation(resource, "get", false, true, 200),
        );
        item.insert(
            "put".to_owned(),
            resource_operation(resource, "update", true, true, 200),
        );
        item.insert("delete".to_owned(), resource_delete_operation(resource));
    }

    let mut schemas: Map<String, Value> = Map::new();
    for record in &ctx.records {
        schemas.insert(record.name.clone(), record_schema(record));
        for e in &record.enums {
            schemas.insert(e.name.clone(), enum_schema(e));
        }
    }
    for resource in &ctx.resources {
        let (in_schema, out_schema) = resource_schemas(resource);
        schemas.insert(format!("{}In", resource.name), in_schema);
        schemas.insert(format!("{}Out", resource.name), out_schema);
        if let Some(record) = &resource.record {
            for e in &record.enums {
                schemas
                    .entry(e.name.clone())
                    .or_insert_with(|| enum_schema(e));
            }
        }
    }

    let mut components = Map::new();
    components.insert("schemas".to_owned(), Value::Object(schemas));
    if ctx.has_auth {
        components.insert(
            "securitySchemes".to_owned(),
            json!({
                SECURITY_SCHEME: {
                    "type": "http",
                    "scheme": "bearer",
                    "bearerFormat": "JWT",
                }
            }),
        );
    }

    json!({
        "openapi": OPENAPI_VERSION,
        "info": {"title": ctx.service_name, "version": DOC_VERSION},
        "paths": Value::Object(paths),
        "components": Value::Object(components),
    })
}

/// A lightweight index for multi-service systems: `ciac build` writes
/// one full spec per service (`<dir>/openapi.json`) plus this at the
/// system root, pointing at each. Deliberately not itself a merged
/// OpenAPI document — services can (and do) share route shapes like
/// `/health`, so a naive path union would silently drop entries.
pub fn build_index(system: &SystemModel) -> Value {
    let services: Vec<Value> = system
        .services
        .iter()
        .map(|ctx| {
            json!({
                "name": ctx.service_name,
                "spec": format!("{}/openapi.json", ctx.dir),
            })
        })
        .collect();
    json!({
        "openapi-index": "0.15",
        "project": system.project_name,
        "services": services,
    })
}

fn health_path_item() -> Value {
    json!({
        "get": {
            "operationId": "health",
            "responses": {
                "200": {
                    "description": "Liveness check.",
                    "content": {
                        "application/json": {
                            "schema": {
                                "type": "object",
                                "properties": {"status": {"type": "string"}},
                                "required": ["status"],
                            }
                        }
                    }
                }
            }
        }
    })
}

/// The envelope every `api` route actually returns
/// (`{"status": "accepted", "data": ..}`) — see `api.py.j2`/
/// `route_api.rs.j2`, both of which wrap the pipeline's result the
/// same way rather than returning it bare.
fn envelope_schema(payload: Option<&crate::model::PayloadRef>) -> Value {
    let data = match payload {
        Some(p) => json!({"$ref": format!("#/components/schemas/{}", p.class_name)}),
        None => json!({"type": "object", "additionalProperties": true}),
    };
    json!({
        "type": "object",
        "properties": {
            "status": {"type": "string"},
            "data": data,
        },
        "required": ["status", "data"],
    })
}

fn api_operation(api: &ApiCtx) -> Value {
    let mut op = Map::new();
    op.insert("operationId".to_owned(), json!(api.snake));
    if api.has_body {
        let schema = match &api.payload {
            Some(p) => json!({"$ref": format!("#/components/schemas/{}", p.class_name)}),
            None => json!({"type": "object", "additionalProperties": true}),
        };
        op.insert(
            "requestBody".to_owned(),
            json!({
                "required": true,
                "content": {"application/json": {"schema": schema}},
            }),
        );
    }
    op.insert(
        "responses".to_owned(),
        json!({
            "200": {
                "description": "Success.",
                "content": {"application/json": {"schema": envelope_schema(api.payload.as_ref())}},
            }
        }),
    );
    apply_security(&mut op, api.has_auth_step, api.scope.as_deref());
    Value::Object(op)
}

fn apply_security(op: &mut Map<String, Value>, requires_auth: bool, scope: Option<&str>) {
    if !requires_auth {
        return;
    }
    op.insert("security".to_owned(), json!([{SECURITY_SCHEME: []}]));
    if let Some(scope) = scope {
        op.insert("x-ciac-scope".to_owned(), json!(scope));
    }
}

fn resource_ref(resource: &ResourceCtx, suffix: &str) -> Value {
    json!({"$ref": format!("#/components/schemas/{}{}", resource.name, suffix)})
}

fn resource_operation(
    resource: &ResourceCtx,
    op_name: &str,
    has_body: bool,
    scoped: bool,
    status: u16,
) -> Value {
    let mut op = Map::new();
    op.insert(
        "operationId".to_owned(),
        json!(format!("{op_name}_{}", resource.snake)),
    );
    if has_body {
        op.insert(
            "requestBody".to_owned(),
            json!({
                "required": true,
                "content": {"application/json": {"schema": resource_ref(resource, "In")}},
            }),
        );
    }
    if op_name != "create" {
        op.insert(
            "parameters".to_owned(),
            json!([{
                "name": "id",
                "in": "path",
                "required": true,
                "schema": {"type": "string"},
            }]),
        );
    }
    let mut responses = Map::new();
    responses.insert(
        status.to_string(),
        json!({
            "description": "Success.",
            "content": {"application/json": {"schema": resource_ref(resource, "Out")}},
        }),
    );
    responses.insert("404".to_owned(), json!({"description": "Not found."}));
    op.insert("responses".to_owned(), Value::Object(responses));
    let scope = if scoped {
        if has_body {
            resource.write_scope.as_deref()
        } else {
            resource.read_scope.as_deref()
        }
    } else {
        None
    };
    apply_security(&mut op, resource.has_auth, scope);
    Value::Object(op)
}

fn resource_list_operation(resource: &ResourceCtx) -> Value {
    let mut op = Map::new();
    op.insert(
        "operationId".to_owned(),
        json!(format!("list_{}", resource.plural)),
    );
    op.insert(
        "parameters".to_owned(),
        json!([
            {"name": "limit", "in": "query", "schema": {"type": "integer", "default": resource.page_size}},
            {"name": "offset", "in": "query", "schema": {"type": "integer", "default": 0}},
        ]),
    );
    op.insert(
        "responses".to_owned(),
        json!({
            "200": {
                "description": "Success.",
                "content": {
                    "application/json": {
                        "schema": {"type": "array", "items": resource_ref(resource, "Out")}
                    }
                }
            }
        }),
    );
    apply_security(&mut op, resource.has_auth, resource.read_scope.as_deref());
    Value::Object(op)
}

fn resource_delete_operation(resource: &ResourceCtx) -> Value {
    let mut op = Map::new();
    op.insert(
        "operationId".to_owned(),
        json!(format!("delete_{}", resource.snake)),
    );
    op.insert(
        "parameters".to_owned(),
        json!([{
            "name": "id",
            "in": "path",
            "required": true,
            "schema": {"type": "string"},
        }]),
    );
    op.insert(
        "responses".to_owned(),
        json!({
            "204": {"description": "Deleted."},
            "404": {"description": "Not found."},
        }),
    );
    apply_security(&mut op, resource.has_auth, resource.write_scope.as_deref());
    Value::Object(op)
}

fn field_schema(field: &FieldCtx) -> Value {
    match &field.type_kind {
        FieldTypeKind::Str => json!({"type": "string"}),
        FieldTypeKind::Int => json!({"type": "integer"}),
        FieldTypeKind::Float => json!({"type": "number"}),
        FieldTypeKind::Bool => json!({"type": "boolean"}),
        FieldTypeKind::Uuid => json!({"type": "string", "format": "uuid"}),
        FieldTypeKind::Timestamp => json!({"type": "string", "format": "date-time"}),
        FieldTypeKind::Json => json!({"type": "object", "additionalProperties": true}),
        FieldTypeKind::Enum { name, .. } => {
            json!({"$ref": format!("#/components/schemas/{name}")})
        }
    }
}

fn fields_object<'a>(fields: impl IntoIterator<Item = &'a FieldCtx>) -> Value {
    let mut properties = Map::new();
    let mut required: Vec<String> = Vec::new();
    for field in fields {
        properties.insert(field.name.clone(), field_schema(field));
        required.push(field.name.clone());
    }
    json!({
        "type": "object",
        "properties": Value::Object(properties),
        "required": required,
    })
}

fn record_schema(record: &RecordCtx) -> Value {
    fields_object(&record.fields)
}

fn enum_schema(e: &EnumCtx) -> Value {
    json!({"type": "string", "enum": e.variants})
}

/// The `{Name}In`/`{Name}Out` pair `models.py.j2` generates for a CRUD
/// resource: typed columns from the bound record (`In` drops `id`,
/// `Out` always carries it), or the generic `id`+`data` document shape
/// when the resource has no record.
fn resource_schemas(resource: &ResourceCtx) -> (Value, Value) {
    match &resource.record {
        Some(record) => {
            let non_id: Vec<&FieldCtx> = record.fields.iter().filter(|f| f.name != "id").collect();
            let in_schema = fields_object(non_id.iter().copied());
            let mut out_properties = Map::new();
            let mut out_required = vec!["id".to_owned()];
            out_properties.insert("id".to_owned(), json!({"type": "string"}));
            for field in &non_id {
                out_properties.insert(field.name.clone(), field_schema(field));
                out_required.push(field.name.clone());
            }
            let out_schema = json!({
                "type": "object",
                "properties": Value::Object(out_properties),
                "required": out_required,
            });
            (in_schema, out_schema)
        }
        None => {
            let data = json!({"type": "object", "additionalProperties": true});
            let in_schema = json!({
                "type": "object",
                "properties": {"data": data},
            });
            let out_schema = json!({
                "type": "object",
                "properties": {"id": {"type": "string"}, "data": data},
                "required": ["id", "data"],
            });
            (in_schema, out_schema)
        }
    }
}
