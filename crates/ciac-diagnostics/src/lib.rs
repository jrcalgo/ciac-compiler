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

/// A single textual edit: replace the byte range `span` with
/// `replacement`. `span.start == span.end` is a pure insertion.
#[derive(Debug, Clone, Serialize)]
pub struct Edit {
    pub span: Span,
    pub replacement: String,
}

/// An applyable fix for a diagnostic (v0.15 M7): a human-readable
/// title plus the edits it takes. `ciac check` only ever *offers*
/// fixes -- an editor's quick-fix or an agent's check -> apply ->
/// re-check loop is what actually applies one.
#[derive(Debug, Clone, Serialize)]
pub struct Fix {
    pub title: String,
    pub edits: Vec<Edit>,
}

impl Fix {
    /// Applies every edit to `src`, which must be the single source
    /// file every edit's span points into -- every v0.15 M7 mechanical
    /// fix is single-file, so multi-file application isn't needed yet.
    /// Edits are applied in descending `span.start` order so an
    /// earlier edit's offsets stay valid while a later one is spliced.
    pub fn apply(&self, src: &str) -> String {
        let mut edits: Vec<&Edit> = self.edits.iter().collect();
        edits.sort_by_key(|edit| std::cmp::Reverse(edit.span.start));
        let mut out = src.to_owned();
        for edit in edits {
            out.replace_range(
                edit.span.start as usize..edit.span.end as usize,
                &edit.replacement,
            );
        }
        out
    }
}

/// A single structured compiler diagnostic.
#[derive(Debug, Clone, Serialize)]
pub struct Diagnostic {
    pub code: ErrorCode,
    pub severity: Severity,
    pub message: String,
    pub labels: Vec<Label>,
    pub help: Option<String>,
    pub fixes: Vec<Fix>,
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
            fixes: Vec::new(),
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

    /// Offers an applyable fix (v0.15 M7). Never applied automatically.
    #[must_use]
    pub fn with_fix(mut self, fix: Fix) -> Self {
        self.fixes.push(fix);
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
