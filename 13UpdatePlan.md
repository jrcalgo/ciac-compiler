# CIaC v0.13 — Friction: the Inner Loop and the Front Door (roadmap forecast)

> Forecast document. Assumes v0.9 (verification), v0.10 (legibility),
> v0.11 (provider/deployment breadth), and v0.12 (authoring &
> ecosystem) have landed. Direction-setting; the v0.13 planning pass
> finalizes the `ciac dev` restart semantics and the Kafka-on-Rust
> dependency choice.

## The gap this version closes

After v0.12 the pipeline from *idea → validated architecture →
generated, proven system* is strong. The remaining time now leaks in
two places that have nothing to do with the language:

1. **The inner development loop is manual.** Edit `main.ciac` →
   `ciac build` → `docker compose up` → poke → `docker compose down` →
   edit again. Every piece of machinery a watch loop needs already
   exists — the regeneration engine with sidecar safety (v0.6), the
   compose assembler (v0.9 M1), health probing (v0.9 M3), the resolved
   import set in `SourceMap` (v0.8 M1) — but nothing composes them
   into the second-scale loop every modern toolchain has. This is the
   single largest recurring tax on someone actually *using* ciac all
   day.

2. **First contact is "clone the repo and cargo install".** No release
   binaries, no install script, no published editor extension (the
   v0.12 grammar and LSP exist but require hand-wiring). For coding
   agents the equivalent wall is subtler: the `--json` contract is
   complete, but an agent must be *told* how to shell out to it — there
   is no MCP surface and no `AGENTS.md` convention in scaffolds or
   generated output.

And one gap that is about the language keeping its own promise:
**Rust is a second-class first-class backend.** `db MySQL` and
`queue Kafka` generate on Python and gate on Rust with `CIAC0011`.
Every "Python only" cell in the provider table is an asterisk on the
core claim ("if it builds, the generated system actually does it").
Parity is unglamorous, and it is a trust product.

**v0.13 theme: the existing product, minus every wait and every wall.
No new language surface — faster loops, full target parity, zero-
friction installation, and a machine-native front door for agents.**

## Pillar 1 — Target parity: MySQL and Kafka on Rust

- **`db MySQL` on Rust.** sqlx pools are typed per database, so this
  is a generation-time branch, not a runtime abstraction: when the
  instance's engine is `mysql`, `state.rs`/`config.rs`/repository
  modules emit `sqlx::MySqlPool` and `mysql://` URLs instead of
  `PgPool`/`postgres://` — the engine is known statically per
  instance, exactly like the Python backend's asyncpg/aiomysql split
  (v0.11 M1). Migration SQL already avoids engine-specific types
  (TEXT, not VARCHAR) after the v0.11 live-verification fix; the Rust
  migration runner gains the same per-engine connect branch.
- **`queue Kafka` on Rust.** Dependency decision to finalize in
  planning: `rdkafka` with the `cmake-build`/vendored feature (full
  consumer-group semantics, native build chain cost, pinned in the
  generated Cargo.toml) versus a pure-Rust client. Default position:
  rdkafka vendored — correctness of consumer groups outweighs build
  time, and the generated Dockerfile absorbs the toolchain cost in
  one place. Topics and group ids reuse the same
  `<service>.<stream>` / per-worker names the Python backend already
  emits (v0.11 M3), so a mixed-target system shares one broker
  correctly.
- Gates removed from `RustBackend::supports()`; `tests/tests/
  gating.rs` flips from "kafka gates on rust" to "kafka generates on
  both"; golden snapshots for `mysql-notes` and `kafka-pipeline` gain
  Rust trees; the CI `generated-system` matrix adds
  `rust × {mysql-notes, kafka-pipeline}`.
- Documentation: the multi-provider table in `docs/language.md` loses
  its last per-target asterisks (except external backends, which stay
  honestly disclosed in `docs/external-backends.md`).

## Pillar 2 — `db SQLite`: the zero-container database

- `use { db SQLite; }` — Python via `sqlite+aiosqlite` (SQLAlchemy
  async, same session machinery as Postgres/MySQL), Rust via sqlx's
  `SqlitePool`. The database is a file under the generated project's
  `data/` directory; compose mounts it as a volume instead of running
  a container — the first capability whose dev story requires no
  Docker at all.
- Why it earns a registry slot now: it collapses `ciac new` →
  *running typed CRUD* to seconds (no image pulls), it is the natural
  companion to Pillar 3's watch loop, and it is a real production
  answer for single-node systems.
- Disclosed limits, enforced or documented rather than discovered:
  `verify --system`'s direct-connection capability round-trip connects
  to the SQLite file rather than a host port; multi-*service* systems
  sharing one SQLite instance are rejected at sema (each service gets
  its own file — SQLite is per-deployable by design, a new diagnostic
  with a clear message).

## Pillar 3 — `ciac dev`: the watch loop

```sh
ciac dev main.ciac --target python --out ./build
```

- Watches the entry file **and the full resolved import set** (the
  `SourceMap` already enumerates every file `load` pulled in,
  including nothing for `std/`/cached `registry:` imports) plus the
  seeded/extern logic files in the output tree. File watching via the
  `notify` crate (no `unsafe`), debounced; a polling fallback flag for
  filesystems where inotify misbehaves.
- On change: re-run the front end. **Compile errors never kill the
  loop or the running stack** — diagnostics render (ariadne, colored)
  and the last good system keeps serving. On a clean compile:
  regenerate through the existing `plan_regeneration`/`apply` path
  (sidecar discipline intact — `ciac dev` can never clobber user
  edits that `ciac build` would have protected), then restart only
  the services whose files actually changed
  (`docker compose up -d --build <service>`), then re-probe
  `/health` (v0.9 M3 prober) and print the per-service up/down line.
- First run boots the full stack; Ctrl-C tears it down unless
  `--keep` (same flag semantics verify already has). `--no-docker`
  runs generate-only watch mode (regenerate + re-verify diagnostics
  on save) for pairing with a hand-run process or SQLite-only
  programs.
- Cut line: no in-process hot reload of handler code (that's the app
  framework's job, e.g. uvicorn --reload inside the container is a
  planning-pass option for the Python target), no browser
  auto-refresh, no TUI dashboard. `ciac dev` is compose orchestration
  plus the compiler in a loop — nothing it does is new machinery.

## Pillar 4 — Releases and the editor extension

- **Release engineering**: a `release.yml` GitHub Actions workflow
  building `ciac` for linux-x86_64/linux-aarch64/macos-aarch64/
  macos-x86_64/windows-x86_64 on tag push, attaching binaries to a
  GitHub Release; a checked-in `install.sh` (download latest release
  for the detected platform into `~/.local/bin`); README installation
  section rewritten to lead with it. Homebrew tap/scoop manifest are
  stretch items — the direct-download path is the milestone.
- **VS Code extension** (`editors/vscode/`): `package.json`
  contributing the language id, the existing TextMate grammar, and an
  LSP client (`vscode-languageclient`) that launches `ciac lsp` from
  PATH. CI job packages the `.vsix` artifact; Marketplace publishing
  is a manual step documented in `editors/vscode/README.md` (no
  credentials in the repo).

## Pillar 5 — The agent front door: `ciac mcp`, `ciac describe`, AGENTS.md

- **`ciac mcp`**: an MCP server over stdio exposing tools
  `check`, `build`, `diff`, `verify`, `graph`, `explain`, `describe`
  — each a thin wrapper over the existing command internals with the
  v0.10 `--json` envelope as the tool result payload. Implementation
  reuses the JSON-RPC framing discipline the v0.12 LSP already
  proved; no async runtime, no new protocol machinery beyond MCP's
  initialize/tools-list/tools-call subset. Registering ciac with any
  MCP-speaking agent becomes one config line.
- **`ciac describe`**: one JSON document (versioned like the `--json`
  envelope) enumerating the machine-facing registry an agent
  otherwise scrapes from docs: capabilities and their providers with
  per-target support, attributes per declaration kind with types and
  defaults, primitive field types, builtin pipeline steps, error
  codes with one-line summaries, available `ciac new` templates.
  Backed by the same static tables the LSP hover uses (single source
  of truth — the tables move to a shared module rather than being
  duplicated).
- **AGENTS.md**: emitted by `ciac new` (alongside the scaffold
  README) and by `ciac build` into generated projects — the regen
  rules stated for a machine audience: which files are owned vs
  seeded, that logic goes in `logic/`/extern handlers, that
  `ciac verify --json` is the truth signal, that owned files must
  never be hand-edited (sidecars explain conflicts). Also a top-level
  `AGENTS.md` in the ciac repo itself.

## Secondary items

- `ciac explain <code> --json` (structured error-code explanation —
  trivially derived from the docs/errors.md source of truth).
- `ciac new --list-templates` (agents and scripts shouldn't parse
  `--help` prose).
- Docs: `docs/dev-loop.md` (ciac dev semantics, what restarts when),
  installation section rewrite, `docs/agents.md` (MCP setup,
  describe/json contracts in one place).

## Milestones

1. **M1 — Rust MySQL parity**: sqlx MySql generation branch, gate
   removal, goldens, gating-test flip, CI matrix row, live proof
   against the apt MariaDB (`verify -t rust` on `mysql-notes`).
2. **M2 — Rust Kafka parity**: rdkafka(vendored) generation, gate
   removal, goldens, CI row; broker-dependent behavior delegated to
   the CI `generated-system` job where no local broker exists
   (disclosed, same as v0.11 M3).
3. **M3 — `db SQLite`**: both backends + compose volume + migration
   dialect + per-deployable sema rule + example (`sqlite-notes.ciac`)
   + live proof (this one runs with no Docker at all).
4. **M4 — `ciac dev`**: notify-based watch over the resolved source
   set, error-tolerant recompile loop, sidecar-safe regen, per-service
   restart + health probe, `--keep`/`--no-docker`; live proof via a
   scripted edit-while-running session.
5. **M5 — agent front door**: `ciac describe`, `ciac mcp` (stdio,
   tools wrapping existing internals), AGENTS.md emission in scaffold
   and build output, `explain --json`; round-trip test speaking MCP
   to the real binary (the lsp_cli.rs pattern).
6. **M6 — releases + extension + docs**: release workflow +
   install.sh, `editors/vscode/` extension + packaged vsix CI
   artifact, docs (dev-loop/agents/installation), provider-table
   reconciliation, version 0.13.0, full verification, whole-version
   analysis.

## Risks

- **rdkafka's native build chain** (cmake, libsasl) can fail in
  environments the generated Dockerfile doesn't control (bare
  `cargo build` on a dev machine). Mitigation: vendored feature
  pinned, build prerequisites stated in the generated README, and the
  Dockerfile — which ciac does control — is the proven path in CI.
- **`ciac dev` restart correctness**: mapping changed generated files
  to the minimal service-restart set can be wrong in both directions.
  Mitigation: conservative default (any shared-file change restarts
  everything), per-service mapping only for per-service trees, and
  the health probe after every restart makes a wrong decision loud
  rather than silent.
- **Marketplace/registry publishing needs credentials** the repo must
  not hold. Mitigation: CI produces the artifacts (binaries, vsix);
  the publish step is documented and manual, and the milestone is
  the artifact, not the listing.
- **MCP SDK churn.** Mitigation: the tool surface is seven wrappers
  over stable internals; implement against the protocol subset
  directly (as the LSP did with lsp-server) rather than chasing a
  fast-moving SDK.

## After v0.13

The loop is seconds, installation is one command, both first-class
targets honor the full provider registry, and agents have a native
protocol instead of a shell contract. What remains is the deepest
question in the project: how much of a real system's *logic* can live
in the model itself — v0.14's subject.
