//! `26UpdatePlan.md` M7: docs/backends.md's two divergence-ledger
//! tables (Permanent by design / Open (tracked)) can't rot silently.
//! Every "Closes in" reference (other than an explicit "no plan yet")
//! must name a plan file that actually exists under [`PLANS_DIR`], and
//! no divergence string may appear in both tables.

use std::path::Path;

/// Splits a markdown table row into trimmed cells, or `None` if `line`
/// isn't a `|`-delimited row at all.
fn row_cells(line: &str) -> Option<Vec<String>> {
    let line = line.trim();
    if !line.starts_with('|') {
        return None;
    }
    Some(
        line.trim_matches('|')
            .split('|')
            .map(|c| c.trim().to_owned())
            .collect(),
    )
}

/// True for the `| --- | --- |` separator row (dashes/colons/spaces only).
fn is_separator_row(cells: &[String]) -> bool {
    cells
        .iter()
        .all(|c| !c.is_empty() && c.chars().all(|ch| ch == '-' || ch == ':'))
}

/// Pulls out every markdown table (a contiguous run of `|`-prefixed
/// lines, header + separator + data) following a `### <heading>` line,
/// keyed by that heading. The returned rows exclude the header and
/// separator.
fn tables_by_heading(doc: &str) -> Vec<(String, Vec<Vec<String>>)> {
    let mut out = Vec::new();
    let mut lines = doc.lines().peekable();
    while let Some(line) = lines.next() {
        let Some(heading) = line.strip_prefix("### ") else {
            continue;
        };
        // Skip blank lines up to the header row.
        while lines.peek().is_some_and(|l| l.trim().is_empty()) {
            lines.next();
        }
        let mut rows = Vec::new();
        let mut seen_separator = false;
        while let Some(next) = lines.peek() {
            match row_cells(next) {
                Some(cells) if !seen_separator && is_separator_row(&cells) => {
                    seen_separator = true;
                    lines.next();
                }
                Some(cells) if seen_separator => {
                    rows.push(cells);
                    lines.next();
                }
                Some(_) => {
                    // header row, before the separator has been seen
                    lines.next();
                }
                None => break, // table ended
            }
        }
        if !rows.is_empty() {
            out.push((heading.trim().to_owned(), rows));
        }
    }
    out
}

fn repo_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .canonicalize()
        .expect("repo root resolves")
}

/// Where `NNUpdatePlan.md` files live, relative to the repo root.
/// They sat at the root until they were moved wholesale into `plans/`;
/// this test resolved against the root and started failing on every
/// row that names a real plan, because none of them were at the root
/// any more.
const PLANS_DIR: &str = "plans";

/// Finds a `<digits>UpdatePlan.md` token inside `cell`, if any.
fn plan_file_reference(cell: &str) -> Option<&str> {
    let idx = cell.find("UpdatePlan.md")?;
    let after = idx + "UpdatePlan.md".len();
    let mut start = idx;
    while start > 0 && cell.as_bytes()[start - 1].is_ascii_digit() {
        start -= 1;
    }
    Some(&cell[start..after])
}

#[test]
fn ledger_tables_are_structurally_sound() {
    let doc =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/backends.md"))
            .expect("docs/backends.md exists");

    let tables = tables_by_heading(&doc);
    let permanent = tables
        .iter()
        .find(|(h, _)| h == "Permanent by design")
        .map(|(_, rows)| rows)
        .expect("docs/backends.md has a '### Permanent by design' table");
    let open = tables
        .iter()
        .find(|(h, _)| h == "Open (tracked)")
        .map(|(_, rows)| rows)
        .expect("docs/backends.md has a '### Open (tracked)' table");

    assert!(!permanent.is_empty(), "Permanent by design table is empty");
    assert!(!open.is_empty(), "Open (tracked) table is empty");

    let root = repo_root();
    for row in open {
        let gap = &row[0];
        let closes_in = row.last().expect("row has a Closes-in cell");
        if closes_in.to_lowercase().starts_with("no plan yet") {
            continue;
        }
        let plan_file = plan_file_reference(closes_in).unwrap_or_else(|| {
            panic!(
                "Open row {gap:?}'s 'Closes in' cell ({closes_in:?}) names \
                 neither an existing plan file nor an explicit \"no plan yet\""
            )
        });
        assert!(
            root.join(PLANS_DIR).join(plan_file).is_file(),
            "Open row {gap:?} closes in {plan_file}, which does not exist in {PLANS_DIR}/"
        );
    }

    let permanent_names: std::collections::HashSet<&str> =
        permanent.iter().map(|r| r[0].as_str()).collect();
    for row in open {
        let gap = row[0].as_str();
        assert!(
            !permanent_names.contains(gap),
            "{gap:?} appears in both the Permanent and Open tables"
        );
    }
}
