//! Rendering of structured diagnostics to human-readable text.
//!
//! Rendering is behind the [`Render`] trait so the CLI can use rich
//! [`ariadne`] reports while tests use the plain renderer and snapshot
//! stable, color-free output.

use crate::{Diagnostic, Severity, SourceMap};
use std::io::Write;

/// Renders diagnostics to a writer.
pub trait Render {
    fn render(
        &self,
        diag: &Diagnostic,
        sources: &SourceMap,
        out: &mut dyn Write,
    ) -> std::io::Result<()>;
}

/// Compact single-line-per-label renderer with stable output, for tests
/// and machine-oriented logs.
#[derive(Debug, Default)]
pub struct PlainRenderer;

impl Render for PlainRenderer {
    fn render(
        &self,
        diag: &Diagnostic,
        sources: &SourceMap,
        out: &mut dyn Write,
    ) -> std::io::Result<()> {
        let sev = match diag.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        writeln!(out, "{sev}[{}]: {}", diag.code, diag.message)?;
        for label in &diag.labels {
            let file = sources.file(label.span.file);
            let (line, col) = file.line_col(label.span.start);
            writeln!(out, "  --> {}:{line}:{col}: {}", file.name, label.message)?;
        }
        if let Some(help) = &diag.help {
            writeln!(out, "  help: {help}")?;
        }
        Ok(())
    }
}

/// Rich renderer with source excerpts and underlines, used by the CLI.
#[derive(Debug)]
pub struct AriadneRenderer {
    /// Disable colors (e.g. when stderr is not a terminal).
    pub color: bool,
}

impl Render for AriadneRenderer {
    fn render(
        &self,
        diag: &Diagnostic,
        sources: &SourceMap,
        out: &mut dyn Write,
    ) -> std::io::Result<()> {
        use ariadne::{Config, Label, Report, ReportKind};

        let Some(primary) = diag.primary_span() else {
            // No span: fall back to the plain renderer.
            return PlainRenderer.render(diag, sources, out);
        };
        let primary_name = sources.file(primary.file).name.clone();
        let kind = match diag.severity {
            Severity::Error => ReportKind::Error,
            Severity::Warning => ReportKind::Warning,
        };

        let mut report = Report::build(kind, (primary_name, primary.range()))
            .with_config(Config::default().with_color(self.color))
            .with_code(diag.code.code())
            .with_message(&diag.message);
        for label in &diag.labels {
            // A label's span may belong to a *different* file than the
            // primary one (e.g. a cross-file duplicate declaration, v0.8
            // M1) — each label must carry its own file's name, not the
            // primary span's, or ariadne resolves the wrong file's byte
            // offsets against the wrong source text.
            let name = sources.file(label.span.file).name.clone();
            report = report
                .with_label(Label::new((name, label.span.range())).with_message(&label.message));
        }
        if let Some(help) = &diag.help {
            report = report.with_help(help);
        }
        // A multi-file cache: every registered file, keyed by name, so
        // labels pointing at any of them resolve correctly — not just
        // the primary span's file.
        let cache = ariadne::sources(
            sources
                .files()
                .map(|file| (file.name.clone(), file.src.clone())),
        );
        report.finish().write(cache, out)
    }
}

/// Renders every diagnostic in order and returns the result as a string.
pub fn render_all(
    diags: impl IntoIterator<Item = Diagnostic>,
    sources: &SourceMap,
    renderer: &dyn Render,
) -> String {
    let mut buf = Vec::new();
    for diag in diags {
        // Writing to a Vec cannot fail.
        let _ = renderer.render(&diag, sources, &mut buf);
    }
    String::from_utf8_lossy(&buf).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Diagnostic, ErrorCode, Span};

    #[test]
    fn plain_renderer_is_stable() {
        let mut sources = SourceMap::new();
        let file = sources.add_file("main.ciac", "service ;\n");
        let diag = Diagnostic::new(ErrorCode::UnexpectedToken, "expected a name")
            .with_label(Span::new(file, 8, 9), "found `;`")
            .with_help("declarations look like `service VideoPlatform;`");
        let text = render_all([diag], &sources, &PlainRenderer);
        assert_eq!(
            text,
            "error[CIAC0002]: expected a name\n  --> main.ciac:1:9: found `;`\n  help: declarations look like `service VideoPlatform;`\n"
        );
    }

    /// v0.8 M1 regression: a diagnostic whose labels span two different
    /// files (e.g. a name declared once per file) must resolve each
    /// label against *its own* file, not the primary span's file —
    /// `AriadneRenderer` once reused the primary span's file name for
    /// every label, misapplying another file's byte offsets to it.
    #[test]
    fn ariadne_renderer_resolves_labels_across_files() {
        let mut sources = SourceMap::new();
        let other = sources.add_file("other.ciac", "record Video {\n    id: Uuid;\n}\n");
        let entry = sources.add_file(
            "entry.ciac",
            "service S;\nrecord Video {\n    id: Uuid;\n}\n",
        );
        let diag = Diagnostic::new(
            ErrorCode::DuplicateDeclaration,
            "record `Video` is declared more than once",
        )
        .with_label(Span::new(other, 0, 30), "first declared here")
        .with_label(Span::new(entry, 11, 42), "duplicate declaration here");

        let text = render_all([diag], &sources, &AriadneRenderer { color: false });

        assert!(
            text.contains("other.ciac"),
            "expected the first-declaration label's own file to appear: {text}"
        );
        assert!(
            text.contains("entry.ciac"),
            "expected the duplicate label's own file to appear: {text}"
        );
    }
}
