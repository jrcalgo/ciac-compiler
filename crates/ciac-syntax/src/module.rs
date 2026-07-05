//! v0.8 M1: multi-file programs via `import "path";`.
//!
//! Resolution is literal textual splicing (like `#include`, not a
//! symbol-table merge): each `import` is replaced, in place, by the
//! resolved item list of the file it names, with diamond imports (the
//! same file reachable through two different import paths) loaded
//! exactly once. By the time [`ciac_sema::analyze`] sees the returned
//! [`Program`], it is indistinguishable from one big file — every
//! downstream pass (duplicate-name checks, graph building, type
//! checking) needs zero awareness that multi-file programs exist.

use crate::ast::{Item, Program};
use crate::parser::parse;
use ciac_diagnostics::{Diagnostic, Diagnostics, ErrorCode, SourceMap};
use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};

/// Parses `entry` and recursively resolves every `import "path";` it
/// (transitively) contains into one flat [`Program`], registering each
/// file's source text in `sources` as it's read so diagnostics render
/// against the right file regardless of which one raised them.
pub fn load(entry: &Path, sources: &mut SourceMap, diags: &mut Diagnostics) -> io::Result<Program> {
    let mut loaded = BTreeSet::new();
    let mut stack = Vec::new();
    let items = resolve_file(entry, None, sources, diags, &mut loaded, &mut stack)?;
    Ok(Program { items })
}

fn resolve_file(
    path: &Path,
    importer: Option<&Path>,
    sources: &mut SourceMap,
    diags: &mut Diagnostics,
    loaded: &mut BTreeSet<PathBuf>,
    stack: &mut Vec<PathBuf>,
) -> io::Result<Vec<Item>> {
    let canonical = path.canonicalize().map_err(|err| {
        let msg = match importer {
            Some(from) => format!(
                "cannot resolve import {} (imported from {}): {err}",
                path.display(),
                from.display()
            ),
            None => format!("cannot read {}: {err}", path.display()),
        };
        io::Error::new(err.kind(), msg)
    })?;

    if loaded.contains(&canonical) {
        return Ok(Vec::new());
    }
    if let Some(pos) = stack.iter().position(|p| p == &canonical) {
        let cycle = stack[pos..]
            .iter()
            .chain(std::iter::once(&canonical))
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(" -> ");
        diags.push(Diagnostic::new(
            ErrorCode::ImportCycle,
            format!("import cycle: {cycle}"),
        ));
        return Ok(Vec::new());
    }

    let src = std::fs::read_to_string(&canonical)?;
    let file_id = sources.add_file(canonical.display().to_string(), src.clone());
    let program = parse(&src, file_id, diags);

    stack.push(canonical.clone());
    let dir = canonical
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let mut items = Vec::with_capacity(program.items.len());
    for item in program.items {
        match item {
            Item::Import(import) => {
                let imported = dir.join(&import.path);
                items.extend(resolve_file(
                    &imported,
                    Some(&canonical),
                    sources,
                    diags,
                    loaded,
                    stack,
                )?);
            }
            other => items.push(other),
        }
    }
    stack.pop();
    loaded.insert(canonical);
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ciac_diagnostics::Diagnostics;

    fn write(dir: &Path, name: &str, content: &str) -> PathBuf {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, content).unwrap();
        path
    }

    fn tmp(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ciac-module-test-{}-{}-{label}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn merges_two_files_preserving_order() {
        let dir = tmp("merge-order");
        write(&dir, "record.ciac", "record Video { id: Uuid; }\n");
        let entry = write(
            &dir,
            "entry.ciac",
            "service S;\nimport \"record.ciac\";\nstream Uploaded: Video;\n",
        );

        let mut sources = SourceMap::new();
        let mut diags = Diagnostics::new();
        let program = load(&entry, &mut sources, &mut diags).expect("loads");

        assert!(diags.is_empty(), "unexpected: {:?}", diags.codes());
        let kinds: Vec<&str> = program
            .items
            .iter()
            .map(|item| match item {
                Item::Service(_) => "service",
                Item::Record(_) => "record",
                Item::Stream(_) => "stream",
                _ => "other",
            })
            .collect();
        // The import splices `record.ciac`'s items in at the position of
        // the `import` statement — service, then the imported record,
        // then the entry file's own remaining declaration.
        assert_eq!(kinds, ["service", "record", "stream"]);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn diamond_import_loads_shared_file_once() {
        let dir = tmp("diamond");
        write(&dir, "shared.ciac", "record Video { id: Uuid; }\n");
        write(
            &dir,
            "a.ciac",
            "import \"shared.ciac\";\nstream A: Video;\n",
        );
        write(
            &dir,
            "b.ciac",
            "import \"shared.ciac\";\nstream B: Video;\n",
        );
        let entry = write(
            &dir,
            "entry.ciac",
            "service S;\nimport \"a.ciac\";\nimport \"b.ciac\";\n",
        );

        let mut sources = SourceMap::new();
        let mut diags = Diagnostics::new();
        let program = load(&entry, &mut sources, &mut diags).expect("loads");

        assert!(diags.is_empty(), "unexpected: {:?}", diags.codes());
        let record_count = program
            .items
            .iter()
            .filter(|item| matches!(item, Item::Record(_)))
            .count();
        assert_eq!(
            record_count, 1,
            "shared.ciac must be spliced in exactly once"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn self_import_is_a_cycle() {
        let dir = tmp("self-cycle");
        let entry = write(&dir, "entry.ciac", "import \"entry.ciac\";\n");

        let mut sources = SourceMap::new();
        let mut diags = Diagnostics::new();
        let program = load(&entry, &mut sources, &mut diags).expect("loads without an io error");

        assert!(program.items.is_empty());
        assert_eq!(diags.codes(), vec![ErrorCode::ImportCycle]);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn two_file_cycle_is_detected() {
        let dir = tmp("two-file-cycle");
        write(&dir, "a.ciac", "import \"b.ciac\";\n");
        let entry = write(&dir, "b.ciac", "import \"a.ciac\";\n");

        let mut sources = SourceMap::new();
        let mut diags = Diagnostics::new();
        let program = load(&entry, &mut sources, &mut diags).expect("loads without an io error");

        assert!(program.items.is_empty());
        assert_eq!(diags.codes(), vec![ErrorCode::ImportCycle]);

        std::fs::remove_dir_all(&dir).ok();
    }
}
