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
        use ariadne::{Config, Label, Report, ReportKind, Source};

        let Some(primary) = diag.primary_span() else {
            // No span: fall back to the plain renderer.
            return PlainRenderer.render(diag, sources, out);
        };
        let file = sources.file(primary.file);
        let kind = match diag.severity {
            Severity::Error => ReportKind::Error,
            Severity::Warning => ReportKind::Warning,
        };

        let mut report = Report::build(kind, (file.name.as_str(), primary.range()))
            .with_config(Config::default().with_color(self.color))
            .with_code(diag.code.code())
            .with_message(&diag.message);
        for label in &diag.labels {
            report = report.with_label(
                Label::new((file.name.as_str(), label.span.range())).with_message(&label.message),
            );
        }
        if let Some(help) = &diag.help {
            report = report.with_help(help);
        }
        report
            .finish()
            .write((file.name.as_str(), Source::from(&file.src)), out)
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
}
