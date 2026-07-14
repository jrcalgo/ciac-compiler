use crate::evolution::RecordSchema;
use crate::migrations::TableSchema;
use crate::semantic_model::SemanticModel;
use crate::{FileRole, GeneratedProject};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io;
use std::path::Path;

pub const MANIFEST_REL_PATH: &str = ".ciac/manifest.json";

fn first_migration_seq() -> u32 {
    1
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub compiler_version: String,
    pub source_hash: String,
    pub target: String,
    pub files: BTreeMap<String, ManifestFile>,
    /// v0.7 `table` schema as of the last migration, keyed by table
    /// name — the "old" side `ciac-codegen::migrations::diff_schema`
    /// diffs the current program's tables against on the next build.
    /// Defaulted so manifests written before v0.7 M5 still deserialize.
    #[serde(default)]
    pub tables: BTreeMap<String, TableSchema>,
    /// The next migration file's sequence number.
    #[serde(default = "first_migration_seq")]
    pub next_migration_seq: u32,
    /// v0.8 M5: field shape of every record used across a service
    /// boundary as of the last build, keyed by record name — the "old"
    /// side `ciac-codegen::evolution::diff_records` diffs the current
    /// program's boundary records against on the next build. Defaulted
    /// so manifests written before v0.8 M5 still deserialize.
    #[serde(default)]
    pub records: BTreeMap<String, RecordSchema>,
    /// v0.18 M1: the canonical `SemanticModel` produced by this build,
    /// cached for `ciac diff --semantic --out <tree>`'s *advisory*
    /// local comparison mode — never the checked-in baseline generated
    /// CI gates on (`ciac baseline`'s own file), and never advanced by
    /// a failed build. `None` for manifests written before v0.18 M1, or
    /// if a build never reached this point.
    #[serde(default)]
    pub semantic_snapshot: Option<SemanticModel>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestFile {
    pub role: FileRole,
    pub hash: String,
}

pub fn hash_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut out, "{byte:02x}").expect("writing to String cannot fail");
    }
    out
}

pub fn hash_content(content: &str) -> String {
    hash_bytes(content.as_bytes())
}

pub fn build_manifest(
    project: &GeneratedProject,
    compiler_version: impl Into<String>,
    source_hash: impl Into<String>,
    target: impl Into<String>,
) -> Manifest {
    let files = project
        .files_with_roles()
        .map(|(path, content, role)| {
            (
                path.to_owned(),
                ManifestFile {
                    role,
                    hash: hash_content(content),
                },
            )
        })
        .collect();
    Manifest {
        compiler_version: compiler_version.into(),
        source_hash: source_hash.into(),
        target: target.into(),
        files,
        tables: BTreeMap::new(),
        next_migration_seq: first_migration_seq(),
        records: BTreeMap::new(),
        semantic_snapshot: None,
    }
}

pub fn manifest_path(root: &Path) -> std::path::PathBuf {
    root.join(MANIFEST_REL_PATH)
}

pub fn load_manifest(root: &Path) -> io::Result<Manifest> {
    let bytes = std::fs::read(manifest_path(root))?;
    serde_json::from_slice(&bytes).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
}

pub fn write_manifest(root: &Path, manifest: &Manifest) -> io::Result<()> {
    let path = manifest_path(root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(manifest)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    std::fs::write(path, [bytes, b"\n".to_vec()].concat())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_is_sorted_and_stable() {
        let mut project = GeneratedProject::new();
        project.add_file("b.txt", "b");
        project.add_seeded_file("a.txt", "a");
        let manifest = build_manifest(&project, "0.6.0", "src", "python");
        let json = serde_json::to_string_pretty(&manifest).expect("serialize");
        let again = serde_json::to_string_pretty(&manifest).expect("serialize");
        assert_eq!(json, again);
        assert!(json.find("\"a.txt\"").unwrap() < json.find("\"b.txt\"").unwrap());
    }
}
