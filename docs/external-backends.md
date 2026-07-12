# External backends (v0.10)

A CIaC backend doesn't have to be a Rust crate linked into the
compiler. `ciac build --target <name>` falls back to running an
executable called **`ciac-backend-<name>`** found on `$PATH` when
`<name>` isn't a built-in target (`python`, `rust`) — the same seam
protobuf's `protoc-gen-<lang>` plugins use. The backend can be written
in any language; the only contract is JSON over stdin/stdout.

## The wire contract

1. `ciac` compiles and validates the program, builds the
   language-neutral `SystemModel`, wraps it in a `CodegenRequest`, and
   writes it as one JSON document to the child's **stdin** (then
   closes it).
2. The backend writes a `CodegenResponse` as one JSON document to its
   **stdout**: the generated file tree (`path`, `content`, `role:
   "owned" | "seeded"`) plus optional `notes`.
3. **stderr is inherited** — anything the backend logs shows up live
   in `ciac`'s own output, the same treatment `docker compose`/`uv`
   subprocesses get.
4. Refusals are loud: anything the backend can't handle (a capability
   it doesn't support, a protocol version it doesn't speak) should be
   a message on stderr and a **non-zero exit**. `ciac` reports it and
   fails the build; there is no capability pre-negotiation.
5. Both halves carry `protocol_version` (currently `1`); `ciac`
   refuses a response with a mismatched version rather than guessing.

The full schema of both payloads is published:

- `ciac codegen-schema` prints it (JSON Schema, derived from the same
  Rust types that serialize the real payloads);
- [`protocol-schema.json`](./protocol-schema.json) is that output
  checked in, held identical by an integration test;
- `ciac codegen-request <file> --target <name>` dumps the exact
  request a given program produces, for inspection or fixture-making.

Field types come with a language-neutral `type_kind`
(`str`/`int`/`float`/`bool`/`uuid`/`timestamp`/`json`/`enum` with the
generated enum name and variants) — map that, not the host-specific
`py_type`/`rust_type` strings, and treat an unrecognized kind as an
error (a newer `ciac` may be speaking a newer contract).

## Worked example

[`backends/go/`](../backends/go/) is a real, standalone Go module with
zero Rust linkage — deliberately narrow (one service, one record, one
api, no capabilities), built to prove the seam rather than to be a
full third backend:

```sh
# from the repo root
go build -C backends/go -o /tmp/bin/ciac-backend-go .
PATH="/tmp/bin:$PATH" ciac build --target go -o ./out examples/ping.ciac
```

Its `main.go` shows the whole authoring pattern in ~350 lines: declare
Go structs mirroring only the request fields you consume
(`encoding/json` ignores the rest), read stdin, refuse what you don't
support with a clear stderr message, generate, write the response.

## Testing a backend without `$PATH` games

`ExternalBackend::with_executable` (used by
`tests/tests/external_backend.rs`) points at a specific binary
directly, so integration tests never mutate process-wide env vars.
`tests/bin/stub-backend.rs` is a scenario stub (echo/fail/garbage/
bad-version) exercising every failure path `ciac` handles:
spawn-failure, non-zero exit, malformed JSON, version mismatch.

## Scope notes (disclosed)

- **No timeout**: a hung backend hangs the build.
- **No capability negotiation**: `supports()` is always `true` for
  external targets; refuse at generate time instead.
- **No `ciac targets` listing**: external backends are discovered only
  when explicitly named.
- **Multi-service composition**: an external backend receives the
  whole system but compose/k8s assembly currently assumes the
  built-in backends' project layout — a real third backend today is
  proven standalone, not as one service among Python/Rust ones.
