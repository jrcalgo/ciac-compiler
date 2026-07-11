//! `ciac lsp` (v0.12 M2): a Language Server Protocol server over
//! stdio — editors configure `ciac lsp` as the server command for
//! `.ciac` files.
//!
//! Scope for the first cut, per 12UpdatePlan.md:
//!
//! * **diagnostics** on didOpen/didSave — the same front end the CLI
//!   runs (`ciac_syntax::load` + `ciac_sema::analyze`, so imports
//!   resolve exactly as `ciac check` would from the file's real
//!   path), spans resolved through the same `line_col` pipeline the
//!   `--json` output uses (converted to LSP's 0-based positions).
//!   didChange only marks the buffer dirty; revalidation happens on
//!   save, because import resolution reads from disk — resolving
//!   unsaved buffers needs a VFS layer that is deliberately out of
//!   v0.12's scope.
//! * **hover** — the shared vocabulary in [`crate::vocab`] (keywords,
//!   capabilities, and providers, with their per-target support
//!   notes — the same table `ciac describe` renders from), plus the
//!   record/stream/api/handler/... declarations harvested from the
//!   file's last good parse.
//! * **completion** — the same shared vocabulary plus the harvested
//!   declaration names.
//!
//! Rename, references, code actions, and incremental parsing are
//! explicitly deferred.

use crate::vocab;
use anyhow::Result;
use ciac_diagnostics::{Diagnostics, Severity, SourceMap};
use ciac_syntax::ast;
use lsp_server::{Connection, Message, Notification, Response};
use lsp_types::{
    CompletionItem, CompletionItemKind, CompletionOptions, CompletionParams, CompletionResponse,
    Diagnostic, DiagnosticSeverity, DidChangeTextDocumentParams, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, DidSaveTextDocumentParams, Hover, HoverContents,
    HoverProviderCapability, MarkupContent, MarkupKind, NumberOrString, Position,
    PublishDiagnosticsParams, Range, ServerCapabilities, TextDocumentPositionParams,
    TextDocumentSyncCapability, TextDocumentSyncKind, TextDocumentSyncOptions,
    TextDocumentSyncSaveOptions, Url,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// Everything the server remembers about one open document.
struct DocState {
    /// Latest buffer content (kept current by FULL didChange sync) —
    /// used only for hover/completion word extraction, never for
    /// validation, which always reads from disk.
    text: String,
    /// Declarations harvested from the last parse that produced a
    /// program (parse errors keep the previous harvest).
    symbols: Vec<Symbol>,
}

/// One declaration surfaced through hover and completion.
struct Symbol {
    name: String,
    kind: &'static str,
    detail: String,
}

pub fn run() -> Result<ExitCode> {
    let (connection, io_threads) = Connection::stdio();
    let capabilities = serde_json::to_value(ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Options(
            TextDocumentSyncOptions {
                open_close: Some(true),
                change: Some(TextDocumentSyncKind::FULL),
                save: Some(TextDocumentSyncSaveOptions::Supported(true)),
                ..Default::default()
            },
        )),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        completion_provider: Some(CompletionOptions::default()),
        ..Default::default()
    })?;
    connection.initialize(capabilities)?;
    main_loop(&connection)?;
    // The writer thread only exits once every sender is gone; keeping
    // `connection` alive across the join would deadlock the shutdown.
    drop(connection);
    io_threads.join()?;
    Ok(ExitCode::SUCCESS)
}

fn main_loop(connection: &Connection) -> Result<()> {
    let mut docs: HashMap<Url, DocState> = HashMap::new();
    for msg in &connection.receiver {
        match msg {
            Message::Request(req) => {
                if connection.handle_shutdown(&req)? {
                    return Ok(());
                }
                let response = match req.method.as_str() {
                    "textDocument/hover" => {
                        let result =
                            serde_json::from_value::<TextDocumentPositionParams>(req.params)
                                .ok()
                                .and_then(|p| hover(&p, &docs));
                        Response::new_ok(req.id, serde_json::to_value(result)?)
                    }
                    "textDocument/completion" => {
                        let result = serde_json::from_value::<CompletionParams>(req.params)
                            .ok()
                            .map(|p| completion(&p, &docs));
                        Response::new_ok(req.id, serde_json::to_value(result)?)
                    }
                    other => Response::new_err(
                        req.id,
                        lsp_server::ErrorCode::MethodNotFound as i32,
                        format!("unhandled method `{other}`"),
                    ),
                };
                connection.sender.send(Message::Response(response))?;
            }
            Message::Notification(note) => match note.method.as_str() {
                "textDocument/didOpen" => {
                    if let Ok(p) = serde_json::from_value::<DidOpenTextDocumentParams>(note.params)
                    {
                        let uri = p.text_document.uri;
                        docs.insert(
                            uri.clone(),
                            DocState {
                                text: p.text_document.text,
                                symbols: Vec::new(),
                            },
                        );
                        revalidate(connection, &uri, &mut docs)?;
                    }
                }
                "textDocument/didChange" => {
                    // FULL sync: the last change is the whole buffer.
                    // Dirty text feeds hover/completion only —
                    // diagnostics wait for the save.
                    if let Ok(mut p) =
                        serde_json::from_value::<DidChangeTextDocumentParams>(note.params)
                    {
                        if let (Some(doc), Some(change)) =
                            (docs.get_mut(&p.text_document.uri), p.content_changes.pop())
                        {
                            doc.text = change.text;
                        }
                    }
                }
                "textDocument/didSave" => {
                    if let Ok(p) = serde_json::from_value::<DidSaveTextDocumentParams>(note.params)
                    {
                        revalidate(connection, &p.text_document.uri, &mut docs)?;
                    }
                }
                "textDocument/didClose" => {
                    if let Ok(p) = serde_json::from_value::<DidCloseTextDocumentParams>(note.params)
                    {
                        docs.remove(&p.text_document.uri);
                        publish(connection, &p.text_document.uri, Vec::new())?;
                    }
                }
                _ => {}
            },
            Message::Response(_) => {}
        }
    }
    Ok(())
}

/// Runs the CLI's front end on the document's on-disk content and
/// publishes the resulting diagnostics; refreshes the symbol harvest
/// when the parse produced a program.
fn revalidate(connection: &Connection, uri: &Url, docs: &mut HashMap<Url, DocState>) -> Result<()> {
    let Ok(path) = uri.to_file_path() else {
        return Ok(());
    };
    let mut sources = SourceMap::new();
    let mut diags = Diagnostics::new();
    let lsp_diags = match ciac_syntax::load(&path, &mut sources, &mut diags) {
        Ok(program) => {
            ciac_sema::analyze(&program, &mut diags);
            diags.sort();
            if let Some(doc) = docs.get_mut(uri) {
                doc.symbols = harvest(&program);
            }
            to_lsp_diagnostics(&diags, &sources, &path)
        }
        // Unreadable entry file (deleted underneath the editor, most
        // likely): a single whole-file diagnostic beats silence.
        Err(err) => vec![Diagnostic {
            range: Range::default(),
            severity: Some(DiagnosticSeverity::ERROR),
            message: format!("{err:#}"),
            source: Some("ciac".into()),
            ..Default::default()
        }],
    };
    publish(connection, uri, lsp_diags)
}

fn publish(connection: &Connection, uri: &Url, diagnostics: Vec<Diagnostic>) -> Result<()> {
    let params = PublishDiagnosticsParams {
        uri: uri.clone(),
        diagnostics,
        version: None,
    };
    connection.sender.send(Message::Notification(Notification {
        method: "textDocument/publishDiagnostics".into(),
        params: serde_json::to_value(params)?,
    }))?;
    Ok(())
}

/// The same span→line/col resolution the `--json` envelope uses,
/// shifted to LSP's 0-based positions. Labels living in *other* files
/// (imports) can't be attached to this document's ranges, so such a
/// diagnostic is reported at the top of the document with the foreign
/// location folded into the message.
fn to_lsp_diagnostics(diags: &Diagnostics, sources: &SourceMap, path: &Path) -> Vec<Diagnostic> {
    let canonical = path.canonicalize().ok();
    let mut out = Vec::new();
    for diag in diags.iter() {
        let severity = Some(match diag.severity {
            Severity::Error => DiagnosticSeverity::ERROR,
            Severity::Warning => DiagnosticSeverity::WARNING,
        });
        let mut range = None;
        let mut foreign = None;
        for label in &diag.labels {
            let file = sources.file(label.span.file);
            let label_path = PathBuf::from(&file.name);
            if label_path.canonicalize().ok() == canonical || label_path == path {
                let (line, col) = file.line_col(label.span.start);
                let (end_line, end_col) = file.line_col(label.span.end);
                range = Some(Range {
                    start: Position::new(line - 1, col - 1),
                    end: Position::new(end_line - 1, end_col - 1),
                });
                break;
            }
            if foreign.is_none() {
                let (line, col) = file.line_col(label.span.start);
                foreign = Some(format!("{}:{line}:{col}", file.name));
            }
        }
        let message = match (&range, foreign) {
            (None, Some(loc)) => format!("{} (at {loc})", diag.message),
            _ => diag.message.clone(),
        };
        let mut message = message;
        if let Some(help) = &diag.help {
            message.push_str("\nhelp: ");
            message.push_str(help);
        }
        out.push(Diagnostic {
            range: range.unwrap_or_default(),
            severity,
            code: Some(NumberOrString::String(
                <&'static str>::from(diag.code).to_string(),
            )),
            source: Some("ciac".into()),
            message,
            ..Default::default()
        });
    }
    out
}

fn hover(params: &TextDocumentPositionParams, docs: &HashMap<Url, DocState>) -> Option<Hover> {
    let doc = docs.get(&params.text_document.uri)?;
    let word = word_at(&doc.text, params.position)?;
    let value = if let Some(text) = vocab::doc_for(&word) {
        text
    } else {
        let sym = doc.symbols.iter().find(|s| s.name == word)?;
        format!("**{}** `{}`\n\n{}", sym.kind, sym.name, sym.detail)
    };
    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value,
        }),
        range: None,
    })
}

fn completion(params: &CompletionParams, docs: &HashMap<Url, DocState>) -> CompletionResponse {
    let mut items: Vec<CompletionItem> = Vec::new();
    for (word, doc) in vocab::KEYWORDS {
        items.push(item(word, CompletionItemKind::KEYWORD, doc));
    }
    for cap in vocab::CAPABILITIES {
        let doc = vocab::doc_for(cap.name).unwrap_or_default();
        items.push(item(cap.name, CompletionItemKind::MODULE, &doc));
    }
    for provider in vocab::PROVIDERS {
        let doc = vocab::doc_for(provider.name).unwrap_or_default();
        items.push(item(provider.name, CompletionItemKind::ENUM_MEMBER, &doc));
    }
    for (word, doc) in vocab::BUILTIN_STEPS {
        items.push(item(word, CompletionItemKind::FUNCTION, doc));
    }
    if let Some(doc) = docs.get(&params.text_document_position.text_document.uri) {
        for sym in &doc.symbols {
            let kind = match sym.kind {
                "record" | "error" | "table" => CompletionItemKind::STRUCT,
                "stream" | "channel" => CompletionItemKind::EVENT,
                "api" => CompletionItemKind::INTERFACE,
                "service" | "project" => CompletionItemKind::MODULE,
                "blueprint" | "crud" => CompletionItemKind::CLASS,
                _ => CompletionItemKind::FUNCTION,
            };
            items.push(CompletionItem {
                label: sym.name.clone(),
                kind: Some(kind),
                detail: Some(sym.detail.clone()),
                ..Default::default()
            });
        }
    }
    CompletionResponse::Array(items)
}

fn item(label: &str, kind: CompletionItemKind, detail: &str) -> CompletionItem {
    CompletionItem {
        label: label.to_string(),
        kind: Some(kind),
        detail: Some(detail.to_string()),
        ..Default::default()
    }
}

/// The identifier under the cursor: contiguous `[A-Za-z0-9_]` around
/// `position.character`. Char-indexed rather than UTF-16 — CIaC
/// identifiers are ASCII, so the two agree everywhere it matters.
fn word_at(text: &str, position: Position) -> Option<String> {
    let line: Vec<char> = text.lines().nth(position.line as usize)?.chars().collect();
    let is_word = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let at = (position.character as usize).min(line.len());
    // The cursor may sit just past the word's last character.
    let anchor = if at < line.len() && is_word(line[at]) {
        at
    } else if at > 0 && is_word(line[at - 1]) {
        at - 1
    } else {
        return None;
    };
    let start = line[..anchor]
        .iter()
        .rposition(|&c| !is_word(c))
        .map_or(0, |i| i + 1);
    let end = line[anchor..]
        .iter()
        .position(|&c| !is_word(c))
        .map_or(line.len(), |i| anchor + i);
    Some(line[start..end].iter().collect())
}

/// Walks a parsed program collecting every named declaration, so
/// hover/completion can answer for user-defined names — including
/// declarations nested in `service { .. }` blocks.
fn harvest(program: &ast::Program) -> Vec<Symbol> {
    fn fields(fields: &[ast::Field]) -> String {
        fields
            .iter()
            .map(|f| {
                let ty = match &f.ty {
                    ast::TypeExpr::Named(id) => id.text.clone(),
                    ast::TypeExpr::Enum { variants, .. } => format!(
                        "enum {{ {} }}",
                        variants
                            .iter()
                            .map(|v| v.text.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                };
                format!("{}: {ty}", f.name.text)
            })
            .collect::<Vec<_>>()
            .join("; ")
    }

    fn push(out: &mut Vec<Symbol>, name: &ast::Ident, kind: &'static str, detail: String) {
        out.push(Symbol {
            name: name.text.clone(),
            kind,
            detail,
        });
    }

    fn service_item(out: &mut Vec<Symbol>, item: &ast::ServiceItem) {
        match item {
            ast::ServiceItem::Api(a) => api(out, a),
            ast::ServiceItem::Worker(w) => worker(out, w),
            ast::ServiceItem::Job(j) => push(out, &j.name, "job", "scheduled job".into()),
            ast::ServiceItem::Channel(c) => push(
                out,
                &c.name,
                "channel",
                format!("realtime channel on stream `{}`", c.stream.text),
            ),
            ast::ServiceItem::Crud(c) => crud(out, c),
            ast::ServiceItem::Events(e) => {
                push(out, &e.name, "events", "stream + worker shorthand".into())
            }
            ast::ServiceItem::Handler(h) => handler(out, h),
            ast::ServiceItem::Pipeline(p) => pipeline(out, p),
            ast::ServiceItem::Use(_) | ast::ServiceItem::Expand(_) => {}
        }
    }

    fn api(out: &mut Vec<Symbol>, a: &ast::ApiDecl) {
        let detail = match &a.request {
            Some(r) => format!("HTTP api; request record `{}`", r.text),
            None => "HTTP api (untyped)".into(),
        };
        push(out, &a.name, "api", detail);
    }

    fn worker(out: &mut Vec<Symbol>, w: &ast::WorkerDecl) {
        let detail = match &w.stream {
            Some(s) => format!("worker consuming stream `{}`", s.text),
            None => "worker on the service's default stream".into(),
        };
        push(out, &w.name, "worker", detail);
    }

    fn crud(out: &mut Vec<Symbol>, c: &ast::CrudDecl) {
        let detail = match &c.record {
            Some(r) => format!("typed CRUD resource over record `{}`", r.text),
            None => "keyed-document CRUD resource".into(),
        };
        push(out, &c.name, "crud", detail);
    }

    fn handler(out: &mut Vec<Symbol>, h: &ast::HandlerDecl) {
        let detail = if h.params.is_empty() {
            "pipeline step handler".to_string()
        } else {
            let params = h
                .params
                .iter()
                .map(|p| p.name.text.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            format!("typed handler({params})")
        };
        push(out, &h.name, "handler", detail);
    }

    fn pipeline(out: &mut Vec<Symbol>, p: &ast::PipelineDecl) {
        push(out, &p.name, "pipeline", "pipeline of steps".into());
    }

    let mut out = Vec::new();
    for item in &program.items {
        match item {
            ast::Item::Project(p) => push(&mut out, &p.name, "project", "project name".into()),
            ast::Item::Service(s) => push(&mut out, &s.name, "service", "service name".into()),
            ast::Item::ServiceBlock(block) => {
                push(
                    &mut out,
                    &block.name,
                    "service",
                    "deployable service".into(),
                );
                for item in &block.items {
                    service_item(&mut out, item);
                }
            }
            ast::Item::Record(r) => {
                let kind = match r.kind {
                    ast::RecordKind::Data => "record",
                    ast::RecordKind::Error => "error",
                };
                push(
                    &mut out,
                    &r.name,
                    kind,
                    format!("{{ {} }}", fields(&r.fields)),
                );
            }
            ast::Item::Stream(s) => push(
                &mut out,
                &s.name,
                "stream",
                format!("typed message stream of record `{}`", s.record.text),
            ),
            ast::Item::Table(t) => push(
                &mut out,
                &t.name,
                "table",
                format!("persistent table of record `{}`", t.record.text),
            ),
            ast::Item::Api(a) => api(&mut out, a),
            ast::Item::Worker(w) => worker(&mut out, w),
            ast::Item::Job(j) => push(&mut out, &j.name, "job", "scheduled job".into()),
            ast::Item::Channel(c) => push(
                &mut out,
                &c.name,
                "channel",
                format!("realtime channel on stream `{}`", c.stream.text),
            ),
            ast::Item::Crud(c) => crud(&mut out, c),
            ast::Item::Events(e) => push(
                &mut out,
                &e.name,
                "events",
                "stream + worker shorthand".into(),
            ),
            ast::Item::Handler(h) => handler(&mut out, h),
            ast::Item::Pipeline(p) => pipeline(&mut out, p),
            ast::Item::Blueprint(b) => push(
                &mut out,
                &b.name,
                "blueprint",
                format!(
                    "parameterized template over <{}: record>",
                    b.type_param.text
                ),
            ),
            ast::Item::Import(_) | ast::Item::Expand(_) | ast::Item::Use(_) => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_at_finds_the_identifier_around_the_cursor() {
        let text = "use {\n    db Postgres;\n}\n";
        // Cursor in the middle of `Postgres`.
        assert_eq!(
            word_at(text, Position::new(1, 9)).as_deref(),
            Some("Postgres")
        );
        // Cursor immediately after `db`.
        assert_eq!(word_at(text, Position::new(1, 6)).as_deref(), Some("db"));
        // Cursor on whitespace.
        assert_eq!(word_at(text, Position::new(0, 4)), None);
    }

    #[test]
    fn static_table_covers_the_provider_registry() {
        for word in [
            "service", "db", "queue", "Kafka", "OAuth2", "Return", "pipeline",
        ] {
            assert!(vocab::doc_for(word).is_some(), "no hover for `{word}`");
        }
        assert!(vocab::doc_for("NotAThing").is_none());
    }

    #[test]
    fn harvest_collects_nested_service_declarations() {
        let mut sources = SourceMap::new();
        let mut diags = Diagnostics::new();
        let dir = std::env::temp_dir().join(format!("ciac-lsp-harvest-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("main.ciac");
        std::fs::write(
            &file,
            "project P;\nrecord Item { id: Uuid; }\nservice S {\n    api Price: Item;\n    pipeline Price: Return;\n}\n",
        )
        .unwrap();
        let program = ciac_syntax::load(&file, &mut sources, &mut diags).unwrap();
        assert!(!diags.has_errors());
        let symbols = harvest(&program);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["P", "Item", "S", "Price", "Price"]);
        let item = symbols.iter().find(|s| s.name == "Item").unwrap();
        assert_eq!(item.kind, "record");
        assert!(item.detail.contains("id: Uuid"), "{}", item.detail);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
