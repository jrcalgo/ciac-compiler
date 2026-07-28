# Authoring CIaC

*Reader: a builder setting up editor support, or reusing blueprints
across projects. [docs/guide/01-first-service.md](guide/01-first-service.md)
is the narrative walkthrough this page assumes as background;
this page is the reference for the editing/reuse tooling that
walkthrough only touches briefly.*

Everything in this page is about the minutes *before* and *during*
`ciac build`: starting a project, editing `.ciac` with live feedback,
keeping a running stack in sync while you iterate, and reusing
blueprints across projects. Once you're generating and iterating on
output, see [docs/dev-loop.md](dev-loop.md); for an agent working in
this loop instead of a human, see [docs/agents.md](agents.md).

## Start from a scaffold: `ciac new`

```sh
ciac new my-app                      # the `crud` template
ciac new my-app --template kafka     # or: crud | multi-service | kafka | minimal
cd my-app && ciac check main.ciac
```

Each template is a **checked-in example embedded verbatim at compile
time** — `crud` is `examples/crud-notes.ciac`, `multi-service` is
`examples/inventory-system.ciac`, `kafka` is
`examples/kafka-pipeline.ciac`, `minimal` is `examples/ping.ciac` —
so a scaffold can never drift from a shape the test suite already
compiles, generates, and (for the multi-service one) system-verifies
in CI. The scaffold is `main.ciac` plus a README with the next
commands to run; `ciac new` refuses a non-empty directory and there
is deliberately no `--force` (regeneration workflows belong to
`ciac build`).

## Live diagnostics while editing: `ciac lsp`

`ciac lsp` speaks the Language Server Protocol over stdio. It
publishes the **same diagnostics `ciac check` prints** (same codes,
same spans, resolved through the same line/column pipeline as
`--json`) on file open and save, plus:

- **hover** over any keyword, capability, provider, or declared name
  — providers carry their per-target support notes (every provider
  generates on both bundled targets as of v0.13; see
  [docs/language.md](language.md)'s support table);
- **completion** for keywords, capabilities, providers, builtin
  pipeline steps, and the names your own file declares.

Diagnostics refresh on *save*, not on every keystroke: imports
resolve against the filesystem exactly as the CLI resolves them, and
resolving unsaved buffers would need a VFS layer that remains
deliberately out of scope. Rename (v0.18) and structured quick-fixes
on mechanically-fixable diagnostics (v0.15 M7) are *not* out of
scope, despite an earlier version of this page saying otherwise —
`ciac lsp` surfaces both; 29UpdatePlan.md's own M8 milestone is
rewriting this section to name the LSP's complete, current
capability set (adding go-to-definition alongside them), so treat
the exact feature list here as provisional until that milestone
lands. `ciac lsp` and `ciac describe` (v0.13, see
[docs/agents.md](agents.md)) render their vocabulary from the same
table in `crates/ciac/src/vocab.rs`, so hover text and the
machine-readable registry can't drift apart.

### Editor setup

**Neovim** (0.10+, no plugins needed):

```lua
vim.filetype.add({ extension = { ciac = "ciac" } })
vim.api.nvim_create_autocmd("FileType", {
  pattern = "ciac",
  callback = function()
    vim.lsp.start({ name = "ciac", cmd = { "ciac", "lsp" } })
  end,
})
```

**Helix** (`~/.config/helix/languages.toml`):

```toml
[language-server.ciac]
command = "ciac"
args = ["lsp"]

[[language]]
name = "ciac"
scope = "source.ciac"
file-types = ["ciac"]
language-servers = ["ciac"]
```

**VS Code**: `editors/vscode/` (v0.13 M6) is a checked-in extension
bundling the TextMate grammar below and an LSP client that launches
`ciac lsp` for `.ciac` files — no generic LSP-client extension needed.
Load it unpacked for local use:

```sh
cd editors/vscode
npm install
code --extensionDevelopmentPath="$PWD" .
```

or package a `.vsix` with `vsce package` (see
`editors/vscode/README.md` for the packaging caveats — `vsce` pulls
from npm, which isn't always reachable from every environment this
repo is built in, so packaging is a documented manual step rather
than a CI hard-requirement).

### Syntax highlighting

`editors/ciac.tmLanguage.json` is a self-contained TextMate grammar
(comments, strings, keywords, capability kinds, the provider
registry, primitive types, HTTP methods, builtin steps, declaration
names) — the same file the VS Code extension bundles. Any
TextMate-compatible editor can consume it directly. A Tree-sitter
grammar was considered for v0.12 and deliberately deferred — the
TextMate file covers highlighting today with zero codegen toolchain.

## Reuse across projects: `registry:` imports

```ciac
import "registry:acme/blueprints/notes/crud.ciac@v1.2.0";
```

resolves to an HTTP GET of
`{base}/acme/blueprints/v1.2.0/notes/crud.ciac`, where `{base}` is
`$CIAC_REGISTRY` (default `https://raw.githubusercontent.com`). **A
plain git-hosted directory of `.ciac` files is the registry** — no
index service, no publish step, no namespace ownership: push
blueprints to a repo, tag it, and the tag is the version.

- **Pin an immutable ref.** Fetches cache at
  `$XDG_CACHE_HOME/ciac/registry/<sha256(url)>.ciac` (`~/.cache`
  fallback) and a cache hit never touches the network — pinning a
  tag or commit makes resolution reproducible and offline after the
  first fetch. Pinning a branch means whatever the branch pointed at
  when first cached.
- **Trust boundary**: fetched content is plain `.ciac` source flowing
  through the identical parse → blueprint-expansion → validation path
  as local files and `std/` imports. Importing a registry blueprint
  grants no execution beyond what `ciac build` already does with
  local source — but it *is* someone else's architecture description;
  read it (it's cached as plain text) like you'd read any dependency.
- Remote blueprints may themselves import `std/...` or further
  `registry:...` specs, but not local files — a remote file has no
  local directory to resolve them against.

`std/` imports (`import "std/crud.ciac";`) remain the embedded,
offline standard library; the registry is for everything the std
library shouldn't absorb.
