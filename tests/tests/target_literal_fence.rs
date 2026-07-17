//! The "grep fence" (v0.22 M1 — 22UpdatePlan.md Pillar 1): a repo test
//! that keeps seam 3 (scattered per-target string matches) from
//! regrowing now that `TargetInfo` closes the audited instances of it.
//!
//! Scans every `crates/*/src/**/*.rs` file *except* the backend crates
//! themselves (where naming the target is the whole point) for the
//! literal target-name strings `"python"`/`"rust"`. A survivor is
//! allowed only when the line is a comment (prose), inside a
//! `#[cfg(test)]` module (test fixtures that exercise a specific
//! target are expected — 22UpdatePlan.md names this class explicitly),
//! or carries a `// target-literal-ok: <reason>` justification. Any
//! other survivor fails the test: a new match-on-target-name site is
//! exactly the accidental cost this milestone removed.

use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// Crate `src/` roots to scan — every workspace crate with a `src/`
/// directory except the two backend crates, where `"python"`/`"rust"`
/// naming the target is the crate's entire purpose.
fn scan_roots() -> Vec<PathBuf> {
    let crates_dir = repo_root().join("crates");
    let mut roots = Vec::new();
    for entry in std::fs::read_dir(&crates_dir).expect("crates/ exists") {
        let entry = entry.expect("readable entry");
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == "ciac-backend-python" || name == "ciac-backend-rust" {
            continue;
        }
        let src = path.join("src");
        if src.is_dir() {
            roots.push(src);
        }
    }
    roots
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap_or_else(|e| panic!("cannot read {dir:?}: {e}")) {
        let entry = entry.expect("readable entry");
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// Lines from `text` up to (not including) the first `#[cfg(test)]`
/// item — every file in this codebase that has one puts its test
/// module last, so this is a safe, simple way to exclude test
/// fixtures without a full parser.
fn non_test_lines(text: &str) -> impl Iterator<Item = (usize, &str)> {
    let cutoff = text
        .lines()
        .position(|l| l.trim_start().starts_with("#[cfg(test)]"))
        .unwrap_or(text.lines().count());
    text.lines().enumerate().take(cutoff)
}

#[test]
fn no_unjustified_target_name_literals_outside_backend_crates() {
    let mut files = Vec::new();
    for root in scan_roots() {
        rust_files(&root, &mut files);
    }
    files.sort();

    let mut violations = Vec::new();
    for file in &files {
        let text = std::fs::read_to_string(file)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", file.display()));
        let all_lines: Vec<&str> = text.lines().collect();
        for (idx, line) in non_test_lines(&text) {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") || trimmed.starts_with("///") || trimmed.starts_with("//!")
            {
                continue; // prose/docs — always allowed
            }
            if !(line.contains("\"python\"") || line.contains("\"rust\"")) {
                continue;
            }
            // The justification may sit on the flagged line itself, or in a
            // block comment immediately above it (up to 12 lines back) —
            // annotating a whole match arm/function once, not every line.
            let window_start = idx.saturating_sub(12);
            let justified = all_lines[window_start..=idx]
                .iter()
                .any(|l| l.contains("target-literal-ok:"));
            if justified {
                continue;
            }
            violations.push(format!("{}:{}: {}", file.display(), idx + 1, line.trim()));
        }
    }

    assert!(
        violations.is_empty(),
        "found target-name literals outside the backend crates/registry, with no \
         `// target-literal-ok: <reason>` justification (v0.22 M1 grep fence — see \
         22UpdatePlan.md Pillar 1):\n{}",
        violations.join("\n")
    );
}
