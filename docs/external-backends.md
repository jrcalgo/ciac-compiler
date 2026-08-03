# External backends (v0.10)

*Reader: someone implementing a code-generation target outside the
compiler itself, in any language, against a versioned wire protocol.*

A CIaC backend doesn't have to be a Rust crate linked into the
compiler. `ciac build --target <name>` falls back to running an
executable called **`ciac-backend-<name>`** found on `$PATH` when
`<name>` isn't a built-in target (`python`, `rust`, `typescript`,
`go`) — the same seam protobuf's `protoc-gen-<lang>` plugins use. The
backend can be written in any language; the only contract is JSON over
stdin/stdout.

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
5. Both halves carry `protocol_version` (currently `2`); `ciac`
   refuses a response with a mismatched version rather than guessing.

   **v2 migration note (v0.22 M2):** `FieldCtx` dropped its
   `py_type`/`py_out_type`/`rust_type`/`db_rust_type` fields — those
   were always host-language spellings the bundled Python/Rust
   backends rendered for themselves, and now do so as minijinja
   filters over `type_kind` instead of precomputed wire fields (see
   `22UpdatePlan.md` Pillar 2). `type_kind` itself is unchanged and was
   already the documented, recommended field to map (next paragraph) —
   an external backend already following that advice needs no changes
   at all; one still reading the removed fields needs to switch to
   `type_kind` the same way the in-repo Go reference backend already
   does.

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
go build -C backends/go -o /tmp/bin/ciac-backend-go-external-demo .
PATH="/tmp/bin:$PATH" ciac build --target go-external-demo -o ./out examples/single-service/ping.ciac
```

Its `main.go` shows the whole authoring pattern in ~350 lines: declare
Go structs mirroring only the request fields you consume
(`encoding/json` ignores the rest), read stdin, refuse what you don't
support with a clear stderr message, generate, write the response.

**Two Go code generators, reconciled (`24UpdatePlan.md` M1).** This
repo now contains two: the module above (id `go-external-demo`,
renamed from its original `go` at this milestone) and
`crates/ciac-backend-go`, an internal crate exactly like
`ciac-backend-python`/`-rust`/`-ts`, which took the `go` id instead.
They are not competing implementations — they demonstrate different
things. `backends/go/` stays exactly what it always was: the wire
protocol's living documentation and test fixture, at the protocol's
own honest capability level (no typed handlers, no validators, no
simulation), updated only when the protocol itself changes.
`crates/ciac-backend-go` is the product: full capability parity with
the other bundled targets as `24UpdatePlan.md`'s milestones land. The
rename is the one user-visible change this split makes — anyone
scripting `--target go` against the old external demo now reaches the
bundled backend instead, and `ciac targets` never listed the external
demo in the first place (external backends are discovered only when
explicitly named, per the scope notes below), so there is no listing
to update.

## Testing a backend without `$PATH` games

`ExternalBackend::with_executable` (used by
`tests/tests/external_backend.rs`) points at a specific binary
directly, so integration tests never mutate process-wide env vars.
`tests/bin/stub-backend.rs` is a scenario stub (echo/fail/garbage/
bad-version) exercising every failure path `ciac` handles:
spawn-failure, non-zero exit, malformed JSON, version mismatch.

## Formatter-shelling backends must batch

If your backend shells out to a real formatter (rather than hand-
tuning its own templates to already be canonical, as the in-tree
Rust/Python backends do), **invoke it once per `generate()` call, not
once per file.** `ciac-backend-java` didn't, for its first five
milestones (`25UpdatePlan.md` through `29UpdatePlan.md`): every
generated `.java` file spawned its own `google-java-format` process, a
fresh JVM cold-started and paid `javac`-internals classload cost each
time — measured at ~0.51s per file regardless of size. A 40-file
`order-system.ciac` Java build cost ~20s in formatter spawns alone,
against low hundreds of *milliseconds* for every other target.
`30UpdatePlan.md` M2 fixed it: collect every file of the relevant
extension, write them to one scratch directory (at their real
relative path — verified live that `google-java-format` does not care
whether a file's on-disk location matches its own package
declaration), and run the formatter **once**, over the whole batch,
via whatever list-of-files mode it supports
(`google-java-format -i @argfile`; `gofmt -w <paths...>` for Go, ported
onto the same seam at M3 since the pattern matters even where the
absolute cost — ~2ms per `gofmt` spawn — does not).

**In-tree backends** share this via `ciac_codegen::format_batch::
format_batch` — see `ciac-backend-java` and `ciac-backend-go`'s own
`format_all_java`/`format_all_go` for the two worked examples (jar
`-i @argfile` vs. plain positional file args). **Out-of-tree backends
cannot import this crate-private helper** (the wire protocol is
process-boundary, not a Rust API — see "The wire contract" above) and
must implement the same one-invocation-per-batch discipline
themselves if their language's canonical formatter is a real,
separate program. The lesson generalizes past formatters: any
external tool your backend shells out to per file pays its own
startup cost that many times, and that cost is invisible until
someone measures the whole test suite's wall time and asks why.

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
