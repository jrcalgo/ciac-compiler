//! `ciac lsp` (v0.12 M2): a Language Server Protocol server over
//! stdio — editors configure `ciac lsp` as the server command for
//! `.ciac` files.
//!
//! Scope for the first cut, per 12UpdatePlan.md (widened since, most
//! recently by `29UpdatePlan.md` M8 -- see below):
//!
//! * **diagnostics** on didOpen/didSave (`revalidate`, always reads
//!   the entry file from disk) and, since M8, on a debounced didChange
//!   too (`revalidate_overlay`, `DIDCHANGE_DEBOUNCE` after the last
//!   keystroke) — the same front end the CLI runs (`ciac_syntax::load`
//!   / `load_with_overlay` + `ciac_sema::analyze`), spans resolved
//!   through the same `line_col` pipeline the `--json` output uses
//!   (converted to LSP's 0-based positions). `load_with_overlay`
//!   substitutes the *open document's own* dirty buffer for its entry-
//!   file read only; anything it `import`s still resolves from disk,
//!   since only the document actually open in the client has unsaved
//!   content to substitute — still no general VFS layer, which stays
//!   deliberately out of scope.
//! * **hover** — the shared vocabulary in [`crate::vocab`] (keywords,
//!   capabilities, and providers, with their per-target support
//!   notes — the same table `ciac describe` renders from), plus the
//!   record/stream/api/handler/... declarations harvested from the
//!   file's last good parse.
//! * **completion** — the same shared vocabulary (snippet bodies for
//!   declaration keywords since M7, `InsertTextFormat::Snippet`) plus
//!   the harvested declaration names.
//! * **rename** (v0.18 M7, Pillar 8) — `textDocument/prepareRename` and
//!   `textDocument/rename` over the same whole-program resolver
//!   `ciac rename` uses ([`ciac_syntax::rename_index`]). Like
//!   diagnostics, this reads the document's on-disk content and treats
//!   the requesting document's own path as the resolution entry point
//!   — consistent with how `revalidate` already parses each open
//!   document independently rather than tracking a separate workspace
//!   root.
//! * **definition** (M8) — a thin projection of the same
//!   `rename_index`: identifier at the cursor resolves to its
//!   declaration site, same-file or across an `import`.
//!
//! References, incremental (non-FULL) sync, and a real VFS overlay for
//! imported files are still explicitly deferred.

use crate::json_out::{JsonEdit, JsonFix};
use crate::vocab;
use anyhow::Result;
use ciac_diagnostics::{Diagnostics, FileId, Severity, SourceMap};
use ciac_syntax::ast;
use lsp_server::{Connection, ErrorCode, Message, Notification, Response};
use lsp_types::{
    CodeAction, CodeActionKind, CodeActionOrCommand, CodeActionParams,
    CodeActionProviderCapability, CodeActionResponse, CompletionItem, CompletionItemKind,
    CompletionOptions, CompletionParams, CompletionResponse, Diagnostic, DiagnosticSeverity,
    DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
    DidSaveTextDocumentParams, Documentation, GotoDefinitionResponse, Hover, HoverContents,
    HoverProviderCapability, InsertTextFormat, Location, MarkupContent, MarkupKind, NumberOrString,
    OneOf, Position, PrepareRenameResponse, PublishDiagnosticsParams, Range, RenameOptions,
    RenameParams, ServerCapabilities, TextDocumentPositionParams, TextDocumentSyncCapability,
    TextDocumentSyncKind, TextDocumentSyncOptions, TextDocumentSyncSaveOptions, TextEdit, Url,
    WorkDoneProgressOptions, WorkspaceEdit,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// Everything the server remembers about one open document.
struct DocState {
    /// Latest buffer content (kept current by FULL didChange sync) —
    /// feeds hover/completion word extraction immediately, and (since
    /// M8) the debounced `revalidate_overlay` reparse once
    /// `DIDCHANGE_DEBOUNCE` has passed with no further edits.
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
        code_action_provider: Some(CodeActionProviderCapability::Simple(true)),
        rename_provider: Some(OneOf::Right(RenameOptions {
            prepare_provider: Some(true),
            work_done_progress_options: WorkDoneProgressOptions::default(),
        })),
        definition_provider: Some(OneOf::Left(true)),
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

/// v0.27 M8: how long a document sits dirty before its debounced
/// `didChange` reparse fires — long enough that a fast typist doesn't
/// trigger a reparse per keystroke, short enough to still read as
/// "live" rather than "waits for save" (the v0.12-era gap this
/// milestone closes).
const DIDCHANGE_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(300);
/// How often the main loop wakes with no message pending, purely to
/// check whether any debounced reparse has come due. Shorter than
/// [`DIDCHANGE_DEBOUNCE`] so the fire time is never late by more than
/// this much.
const DEBOUNCE_POLL: std::time::Duration = std::time::Duration::from_millis(50);

fn main_loop(connection: &Connection) -> Result<()> {
    let mut docs: HashMap<Url, DocState> = HashMap::new();
    // Doc -> the instant its debounced didChange reparse should fire.
    // Reset on every further keystroke; cleared the moment a real
    // save/open reparse (from disk) makes it moot.
    let mut pending_reparse: HashMap<Url, std::time::Instant> = HashMap::new();
    loop {
        let msg = match connection.receiver.recv_timeout(DEBOUNCE_POLL) {
            Ok(msg) => msg,
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                let now = std::time::Instant::now();
                let due: Vec<Url> = pending_reparse
                    .iter()
                    .filter(|(_, deadline)| **deadline <= now)
                    .map(|(uri, _)| uri.clone())
                    .collect();
                for uri in due {
                    pending_reparse.remove(&uri);
                    revalidate_overlay(connection, &uri, &mut docs)?;
                }
                continue;
            }
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => return Ok(()),
        };
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
                    "textDocument/codeAction" => {
                        let result = serde_json::from_value::<CodeActionParams>(req.params)
                            .ok()
                            .map(|p| code_actions(&p));
                        Response::new_ok(req.id, serde_json::to_value(result)?)
                    }
                    "textDocument/prepareRename" => {
                        match serde_json::from_value::<TextDocumentPositionParams>(req.params) {
                            Ok(p) => {
                                Response::new_ok(req.id, serde_json::to_value(prepare_rename(&p))?)
                            }
                            Err(err) => Response::new_err(
                                req.id,
                                ErrorCode::InvalidParams as i32,
                                err.to_string(),
                            ),
                        }
                    }
                    "textDocument/rename" => {
                        match serde_json::from_value::<RenameParams>(req.params) {
                            Ok(p) => match rename(&p) {
                                Ok(edit) => Response::new_ok(req.id, serde_json::to_value(edit)?),
                                Err(msg) => {
                                    Response::new_err(req.id, ErrorCode::InvalidRequest as i32, msg)
                                }
                            },
                            Err(err) => Response::new_err(
                                req.id,
                                ErrorCode::InvalidParams as i32,
                                err.to_string(),
                            ),
                        }
                    }
                    "textDocument/definition" => {
                        let result =
                            serde_json::from_value::<TextDocumentPositionParams>(req.params)
                                .ok()
                                .and_then(|p| definition(&p));
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
                        pending_reparse.remove(&uri);
                        revalidate(connection, &uri, &mut docs)?;
                    }
                }
                "textDocument/didChange" => {
                    // FULL sync: the last change is the whole buffer.
                    // Dirty text feeds hover/completion immediately;
                    // diagnostics follow after `DIDCHANGE_DEBOUNCE` of
                    // no further edits (v0.27 M8), reading this
                    // in-memory buffer via `load_with_overlay` rather
                    // than waiting for a save.
                    if let Ok(mut p) =
                        serde_json::from_value::<DidChangeTextDocumentParams>(note.params)
                    {
                        if let (Some(doc), Some(change)) =
                            (docs.get_mut(&p.text_document.uri), p.content_changes.pop())
                        {
                            doc.text = change.text;
                            pending_reparse.insert(
                                p.text_document.uri,
                                std::time::Instant::now() + DIDCHANGE_DEBOUNCE,
                            );
                        }
                    }
                }
                "textDocument/didSave" => {
                    if let Ok(p) = serde_json::from_value::<DidSaveTextDocumentParams>(note.params)
                    {
                        pending_reparse.remove(&p.text_document.uri);
                        revalidate(connection, &p.text_document.uri, &mut docs)?;
                    }
                }
                "textDocument/didClose" => {
                    if let Ok(p) = serde_json::from_value::<DidCloseTextDocumentParams>(note.params)
                    {
                        pending_reparse.remove(&p.text_document.uri);
                        docs.remove(&p.text_document.uri);
                        publish(connection, &p.text_document.uri, Vec::new())?;
                    }
                }
                _ => {}
            },
            Message::Response(_) => {}
        }
    }
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

/// The debounced `didChange` counterpart to [`revalidate`] (v0.27 M8):
/// same front end, same diagnostic shape, but the entry file's content
/// comes from the dirty in-memory buffer (`load_with_overlay`) instead
/// of disk -- everything the document itself `import`s still resolves
/// from disk, since only the document open in the client has unsaved
/// content to substitute. A no-op if the document has since closed
/// (its `DocState` is gone by the time the debounce timer fires).
fn revalidate_overlay(
    connection: &Connection,
    uri: &Url,
    docs: &mut HashMap<Url, DocState>,
) -> Result<()> {
    let Ok(path) = uri.to_file_path() else {
        return Ok(());
    };
    let Some(text) = docs.get(uri).map(|doc| doc.text.clone()) else {
        return Ok(());
    };
    let mut sources = SourceMap::new();
    let mut diags = Diagnostics::new();
    let lsp_diags =
        match ciac_syntax::module::load_with_overlay(&path, &text, &mut sources, &mut diags) {
            Ok(program) => {
                ciac_sema::analyze(&program, &mut diags);
                diags.sort();
                if let Some(doc) = docs.get_mut(uri) {
                    doc.symbols = harvest(&program);
                }
                to_lsp_diagnostics(&diags, &sources, &path)
            }
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
        // v0.15 M7: fixes ride along in `data`, the one field LSP
        // preserves between `publishDiagnostics` and `codeAction` --
        // `code_actions` below reads it straight back out, so the
        // server never needs to remember diagnostics itself.
        let fixes: Vec<JsonFix> = diag
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
            .collect();
        out.push(Diagnostic {
            range: range.unwrap_or_default(),
            severity,
            code: Some(NumberOrString::String(
                <&'static str>::from(diag.code).to_string(),
            )),
            source: Some("ciac".into()),
            message,
            data: (!fixes.is_empty())
                .then(|| serde_json::to_value(&fixes).ok())
                .flatten(),
            ..Default::default()
        });
    }
    out
}

/// Turns each code-action-request diagnostic's `data` (the fixes
/// [`to_lsp_diagnostics`] stashed there) into an offered quick-fix
/// (v0.15 M7) -- the LSP quick-fix and the `--json`/MCP fix are the
/// same edits, just carried over a different wire.
fn code_actions(params: &CodeActionParams) -> CodeActionResponse {
    let mut actions: CodeActionResponse = Vec::new();
    for diag in &params.context.diagnostics {
        let Some(data) = &diag.data else { continue };
        let Ok(fixes) = serde_json::from_value::<Vec<JsonFix>>(data.clone()) else {
            continue;
        };
        for fix in fixes {
            let edits: Vec<TextEdit> = fix
                .edits
                .iter()
                .map(|edit| TextEdit {
                    range: Range {
                        start: Position::new(edit.line - 1, edit.column - 1),
                        end: Position::new(edit.end_line - 1, edit.end_column - 1),
                    },
                    new_text: edit.replacement.clone(),
                })
                .collect();
            let mut changes = HashMap::new();
            changes.insert(params.text_document.uri.clone(), edits);
            actions.push(CodeActionOrCommand::CodeAction(CodeAction {
                title: fix.title,
                kind: Some(CodeActionKind::QUICKFIX),
                diagnostics: Some(vec![diag.clone()]),
                edit: Some(WorkspaceEdit {
                    changes: Some(changes),
                    ..Default::default()
                }),
                ..Default::default()
            }));
        }
    }
    actions
}

/// Resolves `position.text_document.uri`'s on-disk path to a `FileId`
/// in a freshly loaded `sources`, converting the LSP (0-based) position
/// to a byte offset via `offset_of`. Shared by `prepare_rename` and
/// `rename` so both agree on exactly which site the cursor is on.
fn locate(path: &Path, sources: &SourceMap, position: Position) -> Option<(FileId, u32)> {
    let canonical = path.canonicalize().ok()?;
    let canonical_str = canonical.display().to_string();
    let file_id = sources
        .files()
        .enumerate()
        .find(|(_, f)| f.name == canonical_str)
        .map(|(i, _)| FileId(i as u32))?;
    let offset = sources
        .file(file_id)
        .offset_of(position.line + 1, position.character + 1)?;
    Some((file_id, offset))
}

/// `textDocument/prepareRename` — highlights the exact token under the
/// cursor and seeds the client's rename box with its current name.
/// Returns `None` (no rename here) for an unresolved position, a parse
/// error, or a symbol the rename engine doesn't consider renamable at
/// all (e.g. a keyword) — `ciac rename`'s own resolver is the single
/// source of truth for "is this renamable", so this never duplicates
/// that logic.
fn prepare_rename(params: &TextDocumentPositionParams) -> Option<PrepareRenameResponse> {
    let path = params.text_document.uri.to_file_path().ok()?;
    let mut sources = SourceMap::new();
    let mut diags = Diagnostics::new();
    let program = ciac_syntax::load(&path, &mut sources, &mut diags).ok()?;
    if diags.has_errors() {
        return None;
    }
    let index = ciac_syntax::rename_index::build_index(&program);
    let (file_id, offset) = locate(&path, &sources, params.position)?;
    let (resolved, span) = index.site_at(file_id, offset).into_iter().next()?;
    let file = sources.file(file_id);
    let (start_line, start_col) = file.line_col(span.start);
    let (end_line, end_col) = file.line_col(span.end);
    Some(PrepareRenameResponse::RangeWithPlaceholder {
        range: Range {
            start: Position::new(start_line - 1, start_col - 1),
            end: Position::new(end_line - 1, end_col - 1),
        },
        placeholder: resolved.name,
    })
}

/// `textDocument/rename` — resolves the symbol under the cursor and
/// applies the same whole-program plan `ciac rename` would compute,
/// turning it into a `WorkspaceEdit` the client applies across every
/// affected file. Errors (parse failure, no symbol at the position, a
/// reserved/invalid new name, a namespace collision) come back as a
/// request error rather than a silent no-op, since — unlike
/// `prepareRename` — the user has already committed to a specific new
/// name and deserves to know why it didn't take.
fn rename(params: &RenameParams) -> std::result::Result<WorkspaceEdit, String> {
    let path = params
        .text_document_position
        .text_document
        .uri
        .to_file_path()
        .map_err(|_| "not a file:// URI".to_string())?;
    let mut sources = SourceMap::new();
    let mut diags = Diagnostics::new();
    let (program, origins) =
        ciac_syntax::module::load_with_origins(&path, &mut sources, &mut diags)
            .map_err(|err| format!("{err:#}"))?;
    if diags.has_errors() {
        return Err(format!("{} has parse/analysis errors", path.display()));
    }
    let index = ciac_syntax::rename_index::build_index(&program);
    let (file_id, offset) = locate(&path, &sources, params.text_document_position.position)
        .ok_or_else(|| "position is outside the resolved source set".to_string())?;
    let resolved = index
        .resolve_at(file_id, offset)
        .into_iter()
        .next()
        .ok_or_else(|| "no renamable symbol at this position".to_string())?;
    let plan = index
        .plan_rename(&origins, resolved.id, &params.new_name)
        .map_err(|err| err.to_string())?;

    let mut changes: HashMap<Url, Vec<TextEdit>> = HashMap::new();
    for (edit_file_id, fix) in &plan.edits_by_file {
        let file = sources.file(*edit_file_id);
        let uri = Url::from_file_path(&file.name)
            .map_err(|_| format!("cannot form a file:// URI for {}", file.name))?;
        let edits = fix
            .edits
            .iter()
            .map(|edit| {
                let (l, c) = file.line_col(edit.span.start);
                let (el, ec) = file.line_col(edit.span.end);
                TextEdit {
                    range: Range {
                        start: Position::new(l - 1, c - 1),
                        end: Position::new(el - 1, ec - 1),
                    },
                    new_text: edit.replacement.clone(),
                }
            })
            .collect();
        changes.insert(uri, edits);
    }
    Ok(WorkspaceEdit {
        changes: Some(changes),
        ..Default::default()
    })
}

/// `textDocument/definition` (v0.27 M8) — a thin projection of the
/// same `rename_index` `prepare_rename`/`rename` already ride: the
/// identifier at the cursor resolves to a [`ResolvedSymbol`], whose
/// `def_span` is the declaration site, same-file or across an
/// `import` (the index is built over the whole spliced program, so a
/// definition in another file already carries that file's own span).
/// `None` for an unresolved position, a parse error, or a definition
/// site with no real on-disk file (`std/`-embedded blueprints and
/// `registry:`-fetched content use synthetic source names, not real
/// paths -- there's nowhere to navigate to, so no location is better
/// than a broken one).
fn definition(params: &TextDocumentPositionParams) -> Option<GotoDefinitionResponse> {
    let path = params.text_document.uri.to_file_path().ok()?;
    let mut sources = SourceMap::new();
    let mut diags = Diagnostics::new();
    let program = ciac_syntax::load(&path, &mut sources, &mut diags).ok()?;
    if diags.has_errors() {
        return None;
    }
    let index = ciac_syntax::rename_index::build_index(&program);
    let (file_id, offset) = locate(&path, &sources, params.position)?;
    let resolved = index.resolve_at(file_id, offset).into_iter().next()?;
    let file = sources.file(resolved.def_span.file);
    let uri = Url::from_file_path(&file.name).ok()?;
    let (start_line, start_col) = file.line_col(resolved.def_span.start);
    let (end_line, end_col) = file.line_col(resolved.def_span.end);
    Some(GotoDefinitionResponse::Scalar(Location {
        uri,
        range: Range {
            start: Position::new(start_line - 1, start_col - 1),
            end: Position::new(end_line - 1, end_col - 1),
        },
    }))
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
        match vocab::SNIPPETS.iter().find(|s| s.prefix == *word) {
            // v0.27 Pillar 5: a declaration keyword with a tab-stopped
            // skeleton offers it as a real snippet completion instead
            // of just the bare keyword -- the same table
            // `every_snippet_default_expansion_parses` already proved
            // compiles, so what a user accepts here can never be a
            // skeleton that doesn't actually parse.
            Some(snip) => items.push(snippet_item(snip, doc)),
            None => items.push(item(word, CompletionItemKind::KEYWORD, doc)),
        }
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

/// A declaration keyword's [`vocab::Snippet`] rendered as a real
/// tab-stopped completion — `insert_text` carries the raw VS Code
/// snippet body (`${1:Name}`, `${2|a,b,c|}`, `$0`) verbatim, and
/// `insert_text_format: Snippet` tells the client to interpret it
/// rather than insert it literally.
fn snippet_item(snip: &vocab::Snippet, keyword_doc: &str) -> CompletionItem {
    CompletionItem {
        label: snip.prefix.to_string(),
        kind: Some(CompletionItemKind::SNIPPET),
        detail: Some(snip.description.to_string()),
        documentation: Some(Documentation::String(keyword_doc.to_string())),
        insert_text: Some(snip.body.join("\n")),
        insert_text_format: Some(InsertTextFormat::SNIPPET),
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
    fn type_name(ty: &ast::TypeExpr) -> String {
        match ty {
            ast::TypeExpr::Named(id) => id.text.clone(),
            ast::TypeExpr::Enum { variants, .. } => format!(
                "enum {{ {} }}",
                variants
                    .iter()
                    .map(|v| v.text.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            ast::TypeExpr::List { inner, .. } => format!("[{}]", type_name(inner)),
            ast::TypeExpr::Reference { target, .. } => format!("Reference<{}>", target.text),
        }
    }

    fn fields(fields: &[ast::Field]) -> String {
        fields
            .iter()
            .map(|f| format!("{}: {}", f.name.text, type_name(&f.ty)))
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
            ast::ServiceItem::Table(t) => push(
                out,
                &t.name,
                "table",
                format!("persistent table of record `{}`", t.record.text),
            ),
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
