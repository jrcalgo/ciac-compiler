//! v0.10 M3: `--json` structured output. One JSON document on
//! **stdout** per invocation; all human narration stays on stderr, so
//! the two never interleave. The envelope is versioned independently
//! of the crate (`json_version`) — a tool parsing it pins to that, not
//! to ciac's release number.

use ciac_diagnostics::{Diagnostics, Severity, SourceMap};
use serde::Serialize;

pub const JSON_VERSION: u32 = 1;

#[derive(Debug, Serialize)]
pub struct Envelope {
    pub json_version: u32,
    pub command: &'static str,
    pub success: bool,
    pub diagnostics: Vec<JsonDiagnostic>,
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
        })
        .collect();
    Envelope {
        json_version: JSON_VERSION,
        command,
        success,
        diagnostics,
    }
}

/// Prints the envelope as the invocation's single stdout document.
pub fn emit(envelope: &Envelope) {
    println!(
        "{}",
        serde_json::to_string(envelope).expect("envelope always serializes")
    );
}
