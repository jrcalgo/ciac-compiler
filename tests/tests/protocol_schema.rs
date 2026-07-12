//! v0.10 M2: `docs/protocol-schema.json` is the external-backend wire
//! contract's JSON Schema, checked in for backend authors to consume
//! without running `ciac codegen-schema` themselves. This test holds
//! it byte-identical to what the current types actually derive — the
//! checked-in copy can never silently go stale.

#[test]
fn checked_in_protocol_schema_matches_the_derived_one() {
    let derived = serde_json::to_string_pretty(&ciac_codegen::protocol::schema_document())
        .expect("schema document serializes");
    // `ciac codegen-schema > docs/protocol-schema.json` ends with the
    // newline `println!` adds.
    let derived = format!("{derived}\n");
    let checked_in = include_str!("../../docs/protocol-schema.json");
    assert_eq!(
        checked_in, derived,
        "docs/protocol-schema.json is stale; regenerate it with \
         `cargo run -p ciac -- codegen-schema > docs/protocol-schema.json`"
    );
}
