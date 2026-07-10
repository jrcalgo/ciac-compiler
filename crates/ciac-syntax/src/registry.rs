//! v0.12 M3: registry blueprint imports.
//!
//! `import "registry:<owner>/<repo>/<path>.ciac@<ref>";` resolves to
//! an HTTPS GET of `{base}/{owner}/{repo}/{ref}/{path}.ciac`, where
//! `{base}` is `$CIAC_REGISTRY` (default
//! `https://raw.githubusercontent.com`) — a plain git-hosted
//! directory of `.ciac` files *is* the reference registry; there is
//! no index service, no namespace ownership, no search.
//!
//! Fetched content is cached at
//! `$XDG_CACHE_HOME/ciac/registry/<sha256(url)>.ciac` (falling back
//! to `~/.cache`), and a cache hit never touches the network — pin
//! imports to an immutable ref (a tag or commit, not a branch) and
//! resolution is reproducible and offline after the first fetch.
//!
//! Trust boundary: fetched content is plain `.ciac` source flowing
//! through the identical parse → expansion → validation path as local
//! files and `std/` blueprints. Importing it grants no execution
//! beyond what `ciac build` already does with local source.

use sha2::{Digest, Sha256};
use std::io;
use std::path::PathBuf;
use std::time::Duration;

pub const DEFAULT_REGISTRY: &str = "https://raw.githubusercontent.com";

/// A parsed `registry:` import spec.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RegistrySpec {
    pub owner: String,
    pub repo: String,
    /// Path within the repository, always ending in `.ciac`.
    pub path: String,
    /// Git ref: tag, branch, or commit. Immutable refs make the cache
    /// a permanent, reproducible answer.
    pub git_ref: String,
}

impl RegistrySpec {
    /// The URL this spec resolves to under `base` (no trailing slash
    /// normalization needed by callers).
    pub fn url(&self, base: &str) -> String {
        format!(
            "{}/{}/{}/{}/{}",
            base.trim_end_matches('/'),
            self.owner,
            self.repo,
            self.git_ref,
            self.path
        )
    }
}

/// Parses `registry:<owner>/<repo>/<path>.ciac@<ref>` (the full
/// import string, prefix included).
pub(crate) fn parse_spec(spec: &str) -> Result<RegistrySpec, String> {
    const USAGE: &str = "expected `registry:<owner>/<repo>/<path>.ciac@<ref>`";
    let rest = spec
        .strip_prefix("registry:")
        .ok_or_else(|| format!("not a registry import; {USAGE}"))?;
    let (location, git_ref) = rest
        .rsplit_once('@')
        .ok_or_else(|| format!("missing `@<ref>` (pin a tag or commit); {USAGE}"))?;
    if git_ref.is_empty() {
        return Err(format!("empty ref after `@`; {USAGE}"));
    }
    let mut segments = location.split('/');
    let owner = segments.next().unwrap_or_default();
    let repo = segments.next().unwrap_or_default();
    let path: Vec<&str> = segments.collect();
    if owner.is_empty() || repo.is_empty() || path.is_empty() {
        return Err(format!("need owner, repo, and a file path; {USAGE}"));
    }
    if path.iter().any(|s| s.is_empty() || *s == "." || *s == "..") {
        return Err(format!("path contains empty or relative segments; {USAGE}"));
    }
    let path = path.join("/");
    if !path.ends_with(".ciac") {
        return Err(format!("imported file must end in `.ciac`; {USAGE}"));
    }
    Ok(RegistrySpec {
        owner: owner.to_string(),
        repo: repo.to_string(),
        path,
        git_ref: git_ref.to_string(),
    })
}

/// Resolves a `registry:` import to its source text: parse the spec,
/// answer from the cache when possible, otherwise fetch and cache.
/// All failures come back as `io::Error` with the spec (and URL,
/// where one was formed) in the message — the module loader
/// propagates them through the same unreadable-import path local
/// files use.
pub(crate) fn resolve(spec: &str) -> io::Result<String> {
    let parsed = parse_spec(spec)
        .map_err(|msg| invalid_input(format!("invalid registry import `{spec}`: {msg}")))?;
    let base = std::env::var("CIAC_REGISTRY").unwrap_or_else(|_| DEFAULT_REGISTRY.to_string());
    let url = parsed.url(&base);

    let cache = cache_path(&url);
    if let Some(cache) = &cache {
        if let Ok(source) = std::fs::read_to_string(cache) {
            return Ok(source);
        }
    }

    let source = fetch(spec, &url)?;
    if let Some(cache) = &cache {
        // Best effort: an unwritable cache degrades to re-fetching,
        // never to failure.
        if let Some(parent) = cache.parent() {
            if std::fs::create_dir_all(parent).is_ok() {
                let _ = std::fs::write(cache, &source);
            }
        }
    }
    Ok(source)
}

fn fetch(spec: &str, url: &str) -> io::Result<String> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .build();
    let response = agent.get(url).call().map_err(|err| match err {
        ureq::Error::Status(code, _) => io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "cannot fetch registry import `{spec}`: HTTP {code} from {url} \
                 (check owner/repo/path/ref; set CIAC_REGISTRY to change the base)"
            ),
        ),
        ureq::Error::Transport(t) => io::Error::other(format!(
            "cannot fetch registry import `{spec}` from {url}: {t}"
        )),
    })?;
    response.into_string().map_err(|err| {
        io::Error::new(
            err.kind(),
            format!("cannot read registry import `{spec}` body from {url}: {err}"),
        )
    })
}

/// `$XDG_CACHE_HOME/ciac/registry/<sha256(url)>.ciac`, falling back
/// to `~/.cache`; `None` when no cache root can be determined (the
/// import still works, it just re-fetches).
fn cache_path(url: &str) -> Option<PathBuf> {
    let root = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))?;
    let digest = Sha256::digest(url.as_bytes());
    let mut hex = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(hex, "{byte:02x}");
    }
    Some(
        root.join("ciac")
            .join("registry")
            .join(format!("{hex}.ciac")),
    )
}

fn invalid_input(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_full_spec() {
        let spec = parse_spec("registry:acme/blueprints/notes/crud.ciac@v1.2.0").expect("parses");
        assert_eq!(
            spec,
            RegistrySpec {
                owner: "acme".into(),
                repo: "blueprints".into(),
                path: "notes/crud.ciac".into(),
                git_ref: "v1.2.0".into(),
            }
        );
        assert_eq!(
            spec.url("https://raw.githubusercontent.com/"),
            "https://raw.githubusercontent.com/acme/blueprints/v1.2.0/notes/crud.ciac"
        );
    }

    #[test]
    fn rejects_malformed_specs() {
        for bad in [
            "registry:acme/blueprints/crud.ciac",    // no @ref
            "registry:acme/blueprints/crud.ciac@",   // empty ref
            "registry:acme/crud.ciac@v1",            // no repo/path split
            "registry:acme/blueprints/crud.txt@v1",  // not .ciac
            "registry:acme/blueprints/../x.ciac@v1", // relative segment
            "registry:acme//crud.ciac@v1",           // empty segment
        ] {
            assert!(parse_spec(bad).is_err(), "should reject `{bad}`");
        }
    }

    #[test]
    fn cache_path_is_stable_per_url() {
        let a = cache_path("http://x/a.ciac");
        let b = cache_path("http://x/a.ciac");
        let c = cache_path("http://x/b.ciac");
        assert_eq!(a, b);
        assert_ne!(a, c);
        if let Some(p) = a {
            assert!(p.to_string_lossy().contains("ciac"));
            assert!(p.extension().is_some_and(|e| e == "ciac"));
        }
    }
}
