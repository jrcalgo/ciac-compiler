use std::collections::BTreeMap;
use std::io;
use std::path::{Component as PathComponent, Path};

/// An in-memory generated file tree.
///
/// Paths are relative, `/`-separated, and validated against traversal.
/// Files are stored in a [`BTreeMap`] so iteration — and therefore
/// everything downstream: snapshots, archives, writes — is deterministic.
#[derive(Debug, Default)]
pub struct GeneratedProject {
    files: BTreeMap<String, String>,
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
        let path = path.into();
        assert!(
            is_safe_relative(&path),
            "backend produced unsafe path: {path}"
        );
        let previous = self.files.insert(path.clone(), content.into());
        assert!(previous.is_none(), "backend wrote {path} twice");
    }

    pub fn files(&self) -> impl Iterator<Item = (&str, &str)> {
        self.files.iter().map(|(p, c)| (p.as_str(), c.as_str()))
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    pub fn get(&self, path: &str) -> Option<&str> {
        self.files.get(path).map(String::as_str)
    }

    /// Writes the tree under `root`, creating directories as needed.
    pub fn write_to(&self, root: &Path) -> io::Result<()> {
        for (rel, content) in &self.files {
            let path = root.join(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, content)?;
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
