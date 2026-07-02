//! Source maps, spans, and structured diagnostics for the CIaC compiler.
//!
//! Every stage of the compiler reports problems by pushing [`Diagnostic`]s
//! into a shared [`Diagnostics`] sink instead of panicking or exiting.
//! Diagnostics carry a stable [`ErrorCode`], a severity, and span-anchored
//! labels; rendering to human-readable output is a separate concern (see
//! [`render`]), so tests can assert on structure rather than strings.

mod code;
pub mod render;
mod source;

pub use code::ErrorCode;
pub use source::{FileId, SourceFile, SourceMap, Span};

use serde::Serialize;

/// How severe a diagnostic is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum Severity {
    Warning,
    Error,
}

/// A message anchored to a region of source code.
#[derive(Debug, Clone, Serialize)]
pub struct Label {
    pub span: Span,
    pub message: String,
}

/// A single structured compiler diagnostic.
#[derive(Debug, Clone, Serialize)]
pub struct Diagnostic {
    pub code: ErrorCode,
    pub severity: Severity,
    pub message: String,
    pub labels: Vec<Label>,
    pub help: Option<String>,
}

impl Diagnostic {
    /// Creates a diagnostic with the default severity of its error code.
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            severity: code.default_severity(),
            message: message.into(),
            labels: Vec::new(),
            help: None,
        }
    }

    #[must_use]
    pub fn with_label(mut self, span: Span, message: impl Into<String>) -> Self {
        self.labels.push(Label {
            span,
            message: message.into(),
        });
        self
    }

    #[must_use]
    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    /// The span of the first label, if any. Used for stable sorting.
    pub fn primary_span(&self) -> Option<Span> {
        self.labels.first().map(|l| l.span)
    }
}

/// Sink that collects diagnostics across all compiler stages.
#[derive(Debug, Default)]
pub struct Diagnostics {
    diags: Vec<Diagnostic>,
}

impl Diagnostics {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, diag: Diagnostic) {
        self.diags.push(diag);
    }

    pub fn has_errors(&self) -> bool {
        self.diags.iter().any(|d| d.severity == Severity::Error)
    }

    pub fn is_empty(&self) -> bool {
        self.diags.is_empty()
    }

    pub fn len(&self) -> usize {
        self.diags.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Diagnostic> {
        self.diags.iter()
    }

    /// Sorts diagnostics by primary span offset for deterministic output.
    pub fn sort(&mut self) {
        self.diags
            .sort_by_key(|d| d.primary_span().map_or(u32::MAX, |s| s.start));
    }

    /// Returns the error codes present, useful for compact test assertions.
    pub fn codes(&self) -> Vec<ErrorCode> {
        self.diags.iter().map(|d| d.code).collect()
    }
}

impl IntoIterator for Diagnostics {
    type Item = Diagnostic;
    type IntoIter = std::vec::IntoIter<Diagnostic>;

    fn into_iter(self) -> Self::IntoIter {
        self.diags.into_iter()
    }
}
