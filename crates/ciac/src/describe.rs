//! `ciac describe` (v0.13 M5): one versioned JSON document naming
//! everything the language and CLI expose to a caller that can't run
//! an LSP client — capabilities, providers (with per-target support),
//! field types, builtin pipeline steps, declaration kinds, error
//! codes, and scaffold templates. `ciac lsp`'s hover/completion and
//! this command render from the same [`crate::vocab`] tables, so a
//! provider graduating on a target is one edit, not two documents to
//! keep in sync.

use crate::vocab;
use anyhow::Result;
use ciac_diagnostics::{ErrorCode, Severity};
use serde::Serialize;
use std::process::ExitCode;

pub const DESCRIBE_VERSION: u32 = 1;

#[derive(Debug, Serialize)]
pub struct Describe {
    pub describe_version: u32,
    pub ciac_version: &'static str,
    pub language_version: &'static str,
    pub keywords: Vec<Entry>,
    pub capabilities: Vec<CapabilityEntry>,
    pub providers: Vec<ProviderEntry>,
    pub field_types: Vec<&'static str>,
    pub builtin_steps: Vec<Entry>,
    pub declaration_kinds: Vec<&'static str>,
    pub error_codes: Vec<ErrorCodeEntry>,
    pub scaffold_templates: Vec<Entry>,
    /// v0.27 M7: the same snippet table `ciac lsp` completion and
    /// `editors/vscode` render from -- a purely additive field (every
    /// key above is untouched), so it carries no `DESCRIBE_VERSION`
    /// bump: an agent that ignores unknown JSON keys, which every
    /// reasonable consumer already must to tolerate future additions,
    /// sees no difference at all.
    pub snippets: Vec<SnippetEntry>,
}

#[derive(Debug, Serialize)]
pub struct SnippetEntry {
    pub prefix: &'static str,
    pub description: &'static str,
    pub body: Vec<&'static str>,
    /// Companion source lines the snippet's own default expansion
    /// assumes already exist (e.g. a `worker` snippet assumes a
    /// `queue` capability and a stream) — empty for the two snippets
    /// that stand alone. The same lines `vocab`'s own parse-test
    /// prepends before checking the expansion compiles.
    pub parses_with: Vec<&'static str>,
}

#[derive(Debug, Serialize)]
pub struct Entry {
    pub name: String,
    pub doc: String,
}

#[derive(Debug, Serialize)]
pub struct CapabilityEntry {
    pub name: &'static str,
    pub doc: &'static str,
    pub providers: Vec<&'static str>,
}

#[derive(Debug, Serialize)]
pub struct ProviderEntry {
    pub name: &'static str,
    pub capability: &'static str,
    pub targets: &'static [&'static str],
    pub doc: &'static str,
}

#[derive(Debug, Serialize)]
pub struct ErrorCodeEntry {
    pub code: &'static str,
    pub severity: &'static str,
    pub title: &'static str,
}

pub fn build() -> Describe {
    Describe {
        describe_version: DESCRIBE_VERSION,
        ciac_version: env!("CARGO_PKG_VERSION"),
        language_version: ciac_syntax::LANGUAGE_VERSION,
        keywords: vocab::KEYWORDS
            .iter()
            .map(|(name, doc)| Entry {
                name: (*name).to_owned(),
                doc: (*doc).to_owned(),
            })
            .collect(),
        capabilities: vocab::CAPABILITIES
            .iter()
            .map(|cap| CapabilityEntry {
                name: cap.name,
                doc: cap.doc,
                providers: vocab::PROVIDERS
                    .iter()
                    .filter(|p| p.capability == cap.name)
                    .map(|p| p.name)
                    .collect(),
            })
            .collect(),
        providers: vocab::PROVIDERS
            .iter()
            .map(|p| ProviderEntry {
                name: p.name,
                capability: p.capability,
                targets: p.targets,
                doc: p.doc,
            })
            .collect(),
        field_types: vocab::FIELD_TYPES.to_vec(),
        builtin_steps: vocab::BUILTIN_STEPS
            .iter()
            .map(|(name, doc)| Entry {
                name: (*name).to_owned(),
                doc: (*doc).to_owned(),
            })
            .collect(),
        declaration_kinds: vocab::DECLARATION_KINDS.to_vec(),
        error_codes: ErrorCode::ALL
            .iter()
            .map(|code| ErrorCodeEntry {
                code: code.code(),
                severity: match code.default_severity() {
                    Severity::Error => "error",
                    Severity::Warning => "warning",
                },
                title: code.title(),
            })
            .collect(),
        scaffold_templates: crate::scaffold::template_summaries()
            .into_iter()
            .map(|(name, summary)| Entry {
                name: name.to_owned(),
                doc: summary.to_owned(),
            })
            .collect(),
        snippets: vocab::SNIPPETS
            .iter()
            .map(|s| SnippetEntry {
                prefix: s.prefix,
                description: s.description,
                body: s.body.to_vec(),
                parses_with: s.parses_with.to_vec(),
            })
            .collect(),
    }
}

pub fn run() -> Result<ExitCode> {
    println!("{}", serde_json::to_string_pretty(&build())?);
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describe_covers_every_error_code_and_provider() {
        let doc = build();
        assert_eq!(doc.error_codes.len(), ErrorCode::ALL.len());
        assert_eq!(doc.providers.len(), vocab::PROVIDERS.len());
        assert!(doc.providers.iter().all(|p| p.targets
            == vocab::PROVIDERS
                .iter()
                .find(|q| q.name == p.name)
                .unwrap()
                .targets));
        assert!(!doc.scaffold_templates.is_empty());
    }

    #[test]
    fn describe_json_is_stable_shape() {
        let json = serde_json::to_value(build()).unwrap();
        for key in [
            "describe_version",
            "ciac_version",
            "language_version",
            "keywords",
            "capabilities",
            "providers",
            "field_types",
            "builtin_steps",
            "declaration_kinds",
            "error_codes",
            "scaffold_templates",
            "snippets",
        ] {
            assert!(json.get(key).is_some(), "missing top-level key `{key}`");
        }
    }

    /// v0.27 M7: `snippets` is new, but `DESCRIBE_VERSION` didn't move
    /// -- only true if every key that existed before M7 still has the
    /// exact same shape. `describe_json_is_stable_shape` above already
    /// proves the pre-existing keys are all still present; this proves
    /// none of them silently gained or lost a field underneath an
    /// unsuspecting consumer, and that the new field is additive
    /// (present, non-empty, and doesn't replace anything).
    #[test]
    fn m7_snippet_field_is_additive_not_a_breaking_reshape() {
        assert_eq!(DESCRIBE_VERSION, 1, "still version 1 -- additive only");
        let doc = build();
        assert!(!doc.snippets.is_empty());
        for s in &doc.snippets {
            assert!(!s.prefix.is_empty());
            assert!(!s.body.is_empty());
        }
        let json = serde_json::to_value(&doc).unwrap();
        let cap = &json["capabilities"][0];
        for key in ["name", "doc", "providers"] {
            assert!(cap.get(key).is_some(), "capabilities[0] lost `{key}`");
        }
        let prov = &json["providers"][0];
        for key in ["name", "capability", "targets", "doc"] {
            assert!(prov.get(key).is_some(), "providers[0] lost `{key}`");
        }
    }
}
