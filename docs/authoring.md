# Authoring CIaC (v0.12)

Everything in this page is about the minutes *before* `ciac build`:
starting a project, editing `.ciac` with live feedback, and reusing
blueprints across projects.

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
  — providers carry their per-target support notes (e.g. that
  `MySQL`/`Kafka` generate on Python but gate on Rust with CIAC0011);
- **completion** for keywords, capabilities, providers, builtin
  pipeline steps, and the names your own file declares.

Diagnostics refresh on *save*, not on every keystroke: imports
resolve against the filesystem exactly as the CLI resolves them, and
resolving unsaved buffers would need a VFS layer that is deliberately
out of v0.12's scope (as are rename, references, and code actions).

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

**VS Code**: there is no published extension yet; any generic
LSP-client extension works — point it at command `ciac`, args
`["lsp"]`, for language id/extension `ciac`. Pair it with the
TextMate grammar below.

### Syntax highlighting

`editors/ciac.tmLanguage.json` is a self-contained TextMate grammar
(comments, strings, keywords, capability kinds, the provider
registry, primitive types, HTTP methods, builtin steps, declaration
names). Any TextMate-compatible editor can consume it directly; in a
VS Code extension it slots in as a `grammars` contribution for scope
`source.ciac`. A Tree-sitter grammar was considered for v0.12 and
deliberately deferred — the TextMate file covers highlighting today
with zero codegen toolchain.

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
