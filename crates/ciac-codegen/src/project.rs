use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io;
use std::path::{Component as PathComponent, Path};

/// Regeneration ownership role for a generated file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum FileRole {
    /// Compiler-owned wiring. Regeneration may rewrite it when unchanged.
    Owned,
    /// Generated seed owned by the user after first write.
    Seeded,
    /// A migration file (v0.27 M9): write-once like `Seeded`, but its
    /// `OrphanLeft` state on every later build is the expected,
    /// permanent steady state once the schema stops changing — not a
    /// stale scaffold the user should investigate — so regeneration
    /// does not warn about it (see `regen::RegenEntry::is_warning`).
    Migration,
}

#[derive(Debug, Clone)]
pub struct GeneratedFile {
    pub content: String,
    pub role: FileRole,
}

/// An in-memory generated file tree.
///
/// Paths are relative, `/`-separated, and validated against traversal.
/// Files are stored in a [`BTreeMap`] so iteration — and therefore
/// everything downstream: snapshots, archives, writes — is deterministic.
#[derive(Debug, Default)]
pub struct GeneratedProject {
    files: BTreeMap<String, GeneratedFile>,
    /// Human-oriented post-generation notes (next steps, caveats).
    pub notes: Vec<String>,
}

impl GeneratedProject {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a file. Panics on invalid (absolute/traversing/duplicate)
    /// paths: those are compiler bugs, not user errors.
    pub fn add_file(&mut self, path: impl Into<String>, content: impl Into<String>) {
        self.add_file_with_role(path, content, FileRole::Owned);
    }

    /// Adds a generated-once seed file. Regeneration preserves the on-disk
    /// copy and writes sidecars when the seed changes.
    pub fn add_seeded_file(&mut self, path: impl Into<String>, content: impl Into<String>) {
        self.add_file_with_role(path, content, FileRole::Seeded);
    }

    /// Adds a migration file. Same write-once semantics as
    /// [`Self::add_seeded_file`], tagged separately so regeneration
    /// knows the later "not regenerated this time" state is expected
    /// and permanent rather than a stale scaffold to flag.
    pub fn add_migration_file(&mut self, path: impl Into<String>, content: impl Into<String>) {
        self.add_file_with_role(path, content, FileRole::Migration);
    }

    fn add_file_with_role(
        &mut self,
        path: impl Into<String>,
        content: impl Into<String>,
        role: FileRole,
    ) {
        let path = path.into();
        assert!(
            is_safe_relative(&path),
            "backend produced unsafe path: {path}"
        );
        let previous = self.files.insert(
            path.clone(),
            GeneratedFile {
                content: content.into(),
                role,
            },
        );
        assert!(previous.is_none(), "backend wrote {path} twice");
    }

    pub fn files(&self) -> impl Iterator<Item = (&str, &str)> {
        self.files
            .iter()
            .map(|(p, f)| (p.as_str(), f.content.as_str()))
    }

    pub fn files_with_roles(&self) -> impl Iterator<Item = (&str, &str, FileRole)> {
        self.files
            .iter()
            .map(|(p, f)| (p.as_str(), f.content.as_str(), f.role))
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    pub fn get(&self, path: &str) -> Option<&str> {
        self.files.get(path).map(|f| f.content.as_str())
    }

    pub fn role(&self, path: &str) -> Option<FileRole> {
        self.files.get(path).map(|f| f.role)
    }

    /// Writes the tree under `root`, creating directories as needed.
    ///
    /// A file named exactly `mvnw` (the Maven wrapper shell script
    /// Java's own `ciac-backend-java` vendors) gets its executable bit
    /// set on Unix after writing — found live (`25UpdatePlan.md` M8):
    /// `std::fs::write` never preserves or sets any permission bit, so
    /// every prior "`./mvnw -q -B verify` passes live" claim this arc
    /// made was true only because that command was run by hand after a
    /// manual `chmod +x`, never through `ciac verify`/`ciac build`
    /// itself — `TargetInfo::validate`'s own `Command::new("./mvnw")`
    /// spawn failed with "Permission denied" on a freshly generated
    /// tree until this fix. No other target ships an executable
    /// wrapper script of its own, so this stays a generic "a file
    /// literally named `mvnw`" rule rather than a per-target special
    /// case threaded through the `Backend` trait.
    pub fn write_to(&self, root: &Path) -> io::Result<()> {
        for (rel, file) in &self.files {
            let path = root.join(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, &file.content)?;
            #[cfg(unix)]
            if path.file_name().and_then(|n| n.to_str()) == Some("mvnw") {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
            }
        }
        Ok(())
    }
}

fn is_safe_relative(path: &str) -> bool {
    !path.is_empty()
        && Path::new(path)
            .components()
            .all(|c| matches!(c, PathComponent::Normal(_)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iteration_is_sorted() {
        let mut project = GeneratedProject::new();
        project.add_file("b.txt", "b");
        project.add_file("a/z.txt", "z");
        project.add_file("a.txt", "a");
        let paths: Vec<&str> = project.files().map(|(p, _)| p).collect();
        assert_eq!(paths, ["a.txt", "a/z.txt", "b.txt"]);
    }

    #[test]
    #[should_panic(expected = "unsafe path")]
    fn rejects_traversal() {
        GeneratedProject::new().add_file("../evil", "x");
    }

    #[test]
    #[should_panic(expected = "twice")]
    fn rejects_duplicate_paths() {
        let mut project = GeneratedProject::new();
        project.add_file("a", "1");
        project.add_file("a", "2");
    }

    #[test]
    fn records_seeded_roles() {
        let mut project = GeneratedProject::new();
        project.add_file("owned.txt", "owned");
        project.add_seeded_file("seeded.txt", "seeded");
        assert_eq!(project.role("owned.txt"), Some(FileRole::Owned));
        assert_eq!(project.role("seeded.txt"), Some(FileRole::Seeded));
    }

    #[test]
    fn writes_tree_to_disk() {
        let dir = std::env::temp_dir().join(format!("ciac-test-{}", std::process::id()));
        let mut project = GeneratedProject::new();
        project.add_file("src/main.py", "print('hi')\n");
        project.write_to(&dir).expect("write succeeds");
        let written = std::fs::read_to_string(dir.join("src/main.py")).expect("file exists");
        assert_eq!(written, "print('hi')\n");
        std::fs::remove_dir_all(&dir).ok();
    }
}
