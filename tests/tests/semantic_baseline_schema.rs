//! v0.18 M1: `docs/semantic-baseline-schema.json` is the checked-in
//! semantic baseline's JSON Schema — held byte-identical to what the
//! current types actually derive, mirroring `protocol_schema.rs`'s
//! pattern for the external-backend wire contract.

#[test]
fn checked_in_semantic_baseline_schema_matches_the_derived_one() {
    let derived =
        serde_json::to_string_pretty(&ciac_codegen::semantic_model::baseline_schema_document())
            .expect("schema document serializes");
    let derived = format!("{derived}\n");
    let checked_in = include_str!("../../docs/semantic-baseline-schema.json");
    assert_eq!(
        checked_in, derived,
        "docs/semantic-baseline-schema.json is stale; regenerate it with \
         `cargo run -p ciac -- semantic-baseline-schema > docs/semantic-baseline-schema.json`"
    );
}
