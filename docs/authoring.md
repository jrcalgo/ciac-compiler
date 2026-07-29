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
`--json`) on file open and save, *and* — since `29UpdatePlan.md` M8 —
on a **debounced edit**: a short pause after your last keystroke
reparses the dirty, unsaved buffer and republishes, instead of waiting
for the next save. This is the arc's final editor claim; the full
capability set, current as of this milestone:

- **diagnostics** on open, save, and debounced didChange (above);
- **hover** over any keyword, capability, provider, or declared name.
  A capability hover (v0.27 M7) is a structured block — its providers,
  per-target support (every provider generates on all five bundled
  targets), the handler-body verbs it exposes, and how `ciac sim`
  treats it — not just a one-line description;
- **completion** for keywords, capabilities, providers, builtin
  pipeline steps, and the names your own file declares. A declaration
  keyword (`service`, `worker`, `crud`, ...) completes as a real
  tab-stopped **snippet** (v0.27 M7), not just the bare word;
- **rename** (v0.18) — `prepareRename`/`rename` over the whole
  program, cross-file through `import`s, with the same collision/
  editability checks `ciac rename` enforces on the CLI;
- **go-to-definition** (v0.27 M8) — a thin projection of the same
  whole-program resolver rename already rides: the identifier under
  the cursor jumps to its declaration site, same-file or across an
  `import`;
- **quick-fixes** (v0.15 M7, widened at v0.27 M8) on the mechanically
  fixable diagnostics — missing capability, unknown provider/stream/
  table/capability-instance/attribute name (nearest-match rename,
  offered only when one candidate is a plausible typo, never a
  guess), and OAuth2's missing-`issuer` case. See
  [docs/errors.md](errors.md) for exactly which codes carry one.

One disclosed gap, unchanged since v0.12: only the *entry file you
have open* gets its unsaved edits reparsed (`load_with_overlay`
substitutes its dirty buffer only) — anything it `import`s still
resolves from disk, since only the document actually open in the
client has unsaved content to substitute. A full cross-file VFS
overlay remains out of scope. `ciac lsp` and `ciac describe` (v0.13,
see [docs/agents.md](agents.md)) render their vocabulary — including
snippets — from the same table in `crates/ciac/src/vocab.rs`, so
hover text, completions, and the machine-readable registry can't
drift apart.

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
