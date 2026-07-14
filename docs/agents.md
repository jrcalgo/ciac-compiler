# The agent front door

Everything a human does with the `ciac` CLI, an agent can do too — but
parsing human-formatted stdout is brittle, and speaking LSP just to
look up what a keyword means is a lot of protocol for a lookup. v0.13
M5 adds a machine-native path alongside the human one: `--json`
envelopes, `ciac describe`, and `ciac mcp`, all rendering from the
same tables the CLI and `ciac lsp` already use — nothing here is a
second copy of the truth that can drift from the first.

## `--json`: one envelope per invocation

`ciac check|build|diff|verify --json` each print exactly one JSON
document on stdout; human narration stays on stderr, so the two never
interleave in a captured pipe.

```json
{
  "json_version": 2,
  "command": "check",
  "success": true,
  "diagnostics": []
}
```

Diagnostics carry the resolved file/line/column the human-mode
`ciac check` renders through `ariadne` — same codes, same spans.
`diff --json` additionally carries `entries`: the regeneration plan as
data (status per path, optional sidecar, optional unified diff text
under `--patch`).

Mechanical, unambiguous diagnostics (`json_version` bumped 1 → 2 for
this, v0.15 M7) also carry `fixes`: `[{title, edits: [{file, line,
column, end_line, end_column, replacement}]}]` — never applied by
`check` itself, but exactly the edits `ciac lsp`'s quick-fix and `ciac
mcp`'s `fix` tool apply. An agent's tightest loop is `ciac check
--json` → apply an offered fix mechanically → re-check; no fix is
offered unless applying it is proven to clear that diagnostic (see
`tests/tests/fixes.rs`'s property test over the negative-fixture
corpus).

## `ciac describe`: the language as one document

```sh
ciac describe
```

prints one versioned JSON document naming everything the language and
CLI expose: keywords, capabilities (with their providers), providers
(with per-target support), field types, builtin pipeline steps,
declaration kinds, every `CIAC` error code (with severity and title),
and the scaffold templates `ciac new` offers. It's the same
`crates/ciac/src/vocab.rs` table `ciac lsp` renders hover and
completion from — a provider graduating from one target to both is one
edit in one file, not a hover string and a doc table that can quietly
disagree.

## `ciac mcp`: a Model Context Protocol server

```sh
ciac mcp
```

runs a hand-rolled JSON-RPC 2.0 server over stdio, newline-delimited
(the MCP stdio transport — distinct from `ciac lsp`'s Content-Length
framing, which is LSP's own wire format). It implements `initialize`,
`notifications/initialized`, `tools/list`, and `tools/call`, exposing:

| Tool | Mirrors |
|------|---------|
| `check` | `ciac check --json` |
| `build` | `ciac build --json` (always regenerates in place — never `--force`/`--adopt`) |
| `diff` | `ciac diff --json` |
| `verify` | `ciac verify --json` (static check only — no `--system`/`--live`; those boot Docker and belong to a human at a terminal) |
| `graph` | `ciac graph` |
| `explain` | `ciac explain` |
| `describe` | `ciac describe` |
| `fix` | Applies a diagnostic's offered fix (v0.15 M7): `{file, code, index?, apply?}` — dry-run preview by default, `apply: true` writes the patched source and returns the re-checked envelope |
| `diff_semantic` | `ciac diff --semantic --json` (v0.18 M7) — the architecture changelist, classified `Breaking`/`Additive`/`Internal` |
| `rename` | `ciac rename` (v0.18 M7): position-based (`target_file`/`line`/`column`/`to`) or qualified (`old`/`new_name`) lookup, dry-run preview by default, `apply: true` writes the files. Deliberately source-only — it never replays a `--out` tree's regeneration, unlike the CLI's own `--out` support; a human reviews and applies that separately. See [docs/evolution.md](evolution.md) |

Every tool result carries the same JSON envelope (or `graph`/`describe`
document) as one text content block — an MCP client sees exactly what
a human running `--json` on the command line would, because both paths
call the same envelope-returning functions in `crates/ciac/src/commands.rs`
(`check_envelope`, `build_envelope`, `diff_envelope`, `verify_envelope`,
`graph_document`, `explain_document`).

Point an MCP-capable client at `ciac mcp` as the server command; no
arguments, no config file — the tool list and schemas are discovered
via `tools/list`.

`ciac lsp` gained the editor-native equivalent of the `rename` tool in
the same milestone: `textDocument/prepareRename` and
`textDocument/rename`, resolving through the identical whole-program
symbol index and returning a multi-file `WorkspaceEdit` the editor
applies. See [docs/evolution.md](evolution.md) for the rename engine
itself.

## `AGENTS.md` everywhere

- `ciac new` scaffolds an `AGENTS.md` into the new project directory,
  pointing at the check → build → verify loop and this document.
- Every `ciac build`/`diff`/`verify` writes an `AGENTS.md` into the
  *generated* output tree — regenerated like any other compiler-owned
  file — explaining that tree's own owned-vs-seeded split and where
  handler logic goes.
- This repository's own root [`AGENTS.md`](../AGENTS.md) is about
  working on the compiler itself.

None of the three is hand-maintained prose that can go stale next to
the code it describes in the way a wiki page can: the generated ones
regenerate with the project, and this doc and the root `AGENTS.md`
are the kind of thing CI's doc-consistency checks and `ciac describe`
existing at all are meant to keep honest.
