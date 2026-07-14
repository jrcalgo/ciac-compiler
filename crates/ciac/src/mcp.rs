//! `ciac mcp` (v0.13 M5): a Model Context Protocol server over stdio.
//!
//! Hand-rolled JSON-RPC 2.0, newline-delimited (the MCP stdio
//! transport) — small enough that a client SDK dependency would cost
//! more than it saves for seven tools. Every tool result carries the
//! same JSON envelope `--json` mode already produces (or the
//! `graph`/`describe` document) as one text content block, so an
//! agent speaking MCP sees exactly what a human running `--json` on
//! the command line would — [`crate::commands`]'s envelope-returning
//! functions (v0.13 M5's envelope refactor) are shared, not
//! reimplemented.
//!
//! Scope, per 13UpdatePlan.md: `check`, `build`, `diff`, `verify`
//! (no `--system`/`--live` — those boot Docker and belong to a human
//! at a terminal), `graph`, `explain`, `describe`.
//!
//! v0.18 M7 (18UpdatePlan.md Pillar 8) adds `diff_semantic` (reusing
//! [`commands::diff_semantic_envelope`] verbatim) and `rename` (reusing
//! [`crate::rename::rename_tool`]) — the same envelope-sharing
//! discipline extended to the two new features.

use crate::commands;
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::process::ExitCode;

const PROTOCOL_VERSION: &str = "2024-11-05";

pub fn run() -> Result<ExitCode> {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(err) => {
                write_message(
                    &mut stdout,
                    &json!({
                        "jsonrpc": "2.0",
                        "id": Value::Null,
                        "error": {"code": -32700, "message": format!("parse error: {err}")},
                    }),
                )?;
                continue;
            }
        };
        let Some(method) = msg.get("method").and_then(Value::as_str) else {
            continue;
        };
        // A JSON-RPC notification has no `id` and gets no response —
        // `notifications/initialized` is the only one this server
        // expects, and it needs no action beyond acknowledging receipt.
        let Some(id) = msg.get("id").cloned() else {
            continue;
        };
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        let response = match method {
            "initialize" => ok(id, initialize_result()),
            "tools/list" => ok(id, tools_list_result()),
            "tools/call" => match tools_call(&params) {
                Ok(result) => ok(id, result),
                Err(err) => ok(
                    id,
                    json!({
                        "content": [{"type": "text", "text": format!("{err:#}")}],
                        "isError": true,
                    }),
                ),
            },
            other => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {"code": -32601, "message": format!("unknown method `{other}`")},
            }),
        };
        write_message(&mut stdout, &response)?;
    }
    Ok(ExitCode::SUCCESS)
}

fn ok(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn write_message(out: &mut impl Write, value: &Value) -> Result<()> {
    writeln!(out, "{}", serde_json::to_string(value)?)?;
    out.flush()?;
    Ok(())
}

fn initialize_result() -> Value {
    json!({
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": {"tools": {}},
        "serverInfo": {"name": "ciac", "version": env!("CARGO_PKG_VERSION")},
    })
}

fn tool(name: &str, description: &str, input_schema: Value) -> Value {
    json!({"name": name, "description": description, "inputSchema": input_schema})
}

fn tools_list_result() -> Value {
    let file_prop = json!({"type": "string", "description": "Path to the .ciac source file."});
    let target_prop =
        json!({"type": "string", "description": "Code-generation target, e.g. python, rust."});
    let out_prop =
        json!({"type": "string", "description": "Output directory for the generated project."});
    let name_prop =
        json!({"type": "string", "description": "Override the generated project's name."});
    json!({"tools": [
        tool(
            "check",
            "Parse and validate a CIaC program, reporting diagnostics.",
            json!({
                "type": "object",
                "properties": {"file": file_prop},
                "required": ["file"],
            }),
        ),
        tool(
            "build",
            "Compile a CIaC program into a backend project (regenerates in place; never --force/--adopt).",
            json!({
                "type": "object",
                "properties": {
                    "file": file_prop, "target": target_prop, "out": out_prop, "name": name_prop,
                },
                "required": ["file", "target", "out"],
            }),
        ),
        tool(
            "diff",
            "Show what regeneration would change without writing files.",
            json!({
                "type": "object",
                "properties": {
                    "file": file_prop, "target": target_prop, "out": out_prop, "name": name_prop,
                    "patch": {"type": "boolean", "description": "Include unified diff text for changed entries."},
                },
                "required": ["file", "target", "out"],
            }),
        ),
        tool(
            "verify",
            "Verify a generated project still matches its CIaC source and passes its own test suite (static only; no --system/--live).",
            json!({
                "type": "object",
                "properties": {
                    "file": file_prop, "target": target_prop, "out": out_prop, "name": name_prop,
                },
                "required": ["file", "target", "out"],
            }),
        ),
        tool(
            "graph",
            "Dump the validated system graph.",
            json!({
                "type": "object",
                "properties": {
                    "file": file_prop,
                    "format": {"type": "string", "enum": ["json", "dot"], "description": "Defaults to json."},
                },
                "required": ["file"],
            }),
        ),
        tool(
            "explain",
            "Explain an error code, e.g. CIAC0006.",
            json!({
                "type": "object",
                "properties": {"code": {"type": "string"}},
                "required": ["code"],
            }),
        ),
        tool(
            "describe",
            "The language and CLI's machine-facing vocabulary (capabilities, providers, field types, builtin steps, declaration kinds, error codes, scaffold templates) as one versioned JSON document.",
            json!({"type": "object", "properties": {}}),
        ),
        tool(
            "fix",
            "Apply a named fix `check` offered for a diagnostic (v0.15 M7): `code` selects the diagnostic (e.g. CIAC0005), `index` selects among its offered fixes when it has more than one (default 0). Dry-run by default -- pass apply: true to write the patched source and re-check.",
            json!({
                "type": "object",
                "properties": {
                    "file": file_prop,
                    "code": {"type": "string", "description": "Diagnostic code carrying the fix, e.g. CIAC0005."},
                    "index": {"type": "integer", "description": "Which of that diagnostic's offered fixes to use. Defaults to 0."},
                    "apply": {"type": "boolean", "description": "Write the patched source to `file` and re-check. Defaults to false (preview only)."},
                },
                "required": ["file", "code"],
            }),
        ),
        tool(
            "diff_semantic",
            "Compare this program's canonical semantic model against a baseline (checked-in by default), classifying each change as breaking, additive, or internal (v0.18 Pillar 1/2).",
            json!({
                "type": "object",
                "properties": {
                    "file": file_prop,
                    "against": {"type": "string", "description": "Compare against another .ciac source file instead of a baseline."},
                    "baseline": {"type": "string", "description": "Compare against a specific checked-in baseline JSON file instead of the default."},
                    "deny_breaking": {"type": "boolean", "description": "Fail if any breaking change is present. Defaults to false."},
                },
                "required": ["file"],
            }),
        ),
        tool(
            "rename",
            "Rename a declared symbol across the whole program (v0.18 Pillar 6). Locate the symbol either by position (`target_file`+`line`+`column`) or by qualified name (`old`); `to`/`new_name` gives the replacement. Dry-run by default -- pass apply: true to stage, recompile-check, and commit the edits. Source-only: does not replay any generated --out tree's regeneration.",
            json!({
                "type": "object",
                "properties": {
                    "file": file_prop,
                    "target_file": {"type": "string", "description": "File containing the symbol, for position-based lookup."},
                    "line": {"type": "integer", "description": "1-based line of the symbol, for position-based lookup."},
                    "column": {"type": "integer", "description": "1-based column of the symbol, for position-based lookup."},
                    "to": {"type": "string", "description": "New name, for position-based lookup."},
                    "old": {"type": "string", "description": "Qualified old name, e.g. `Order` or `Order.total`, for name-based lookup."},
                    "new_name": {"type": "string", "description": "New name, for name-based lookup."},
                    "apply": {"type": "boolean", "description": "Write the renamed files. Defaults to false (preview only)."},
                },
                "required": ["file"],
            }),
        ),
    ]})
}

fn tools_call(params: &Value) -> Result<Value> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let args = params.get("arguments").cloned().unwrap_or(Value::Null);
    let text = match name {
        "check" => {
            let file = arg_path(&args, "file")?;
            let (envelope, _code) = commands::check_envelope(&file)?;
            serde_json::to_string_pretty(&envelope)?
        }
        "build" => {
            let file = arg_path(&args, "file")?;
            let target = arg_str(&args, "target")?;
            let out = arg_path(&args, "out")?;
            let name = arg_opt_str(&args, "name");
            let deploy = commands::DeployOpts {
                deploy: Vec::new(),
                image_prefix: None,
                image_tag: "latest".to_owned(),
                profile: "dev".to_owned(),
                secrets: false,
                semantic_baseline: None,
            };
            let (envelope, _code) = commands::build_envelope(&file, &target, &out, deploy, name)?;
            serde_json::to_string_pretty(&envelope)?
        }
        "diff" => {
            let file = arg_path(&args, "file")?;
            let target = arg_str(&args, "target")?;
            let out = arg_path(&args, "out")?;
            let patch = args.get("patch").and_then(Value::as_bool).unwrap_or(false);
            let name = arg_opt_str(&args, "name");
            let (envelope, _code) = commands::diff_envelope(&file, &target, &out, patch, name)?;
            serde_json::to_string_pretty(&envelope)?
        }
        "verify" => {
            let file = arg_path(&args, "file")?;
            let target = arg_str(&args, "target")?;
            let out = arg_path(&args, "out")?;
            let name = arg_opt_str(&args, "name");
            let (envelope, _code) = commands::verify_envelope(&file, &target, &out, name)?;
            serde_json::to_string_pretty(&envelope)?
        }
        "graph" => {
            let file = arg_path(&args, "file")?;
            let format = args.get("format").and_then(Value::as_str).unwrap_or("json");
            match commands::graph_document(&file, format)? {
                Some(text) => text,
                None => anyhow::bail!(
                    "the program has compile errors; call `check` first for diagnostics"
                ),
            }
        }
        "explain" => {
            let code = arg_str(&args, "code")?;
            commands::explain_document(&code)?
        }
        "describe" => serde_json::to_string_pretty(&crate::describe::build())?,
        "fix" => {
            let file = arg_path(&args, "file")?;
            let code = arg_str(&args, "code")?;
            let index = args.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
            let apply = args.get("apply").and_then(Value::as_bool).unwrap_or(false);
            let (_, _, _sources, diags) = commands::front_end_quiet(&file)?;
            let diag = diags
                .iter()
                .find(|d| <&'static str>::from(d.code) == code)
                .ok_or_else(|| {
                    anyhow::anyhow!("no `{code}` diagnostic reported for {}", file.display())
                })?;
            let fix = diag.fixes.get(index).ok_or_else(|| {
                anyhow::anyhow!(
                    "`{code}` offers no fix at index {index} (has {})",
                    diag.fixes.len()
                )
            })?;
            let src = std::fs::read_to_string(&file)
                .with_context(|| format!("cannot read {}", file.display()))?;
            let patched = fix.apply(&src);
            if apply {
                std::fs::write(&file, &patched)
                    .with_context(|| format!("cannot write {}", file.display()))?;
                let (envelope, _code) = commands::check_envelope(&file)?;
                serde_json::to_string_pretty(&json!({
                    "applied": true,
                    "fix": fix.title,
                    "recheck": envelope,
                }))?
            } else {
                serde_json::to_string_pretty(&json!({
                    "applied": false,
                    "fix": fix.title,
                    "patched_source": patched,
                }))?
            }
        }
        "diff_semantic" => {
            let file = arg_path(&args, "file")?;
            let against = arg_opt_str(&args, "against").map(PathBuf::from);
            let baseline = arg_opt_str(&args, "baseline").map(PathBuf::from);
            let deny_breaking = args
                .get("deny_breaking")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let (envelope, _code) = commands::diff_semantic_envelope(
                &file,
                against.as_deref(),
                baseline.as_deref(),
                deny_breaking,
            )?;
            serde_json::to_string_pretty(&envelope)?
        }
        "rename" => {
            let file = arg_path(&args, "file")?;
            let target_file = arg_opt_str(&args, "target_file").map(PathBuf::from);
            let line = args.get("line").and_then(Value::as_u64).map(|v| v as u32);
            let column = args.get("column").and_then(Value::as_u64).map(|v| v as u32);
            let to = arg_opt_str(&args, "to");
            let old = arg_opt_str(&args, "old");
            let new_name = arg_opt_str(&args, "new_name");
            let apply = args.get("apply").and_then(Value::as_bool).unwrap_or(false);
            let result = crate::rename::rename_tool(
                &file,
                target_file.as_deref(),
                line,
                column,
                to.as_deref(),
                old.as_deref(),
                new_name.as_deref(),
                apply,
            )?;
            serde_json::to_string_pretty(&result)?
        }
        other => anyhow::bail!("unknown tool `{other}`"),
    };
    Ok(json!({"content": [{"type": "text", "text": text}], "isError": false}))
}

fn arg_str(args: &Value, key: &str) -> Result<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| anyhow::anyhow!("missing required argument `{key}`"))
}

fn arg_opt_str(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn arg_path(args: &Value, key: &str) -> Result<PathBuf> {
    arg_str(args, key).map(PathBuf::from)
}
