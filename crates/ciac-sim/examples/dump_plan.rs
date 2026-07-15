//! A dev-only helper, not part of `ciac`'s public CLI surface (that's
//! `ciac sim`'s job, M10) -- prints one `.ciac` file's `SimPlan` as
//! JSON to stdout. v0.17 M6 uses this so the Python fake database
//! (`sim/pyrunner/world.py`) can enforce the same reference/unique/
//! cascade metadata `SimPlan` already derives from validated IR,
//! rather than re-deriving schema knowledge a second time in Python
//! from scratch or trying to reflect it out of SQLAlchemy model
//! metadata, which (see 17UpdatePlan.md's M6 milestone entry) carries
//! none of it -- generated `models.py` columns are plain, constraint-
//! free `Mapped[str]` declarations; real constraints live only in
//! migration SQL and (canonically) in `SimPlan` itself.
//!
//! Usage: `cargo run --example dump_plan -p ciac-sim -- <path.ciac>`

use ciac_diagnostics::{Diagnostics, SourceMap};
use ciac_sim::SimPlan;
use sha2::{Digest, Sha256};
use std::path::PathBuf;

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: dump_plan <path.ciac>");
        std::process::exit(2);
    });
    let source = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        eprintln!("reading {path}: {e}");
        std::process::exit(1);
    });

    let mut sources = SourceMap::new();
    let file = sources.add_file(PathBuf::from(&path).display().to_string(), &source);
    let mut diags = Diagnostics::new();
    let program = ciac_syntax::parse(&source, file, &mut diags);
    let ir = ciac_sema::analyze(&program, &mut diags);
    if diags.has_errors() {
        eprintln!("{path} has compile errors; SimPlan requires validated IR");
        std::process::exit(1);
    }
    let ir = ir.expect("no errors implies IR present");

    let mut hasher = Sha256::new();
    hasher.update(source.as_bytes());
    let source_hash = format!("sha256:{:x}", hasher.finalize());

    let plan = SimPlan::from_ir(&ir, source_hash);
    println!(
        "{}",
        serde_json::to_string_pretty(&plan).expect("SimPlan serializes")
    );
}
