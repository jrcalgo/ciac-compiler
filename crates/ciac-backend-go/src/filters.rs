//! Backend-owned minijinja filters rendering neutral model data into
//! Go syntax (following `22UpdatePlan.md` Pillar 2 Move 2's pattern,
//! set by `ciac-backend-python`/`-rust`/`-ts`'s own `filters.rs`).
//! Registered once in [`crate::environment`]; nothing per-language is
//! precomputed in `ciac-codegen::model` for what these cover.

use ciac_codegen::model::FieldTypeKind;
use minijinja::value::ViaDeserialize;
use serde::Deserialize;

/// The only piece of a `FieldCtx` these filters need — see the other
/// backends' identical wrapper for why deserializing just this shape
/// (serde ignores the rest) is what lets templates write
/// `{{ field | go_type }}` instead of `{{ field.type_kind | go_type }}`.
#[derive(Deserialize)]
pub(crate) struct HasTypeKind {
    type_kind: FieldTypeKind,
}

/// Go type for a field's neutral type, e.g. `string`, `time.Time`,
/// `VideoStatus`. Every record field is currently non-optional (there
/// is no `Option<T>` at the `FieldCtx` level — v0.14 M1's optional
/// types apply to typed-handler signatures, an HIR-level concern
/// `lower.rs` handles, not to declared record fields), so this never
/// needs to return a pointer type per Pillar 2's Option discipline.
pub fn go_type(field: ViaDeserialize<HasTypeKind>) -> String {
    go_type_of(field.0.type_kind.clone())
}

/// The Go zero value for a field's type, used by generated code that
/// needs an explicit zero-value return alongside a non-nil error (the
/// multiple-return error idiom in a record's own methods, e.g.
/// `DecodeXxx`'s failure paths return `Xxx{}, err`. Records always
/// zero-value-return the whole struct, so this filter exists for the
/// rare per-field zero (map/slice defaults) rather than the common
/// case.
pub fn go_zero(field: ViaDeserialize<HasTypeKind>) -> String {
    match field.0.type_kind {
        FieldTypeKind::Str | FieldTypeKind::Uuid | FieldTypeKind::Reference { .. } => {
            "\"\"".to_owned()
        }
        FieldTypeKind::Int => "0".to_owned(),
        FieldTypeKind::Float => "0".to_owned(),
        FieldTypeKind::Bool => "false".to_owned(),
        FieldTypeKind::Timestamp => "time.Time{}".to_owned(),
        FieldTypeKind::Json => "nil".to_owned(),
        FieldTypeKind::Enum { name, .. } => format!("{name}(\"\")"),
    }
}

/// Go type as stored in the database (enums are TEXT -> `string`);
/// mirrors Rust's `db_rust_type`. A `table`/`crud` row is scanned into
/// this shape, then converted to the wire type at the record boundary
/// (see `models.go.j2`'s `TryFrom`-equivalent conversion).
pub fn go_db_type(field: ViaDeserialize<HasTypeKind>) -> String {
    if matches!(field.0.type_kind, FieldTypeKind::Enum { .. }) {
        "string".to_owned()
    } else {
        go_type_of(field.0.type_kind.clone())
    }
}

fn go_type_of(kind: FieldTypeKind) -> String {
    match kind {
        FieldTypeKind::Str | FieldTypeKind::Uuid | FieldTypeKind::Reference { .. } => {
            "string".to_owned()
        }
        FieldTypeKind::Int => "int64".to_owned(),
        FieldTypeKind::Float => "float64".to_owned(),
        FieldTypeKind::Bool => "bool".to_owned(),
        FieldTypeKind::Timestamp => "time.Time".to_owned(),
        FieldTypeKind::Json => "json.RawMessage".to_owned(),
        FieldTypeKind::Enum { name, .. } => name,
    }
}

/// Common initialisms `staticcheck`'s default ST1003 check (and
/// idiomatic Go generally) wants fully capitalized in an identifier
/// (`URL`, not `Url`; `ID`, not `Id`) — the same fixed list
/// `golang.org/x/lint`/`staticcheck` ship. Applied by [`go_pascal`]
/// after word-splitting, since the shared `pascal_case` filter
/// (`heck`, installed by every backend) has no notion of Go's
/// initialism convention and would otherwise emit
/// `DatabaseUrl`/`JwtSecret` — technically valid Go, but exactly what
/// `staticcheck` flags, so this is a real lint-cleanliness cost, not
/// cosmetic preference.
///
/// `Oauth` is deliberately absent: Go code conventionally spells it
/// `OAuth` (mixed case, not all-caps like a true initialism), and
/// every `auth_scheme == "oauth2"` field name is hand-spelled in its
/// own template rather than routed through this filter, so no
/// auto-uppercased "OAUTH" ever needs reconciling against that.
const INITIALISMS: &[&str] = &[
    "Api", "Db", "Http", "Https", "Id", "Json", "Jwt", "Sql", "Url", "Uuid", "Otel",
];

fn fix_initialisms(word: &str) -> String {
    for initialism in INITIALISMS {
        if word.eq_ignore_ascii_case(initialism) {
            return initialism.to_uppercase();
        }
    }
    word.to_owned()
}

/// Go `PascalCase` for `snake_case`/`kebab-case` input, splitting on
/// `heck`'s own word boundaries and re-title-casing each word through
/// [`fix_initialisms`] so `database_url` -> `DatabaseURL`,
/// `jwt_secret` -> `JWTSecret`.
fn pascal_with_initialisms(input: &str) -> String {
    use heck::ToPascalCase;
    let merged = input.to_pascal_case();
    let mut words: Vec<String> = Vec::new();
    for (i, ch) in merged.char_indices() {
        if i == 0 || ch.is_uppercase() {
            words.push(String::new());
        }
        words.last_mut().expect("just pushed").push(ch);
    }
    words.iter().map(|w| fix_initialisms(w)).collect()
}

/// Go `PascalCase` identifier for a `snake_case`/`kebab-case` name,
/// with common initialisms capitalized per Go convention.
pub fn go_pascal(input: String) -> String {
    pascal_with_initialisms(&input)
}

/// `validate` struct-tag fragment for a field's neutral type (Pillar 2):
/// `uuid4` for `Uuid`, membership-by-enum-switch is generated
/// separately (not a validator tag) so an invalid string reports the
/// same "not a member of {enum}" shape every other target's decoder
/// gives, rather than validator's generic tag-failure message.
pub fn go_validate_tag(field: ViaDeserialize<HasTypeKind>) -> String {
    match field.0.type_kind {
        FieldTypeKind::Uuid => "uuid4".to_owned(),
        _ => String::new(),
    }
}
