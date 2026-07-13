//! v0.15 M2: golden coverage for `ciac build --client ts`'s generated
//! TypeScript package — independent of `--target`, so it gets its own
//! snapshot pass rather than riding the per-backend `golden.rs` loop.

use ciac_codegen::model::build_system;
use ciac_codegen::GenOptions;
use ciac_integration_tests::{ciac_files, compile_file, examples_dir};

#[test]
fn example_ts_client_snapshots() {
    for path in ciac_files(&examples_dir()) {
        let name = path.file_stem().expect("file name").to_string_lossy();
        let ir = compile_file(&path);
        let system = build_system(&ir, &GenOptions::default());
        let files = ciac_codegen::ts_client::build(&system);
        let mut dump = String::new();
        for (path, content) in &files {
            dump.push_str(&"=".repeat(60));
            dump.push('\n');
            dump.push_str(path);
            dump.push('\n');
            dump.push_str(&"=".repeat(60));
            dump.push('\n');
            dump.push_str(content);
        }
        insta::assert_snapshot!(name.as_ref(), dump);
    }
}
