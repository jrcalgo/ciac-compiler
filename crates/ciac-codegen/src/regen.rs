use crate::manifest::{hash_bytes, hash_content, Manifest};
use crate::{FileRole, GeneratedProject};
use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegenMode {
    Normal,
    Adopt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegenStatus {
    New,
    Unchanged,
    Update,
    Conflict,
    SeededDrift,
    OrphanDelete,
    OrphanLeft,
}

impl RegenStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            RegenStatus::New => "new",
            RegenStatus::Unchanged => "unchanged",
            RegenStatus::Update => "update",
            RegenStatus::Conflict => "conflict",
            RegenStatus::SeededDrift => "seeded-drift",
            RegenStatus::OrphanDelete => "orphan-delete",
            RegenStatus::OrphanLeft => "orphan",
        }
    }
}

#[derive(Debug, Clone)]
pub struct RegenEntry {
    pub path: String,
    pub role: FileRole,
    pub status: RegenStatus,
    pub old_content: Option<String>,
    pub new_content: Option<String>,
    pub sidecar_path: Option<String>,
    /// The freshly generated content's hash -- for `New`/`Unchanged`/
    /// `Update`/`Conflict`/`SeededDrift` this is `plan_regeneration`'s
    /// own `new_hash` local, computed once from the same
    /// `GeneratedProject` a caller like `commands::build_inner` is
    /// about to write to disk; for `OrphanDelete`/`OrphanLeft` (no
    /// generated content at all) it is the manifest's already-recorded
    /// hash for that path instead. Lets `manifest::build_manifest_from_hashes`
    /// build the post-build manifest from this plan directly, rather
    /// than re-hashing every file's content a second time
    /// (`32UpdatePlan.md` M2).
    pub new_hash: String,
}

impl RegenEntry {
    pub fn is_error(&self) -> bool {
        self.status == RegenStatus::Conflict
    }

    pub fn is_warning(&self) -> bool {
        match self.status {
            RegenStatus::SeededDrift => true,
            // A migration's `OrphanLeft` state is the expected,
            // permanent steady state once its schema stops changing
            // (see `FileRole::Migration`) -- not a stale scaffold to
            // flag, so it never warns.
            RegenStatus::OrphanLeft => self.role != FileRole::Migration,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct RegenPlan {
    pub entries: Vec<RegenEntry>,
}

impl RegenPlan {
    pub fn has_errors(&self) -> bool {
        self.entries.iter().any(RegenEntry::is_error)
    }

    pub fn has_warnings(&self) -> bool {
        self.entries.iter().any(RegenEntry::is_warning)
    }

    pub fn changed_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| {
                !matches!(
                    entry.status,
                    RegenStatus::Unchanged | RegenStatus::OrphanLeft
                )
            })
            .count()
    }

    /// The `(path, role, hash)` triples for this plan's current
    /// generated project -- every entry except the orphan arms
    /// (`OrphanDelete`/`OrphanLeft`), which describe files absent from
    /// the project just generated, not present in it. Feeds
    /// `manifest::build_manifest_from_hashes` directly, so a caller
    /// that already ran `plan_regeneration` never re-hashes the same
    /// `GeneratedProject` a second time (`32UpdatePlan.md` M2).
    pub fn manifest_files(&self) -> impl Iterator<Item = (String, FileRole, String)> + '_ {
        self.entries
            .iter()
            .filter(|entry| {
                !matches!(
                    entry.status,
                    RegenStatus::OrphanDelete | RegenStatus::OrphanLeft
                )
            })
            .map(|entry| (entry.path.clone(), entry.role, entry.new_hash.clone()))
    }
}

pub fn plan_regeneration(
    project: &GeneratedProject,
    root: &Path,
    manifest: Option<&Manifest>,
    mode: RegenMode,
) -> io::Result<RegenPlan> {
    let mut entries = Vec::new();
    let mut seen = BTreeSet::new();

    for (rel, generated, role) in project.files_with_roles() {
        seen.insert(rel.to_owned());
        let disk = read_optional(root.join(rel))?;
        let disk_hash = disk.as_deref().map(hash_bytes);
        let new_hash = hash_content(generated);
        let base = manifest.and_then(|m| m.files.get(rel));
        let base_hash = base.map(|entry| entry.hash.as_str());
        let generated_content = generated.to_owned();
        let disk_content = disk
            .as_deref()
            .map(|bytes| String::from_utf8_lossy(bytes).into_owned());

        let entry = match (mode, role, disk_hash.as_deref(), base_hash) {
            (_, _, None, _) => RegenEntry {
                path: rel.to_owned(),
                role,
                status: RegenStatus::New,
                old_content: None,
                new_content: Some(generated_content),
                sidecar_path: None,
                new_hash,
            },
            (RegenMode::Adopt, FileRole::Owned, Some(disk_hash), _) => {
                if disk_hash == new_hash {
                    RegenEntry {
                        path: rel.to_owned(),
                        role,
                        status: RegenStatus::Unchanged,
                        old_content: disk_content,
                        new_content: Some(generated_content),
                        sidecar_path: None,
                        new_hash,
                    }
                } else {
                    sidecar_entry(
                        rel,
                        role,
                        RegenStatus::Conflict,
                        disk_content,
                        generated_content,
                        new_hash,
                    )
                }
            }
            (RegenMode::Adopt, FileRole::Seeded | FileRole::Migration, Some(disk_hash), _) => {
                if disk_hash == new_hash {
                    RegenEntry {
                        path: rel.to_owned(),
                        role,
                        status: RegenStatus::Unchanged,
                        old_content: disk_content,
                        new_content: Some(generated_content),
                        sidecar_path: None,
                        new_hash,
                    }
                } else {
                    sidecar_entry(
                        rel,
                        role,
                        RegenStatus::SeededDrift,
                        disk_content,
                        generated_content,
                        new_hash,
                    )
                }
            }
            (RegenMode::Normal, FileRole::Owned, Some(disk_hash), Some(base_hash)) => {
                if disk_hash == base_hash {
                    if disk_hash == new_hash {
                        unchanged_entry(rel, role, disk_content, generated_content, new_hash)
                    } else {
                        RegenEntry {
                            path: rel.to_owned(),
                            role,
                            status: RegenStatus::Update,
                            old_content: disk_content,
                            new_content: Some(generated_content),
                            sidecar_path: None,
                            new_hash,
                        }
                    }
                } else if disk_hash == new_hash {
                    unchanged_entry(rel, role, disk_content, generated_content, new_hash)
                } else {
                    sidecar_entry(
                        rel,
                        role,
                        RegenStatus::Conflict,
                        disk_content,
                        generated_content,
                        new_hash,
                    )
                }
            }
            (RegenMode::Normal, FileRole::Owned, Some(disk_hash), None) => {
                if disk_hash == new_hash {
                    unchanged_entry(rel, role, disk_content, generated_content, new_hash)
                } else {
                    sidecar_entry(
                        rel,
                        role,
                        RegenStatus::Conflict,
                        disk_content,
                        generated_content,
                        new_hash,
                    )
                }
            }
            (
                RegenMode::Normal,
                FileRole::Seeded | FileRole::Migration,
                Some(disk_hash),
                Some(base_hash),
            ) => {
                if base_hash != new_hash && disk_hash != new_hash {
                    sidecar_entry(
                        rel,
                        role,
                        RegenStatus::SeededDrift,
                        disk_content,
                        generated_content,
                        new_hash,
                    )
                } else {
                    unchanged_entry(rel, role, disk_content, generated_content, new_hash)
                }
            }
            (RegenMode::Normal, FileRole::Seeded | FileRole::Migration, Some(disk_hash), None) => {
                if disk_hash == new_hash {
                    unchanged_entry(rel, role, disk_content, generated_content, new_hash)
                } else {
                    sidecar_entry(
                        rel,
                        role,
                        RegenStatus::SeededDrift,
                        disk_content,
                        generated_content,
                        new_hash,
                    )
                }
            }
        };
        entries.push(entry);
    }

    if mode == RegenMode::Normal {
        if let Some(manifest) = manifest {
            for (rel, entry) in &manifest.files {
                if seen.contains(rel) {
                    continue;
                }
                let disk = read_optional(root.join(rel))?;
                let disk_hash = disk.as_deref().map(hash_bytes);
                let old_content = disk
                    .as_deref()
                    .map(|bytes| String::from_utf8_lossy(bytes).into_owned());
                let status = if entry.role == FileRole::Owned
                    && disk_hash.as_deref() == Some(entry.hash.as_str())
                {
                    RegenStatus::OrphanDelete
                } else {
                    RegenStatus::OrphanLeft
                };
                entries.push(RegenEntry {
                    path: rel.clone(),
                    role: entry.role,
                    status,
                    old_content,
                    new_content: None,
                    sidecar_path: None,
                    new_hash: entry.hash.clone(),
                });
            }
        }
    }

    Ok(RegenPlan { entries })
}

/// How much of a [`RegenPlan`] to write to disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyMode {
    /// Write everything: new/updated files, orphan deletes, and sidecars.
    /// Used when the plan has no conflicts.
    Full,
    /// Write only `.ciac-new` sidecars for `Conflict`/`SeededDrift` entries
    /// and touch nothing else. Used when the plan has errors, so a failed
    /// build doesn't silently rewrite unrelated files without recording the
    /// result in the manifest (see D3 in the v0.6.1 review).
    SidecarsOnly,
}

pub fn apply_regeneration(plan: &RegenPlan, root: &Path, mode: ApplyMode) -> io::Result<()> {
    for entry in &plan.entries {
        match entry.status {
            RegenStatus::New | RegenStatus::Update => {
                if mode == ApplyMode::Full {
                    if let Some(content) = &entry.new_content {
                        write_rel(root, &entry.path, content)?;
                    }
                }
            }
            RegenStatus::Conflict | RegenStatus::SeededDrift => {
                if let (Some(path), Some(content)) = (&entry.sidecar_path, &entry.new_content) {
                    write_rel(root, path, content)?;
                }
            }
            RegenStatus::OrphanDelete => {
                if mode == ApplyMode::Full {
                    let path = root.join(&entry.path);
                    match std::fs::remove_file(path) {
                        Ok(()) => {}
                        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
                        Err(err) => return Err(err),
                    }
                }
            }
            RegenStatus::Unchanged | RegenStatus::OrphanLeft => {}
        }
    }
    Ok(())
}

fn unchanged_entry(
    rel: &str,
    role: FileRole,
    disk_content: Option<String>,
    generated_content: String,
    new_hash: String,
) -> RegenEntry {
    RegenEntry {
        path: rel.to_owned(),
        role,
        status: RegenStatus::Unchanged,
        old_content: disk_content,
        new_content: Some(generated_content),
        sidecar_path: None,
        new_hash,
    }
}

fn sidecar_entry(
    rel: &str,
    role: FileRole,
    status: RegenStatus,
    disk_content: Option<String>,
    generated_content: String,
    new_hash: String,
) -> RegenEntry {
    RegenEntry {
        path: rel.to_owned(),
        role,
        status,
        old_content: disk_content,
        new_content: Some(generated_content),
        sidecar_path: Some(format!("{rel}.ciac-new")),
        new_hash,
    }
}

fn read_optional(path: PathBuf) -> io::Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err),
    }
}

/// Mirrors `GeneratedProject::write_to`'s own `mvnw`-executable-bit fix
/// (25UpdatePlan.md M8) — the incremental regeneration path
/// (`ciac dev`'s own initial generate, and any subsequent full
/// `ciac build`/`ciac diff --apply`) writes new files through here,
/// not through `write_to`, so both paths need the identical fix or a
/// project generated by one path and not the other would silently
/// diverge in whether `./mvnw` is runnable.
fn write_rel(root: &Path, rel: &str, content: &str) -> io::Result<()> {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, content)?;
    #[cfg(unix)]
    if path.file_name().and_then(|n| n.to_str()) == Some("mvnw") {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::build_manifest;

    #[test]
    fn clean_owned_file_updates() {
        let dir = temp_dir("clean-owned-file-updates");
        let mut old = GeneratedProject::new();
        old.add_file("a.txt", "old");
        old.write_to(&dir).unwrap();
        let manifest = build_manifest(&old, "0.6.0", "1.0.0", "src", "python");

        let mut new = GeneratedProject::new();
        new.add_file("a.txt", "new");
        let plan = plan_regeneration(&new, &dir, Some(&manifest), RegenMode::Normal).unwrap();
        assert_eq!(plan.entries[0].status, RegenStatus::Update);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn modified_owned_file_conflicts() {
        let dir = temp_dir("modified-owned-file-conflicts");
        let mut old = GeneratedProject::new();
        old.add_file("a.txt", "old");
        old.write_to(&dir).unwrap();
        let manifest = build_manifest(&old, "0.6.0", "1.0.0", "src", "python");
        std::fs::write(dir.join("a.txt"), "user").unwrap();

        let mut new = GeneratedProject::new();
        new.add_file("a.txt", "new");
        let plan = plan_regeneration(&new, &dir, Some(&manifest), RegenMode::Normal).unwrap();
        assert_eq!(plan.entries[0].status, RegenStatus::Conflict);
        assert!(plan.has_errors());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn sidecars_only_writes_sidecars_but_nothing_else() {
        let dir = temp_dir("sidecars-only-writes-sidecars-but-nothing-else");
        let mut old = GeneratedProject::new();
        old.add_file("a.txt", "old-a");
        old.add_file("b.txt", "old-b");
        old.write_to(&dir).unwrap();
        let manifest = build_manifest(&old, "0.6.0", "1.0.0", "src", "python");
        std::fs::write(dir.join("a.txt"), "user-edit").unwrap();

        let mut new = GeneratedProject::new();
        new.add_file("a.txt", "new-a");
        new.add_file("b.txt", "new-b");
        new.add_file("c.txt", "new-c");
        let plan = plan_regeneration(&new, &dir, Some(&manifest), RegenMode::Normal).unwrap();
        assert!(plan.has_errors());

        apply_regeneration(&plan, &dir, ApplyMode::SidecarsOnly).unwrap();

        assert_eq!(
            std::fs::read_to_string(dir.join("a.txt")).unwrap(),
            "user-edit",
            "conflicting owned file must not be overwritten"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("a.txt.ciac-new")).unwrap(),
            "new-a"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("b.txt")).unwrap(),
            "old-b",
            "unrelated update must not be applied on a failed build"
        );
        assert!(
            !dir.join("c.txt").exists(),
            "unrelated new file must not be written on a failed build"
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn failed_build_does_not_poison_the_next_diff() {
        let dir = temp_dir("failed-build-does-not-poison-the-next-diff");

        let v1 = {
            let mut p = GeneratedProject::new();
            p.add_file("app/main.py", "v1-main");
            p.add_file("app/schemas.py", "v1-schemas");
            p
        };
        v1.write_to(&dir).unwrap();
        let manifest_v1 = build_manifest(&v1, "0.6.0", "1.0.0", "src-v1", "python");

        // User edits one owned file.
        std::fs::write(dir.join("app/main.py"), "user-edit").unwrap();

        // v2 build conflicts on main.py; applied as SidecarsOnly, so the
        // manifest is NOT rewritten and schemas.py is left untouched.
        let v2 = {
            let mut p = GeneratedProject::new();
            p.add_file("app/main.py", "v2-main");
            p.add_file("app/schemas.py", "v2-schemas");
            p
        };
        let plan_v2 = plan_regeneration(&v2, &dir, Some(&manifest_v1), RegenMode::Normal).unwrap();
        assert!(plan_v2.has_errors());
        apply_regeneration(&plan_v2, &dir, ApplyMode::SidecarsOnly).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("app/schemas.py")).unwrap(),
            "v1-schemas",
            "schemas.py must be untouched by the failed v2 build"
        );

        // v3 diff, still against the v1 manifest (since v2 never wrote one),
        // must report the untouched schemas.py as an update, not a false
        // conflict.
        let v3 = {
            let mut p = GeneratedProject::new();
            p.add_file("app/main.py", "v3-main");
            p.add_file("app/schemas.py", "v3-schemas");
            p
        };
        let plan_v3 = plan_regeneration(&v3, &dir, Some(&manifest_v1), RegenMode::Normal).unwrap();
        let schemas_entry = plan_v3
            .entries
            .iter()
            .find(|e| e.path == "app/schemas.py")
            .unwrap();
        assert_eq!(
            schemas_entry.status,
            RegenStatus::Update,
            "schemas.py was never touched by a user and must not be reported as a conflict"
        );

        std::fs::remove_dir_all(dir).ok();
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ciac-regen-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}
