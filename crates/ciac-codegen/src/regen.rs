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
}

impl RegenEntry {
    pub fn is_error(&self) -> bool {
        self.status == RegenStatus::Conflict
    }

    pub fn is_warning(&self) -> bool {
        matches!(
            self.status,
            RegenStatus::SeededDrift | RegenStatus::OrphanLeft
        )
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
        let disk_hash = disk.as_deref().map(|bytes| hash_bytes(bytes));
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
                    }
                } else {
                    sidecar_entry(
                        rel,
                        role,
                        RegenStatus::Conflict,
                        disk_content,
                        generated_content,
                    )
                }
            }
            (RegenMode::Adopt, FileRole::Seeded, Some(disk_hash), _) => {
                if disk_hash == new_hash {
                    RegenEntry {
                        path: rel.to_owned(),
                        role,
                        status: RegenStatus::Unchanged,
                        old_content: disk_content,
                        new_content: Some(generated_content),
                        sidecar_path: None,
                    }
                } else {
                    sidecar_entry(
                        rel,
                        role,
                        RegenStatus::SeededDrift,
                        disk_content,
                        generated_content,
                    )
                }
            }
            (RegenMode::Normal, FileRole::Owned, Some(disk_hash), Some(base_hash)) => {
                if disk_hash == base_hash {
                    if disk_hash == new_hash {
                        unchanged_entry(rel, role, disk_content, generated_content)
                    } else {
                        RegenEntry {
                            path: rel.to_owned(),
                            role,
                            status: RegenStatus::Update,
                            old_content: disk_content,
                            new_content: Some(generated_content),
                            sidecar_path: None,
                        }
                    }
                } else if disk_hash == new_hash {
                    unchanged_entry(rel, role, disk_content, generated_content)
                } else {
                    sidecar_entry(
                        rel,
                        role,
                        RegenStatus::Conflict,
                        disk_content,
                        generated_content,
                    )
                }
            }
            (RegenMode::Normal, FileRole::Owned, Some(disk_hash), None) => {
                if disk_hash == new_hash {
                    unchanged_entry(rel, role, disk_content, generated_content)
                } else {
                    sidecar_entry(
                        rel,
                        role,
                        RegenStatus::Conflict,
                        disk_content,
                        generated_content,
                    )
                }
            }
            (RegenMode::Normal, FileRole::Seeded, Some(disk_hash), Some(base_hash)) => {
                if base_hash != new_hash && disk_hash != new_hash {
                    sidecar_entry(
                        rel,
                        role,
                        RegenStatus::SeededDrift,
                        disk_content,
                        generated_content,
                    )
                } else {
                    unchanged_entry(rel, role, disk_content, generated_content)
                }
            }
            (RegenMode::Normal, FileRole::Seeded, Some(disk_hash), None) => {
                if disk_hash == new_hash {
                    unchanged_entry(rel, role, disk_content, generated_content)
                } else {
                    sidecar_entry(
                        rel,
                        role,
                        RegenStatus::SeededDrift,
                        disk_content,
                        generated_content,
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
                let disk_hash = disk.as_deref().map(|bytes| hash_bytes(bytes));
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
                });
            }
        }
    }

    Ok(RegenPlan { entries })
}

pub fn apply_regeneration(plan: &RegenPlan, root: &Path) -> io::Result<()> {
    for entry in &plan.entries {
        match entry.status {
            RegenStatus::New | RegenStatus::Update => {
                if let Some(content) = &entry.new_content {
                    write_rel(root, &entry.path, content)?;
                }
            }
            RegenStatus::Conflict | RegenStatus::SeededDrift => {
                if let (Some(path), Some(content)) = (&entry.sidecar_path, &entry.new_content) {
                    write_rel(root, path, content)?;
                }
            }
            RegenStatus::OrphanDelete => {
                let path = root.join(&entry.path);
                match std::fs::remove_file(path) {
                    Ok(()) => {}
                    Err(err) if err.kind() == io::ErrorKind::NotFound => {}
                    Err(err) => return Err(err),
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
) -> RegenEntry {
    RegenEntry {
        path: rel.to_owned(),
        role,
        status: RegenStatus::Unchanged,
        old_content: disk_content,
        new_content: Some(generated_content),
        sidecar_path: None,
    }
}

fn sidecar_entry(
    rel: &str,
    role: FileRole,
    status: RegenStatus,
    disk_content: Option<String>,
    generated_content: String,
) -> RegenEntry {
    RegenEntry {
        path: rel.to_owned(),
        role,
        status,
        old_content: disk_content,
        new_content: Some(generated_content),
        sidecar_path: Some(format!("{rel}.ciac-new")),
    }
}

fn read_optional(path: PathBuf) -> io::Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err),
    }
}

fn write_rel(root: &Path, rel: &str, content: &str) -> io::Result<()> {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, content)
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
        let manifest = build_manifest(&old, "0.6.0", "src", "python");

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
        let manifest = build_manifest(&old, "0.6.0", "src", "python");
        std::fs::write(dir.join("a.txt"), "user").unwrap();

        let mut new = GeneratedProject::new();
        new.add_file("a.txt", "new");
        let plan = plan_regeneration(&new, &dir, Some(&manifest), RegenMode::Normal).unwrap();
        assert_eq!(plan.entries[0].status, RegenStatus::Conflict);
        assert!(plan.has_errors());
        std::fs::remove_dir_all(dir).ok();
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ciac-regen-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}
