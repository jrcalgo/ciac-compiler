//! v0.10 M3: `--json` structured output. One JSON document on
//! **stdout** per invocation; all human narration stays on stderr, so
//! the two never interleave. The envelope is versioned independently
//! of the crate (`json_version`) — a tool parsing it pins to that, not
//! to ciac's release number.

use ciac_diagnostics::{Diagnostics, Severity, SourceMap};
use serde::{Deserialize, Serialize};

/// v0.15 M7 bumped this from 1: `JsonDiagnostic` gained `fixes`.
pub const JSON_VERSION: u32 = 2;

#[derive(Debug, Serialize)]
pub struct Envelope {
    pub json_version: u32,
    pub command: &'static str,
    pub success: bool,
    pub diagnostics: Vec<JsonDiagnostic>,
    /// `diff --json` only (v0.10 M4): the regeneration plan as data.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entries: Option<Vec<DiffEntry>>,
}

/// One regeneration-plan entry — what a real `ciac build` would do to
/// this path, without having done it.
#[derive(Debug, Serialize)]
pub struct DiffEntry {
    pub path: String,
    /// [`ciac_codegen::regen::RegenStatus::as_str`]'s vocabulary.
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sidecar: Option<String>,
    /// Unified diff text, present only under `--patch` and only when
    /// content actually changed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub patch: Option<String>,
}

/// A [`ciac_diagnostics::Diagnostic`] with every span resolved to
/// file/line/column — the form a tool wants, rather than the byte
/// offsets the compiler tracks internally.
#[derive(Debug, Serialize)]
pub struct JsonDiagnostic {
    /// e.g. `CIAC0010`.
    pub code: &'static str,
    /// `error` | `warning`.
    pub severity: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
    pub labels: Vec<JsonLabel>,
    /// Applyable fixes (v0.15 M7) — mechanical, unambiguous edits only.
    /// Never applied by `ciac check` itself; an editor's quick-fix or
    /// an agent's check → apply → re-check loop applies one.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub fixes: Vec<JsonFix>,
}

#[derive(Debug, Serialize)]
pub struct JsonLabel {
    pub file: String,
    /// 1-based, inclusive.
    pub line: u32,
    pub column: u32,
    pub end_line: u32,
    pub end_column: u32,
    pub message: String,
}

/// A [`ciac_diagnostics::Fix`] with every edit's span resolved to
/// file/line/column, same shape as [`JsonLabel`] (v0.15 M7).
/// `Deserialize` too: `ciac lsp`'s `codeAction` handler round-trips
/// this through an LSP diagnostic's `data` field.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonFix {
    pub title: String,
    pub edits: Vec<JsonEdit>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonEdit {
    pub file: String,
    /// 1-based, inclusive. `line == end_line && column == end_column`
    /// is a pure insertion.
    pub line: u32,
    pub column: u32,
    pub end_line: u32,
    pub end_column: u32,
    pub replacement: String,
}

pub fn envelope(
    command: &'static str,
    success: bool,
    diags: &Diagnostics,
    sources: &SourceMap,
) -> Envelope {
    let diagnostics = diags
        .iter()
        .map(|diag| JsonDiagnostic {
            code: diag.code.into(),
            severity: match diag.severity {
                Severity::Error => "error",
                Severity::Warning => "warning",
            },
            message: diag.message.clone(),
            help: diag.help.clone(),
            labels: diag
                .labels
                .iter()
                .map(|label| {
                    let file = sources.file(label.span.file);
                    let (line, column) = file.line_col(label.span.start);
                    let (end_line, end_column) = file.line_col(label.span.end);
                    JsonLabel {
                        file: file.name.clone(),
                        line,
                        column,
                        end_line,
                        end_column,
                        message: label.message.clone(),
                    }
                })
                .collect(),
            fixes: diag
                .fixes
                .iter()
                .map(|fix| JsonFix {
                    title: fix.title.clone(),
                    edits: fix
                        .edits
                        .iter()
                        .map(|edit| {
                            let file = sources.file(edit.span.file);
                            let (line, column) = file.line_col(edit.span.start);
                            let (end_line, end_column) = file.line_col(edit.span.end);
                            JsonEdit {
                                file: file.name.clone(),
                                line,
                                column,
                                end_line,
                                end_column,
                                replacement: edit.replacement.clone(),
                            }
                        })
                        .collect(),
                })
                .collect(),
        })
        .collect();
    Envelope {
        json_version: JSON_VERSION,
        command,
        success,
        diagnostics,
        entries: None,
    }
}

/// Prints the envelope as the invocation's single stdout document.
pub fn emit(envelope: &Envelope) {
    println!(
        "{}",
        serde_json::to_string(envelope).expect("envelope always serializes")
    );
}
