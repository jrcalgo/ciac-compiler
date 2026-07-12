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
use crate::registry;
use ciac_diagnostics::{Diagnostic, Diagnostics, ErrorCode, SourceMap};
use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};

/// The `std` blueprint library (v0.8 M3), embedded at compile time —
/// `import "std/<name>.ciac";` resolves against this table instead of
/// the filesystem, a reserved namespace independent of the user's own
/// directory layout or working directory.
const STD_BLUEPRINTS: &[(&str, &str)] = &[
    ("std/crud.ciac", include_str!("../std/crud.ciac")),
    ("std/webhook.ciac", include_str!("../std/webhook.ciac")),
    (
        "std/rate-limited-api.ciac",
        include_str!("../std/rate-limited-api.ciac"),
    ),
];

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
            Item::Import(import) if import.path.starts_with("std/") => {
                items.extend(resolve_std(&import.path, sources, diags, loaded, stack)?);
            }
            Item::Import(import) if import.path.starts_with("registry:") => {
                items.extend(resolve_registry(
                    &import.path,
                    sources,
                    diags,
                    loaded,
                    stack,
                )?);
            }
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

/// Resolves a `std/` import against [`STD_BLUEPRINTS`] instead of the
/// filesystem — never touches `std::fs`, so it works regardless of the
/// caller's cwd or directory layout. The virtual path string itself
/// (e.g. `"std/crud.ciac"`) is used as the `loaded`/`stack` dedup key;
/// it can never collide with a real file's key since [`resolve_file`]
/// always canonicalizes those to absolute paths first.
fn resolve_std(
    path: &str,
    sources: &mut SourceMap,
    diags: &mut Diagnostics,
    loaded: &mut BTreeSet<PathBuf>,
    stack: &mut Vec<PathBuf>,
) -> io::Result<Vec<Item>> {
    let key = PathBuf::from(path);

    if loaded.contains(&key) {
        return Ok(Vec::new());
    }
    if let Some(pos) = stack.iter().position(|p| p == &key) {
        let cycle = stack[pos..]
            .iter()
            .chain(std::iter::once(&key))
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(" -> ");
        diags.push(Diagnostic::new(
            ErrorCode::ImportCycle,
            format!("import cycle: {cycle}"),
        ));
        return Ok(Vec::new());
    }

    let src = STD_BLUEPRINTS
        .iter()
        .find(|(name, _)| *name == path)
        .map(|(_, src)| *src)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("no such std blueprint '{path}'"),
            )
        })?;

    let file_id = sources.add_file(path.to_string(), src.to_string());
    let program = parse(src, file_id, diags);

    stack.push(key.clone());
    let mut items = Vec::with_capacity(program.items.len());
    for item in program.items {
        match item {
            Item::Import(import) if import.path.starts_with("std/") => {
                items.extend(resolve_std(&import.path, sources, diags, loaded, stack)?);
            }
            Item::Import(import) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "std blueprint '{path}' cannot import a real file ({})",
                        import.path
                    ),
                ));
            }
            other => items.push(other),
        }
    }
    stack.pop();
    loaded.insert(key);
    Ok(items)
}

/// Resolves a `registry:` import (v0.12 M3): fetch-or-cache via
/// [`registry::resolve`], then splice exactly like any other import.
/// The spec string is the dedup/cycle key — like `std/` keys, it can
/// never collide with a canonicalized real path. Fetched content may
/// import `std/` blueprints or further `registry:` specs, but not
/// local files: a remote blueprint has no local directory to resolve
/// them against.
fn resolve_registry(
    spec: &str,
    sources: &mut SourceMap,
    diags: &mut Diagnostics,
    loaded: &mut BTreeSet<PathBuf>,
    stack: &mut Vec<PathBuf>,
) -> io::Result<Vec<Item>> {
    let key = PathBuf::from(spec);

    if loaded.contains(&key) {
        return Ok(Vec::new());
    }
    if let Some(pos) = stack.iter().position(|p| p == &key) {
        let cycle = stack[pos..]
            .iter()
            .chain(std::iter::once(&key))
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(" -> ");
        diags.push(Diagnostic::new(
            ErrorCode::ImportCycle,
            format!("import cycle: {cycle}"),
        ));
        return Ok(Vec::new());
    }

    let src = registry::resolve(spec)?;
    let file_id = sources.add_file(spec.to_string(), src.clone());
    let program = parse(&src, file_id, diags);

    stack.push(key.clone());
    let mut items = Vec::with_capacity(program.items.len());
    for item in program.items {
        match item {
            Item::Import(import) if import.path.starts_with("std/") => {
                items.extend(resolve_std(&import.path, sources, diags, loaded, stack)?);
            }
            Item::Import(import) if import.path.starts_with("registry:") => {
                items.extend(resolve_registry(
                    &import.path,
                    sources,
                    diags,
                    loaded,
                    stack,
                )?);
            }
            Item::Import(import) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "registry import '{spec}' cannot import a local file ({}); \
                         remote blueprints may only import `std/` or other \
                         `registry:` blueprints",
                        import.path
                    ),
                ));
            }
            other => items.push(other),
        }
    }
    stack.pop();
    loaded.insert(key);
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

    #[test]
    fn std_import_resolves_without_touching_the_filesystem() {
        let dir = tmp("std-import");
        let entry = write(
            &dir,
            "entry.ciac",
            "import \"std/crud.ciac\";\nrecord Video { id: Uuid; }\n",
        );

        let mut sources = SourceMap::new();
        let mut diags = Diagnostics::new();
        let program = load(&entry, &mut sources, &mut diags).expect("loads");

        assert!(diags.is_empty(), "unexpected: {:?}", diags.codes());
        assert!(
            program
                .items
                .iter()
                .any(|item| matches!(item, Item::Blueprint(_))),
            "std/crud.ciac's Crud blueprint should be spliced in"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn unknown_std_blueprint_is_a_clear_error() {
        let dir = tmp("unknown-std");
        let entry = write(&dir, "entry.ciac", "import \"std/nope.ciac\";\n");

        let mut sources = SourceMap::new();
        let mut diags = Diagnostics::new();
        let err = load(&entry, &mut sources, &mut diags).expect_err("no such std blueprint");

        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        assert!(err.to_string().contains("std/nope.ciac"), "{err}");

        std::fs::remove_dir_all(&dir).ok();
    }
}
