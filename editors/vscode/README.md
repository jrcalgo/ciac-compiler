# CIaC for VS Code

Syntax highlighting (the checked-in `../ciac.tmLanguage.json` grammar)
plus a Language Server Protocol client that launches `ciac lsp` for
every open `.ciac` file — live diagnostics, hover, and completion, the
same as any other LSP-backed language extension. `ciac` itself must be
on `$PATH` (or set `ciac.serverPath` in your VS Code settings).

Declaration keywords (`service`, `worker`, `crud`, ...) complete as
real tab-stopped snippets, not just bare words — type `worker` and tab
through `Name`/`Stream` — via `snippets/ciac.json` (v0.27 M7), generated
from the same `vocab.rs` table the LSP's own completion reads, with a
test (`crates/ciac/tests/snippets_cli.rs`) holding the two in sync.
Hovering a capability (`db`, `cache`, ...) shows its providers,
per-target support, verbs, and `ciac sim` behavior in one block, not
just a one-line description.

## Load it unpacked (no packaging needed)

```sh
cd editors/vscode
npm install
code --extensionDevelopmentPath="$PWD" .
```

This opens a new VS Code window with the extension active — the
fastest loop for trying it or developing it further.

## Package a `.vsix`

```sh
cd editors/vscode
npm install
npx vsce package
```

produces `ciac-0.14.0.vsix`, installable via `code --install-extension
ciac-0.14.0.vsix` or VS Code's "Install from VSIX" command.

**Disclosed**: `vsce` and `vscode-languageclient` are npm packages;
whether `npm install`/`vsce package` succeed depends on npm registry
reachability from wherever you run this, which CI's default runners
have and some sandboxed/offline environments don't. This repo's CI
does not currently gate on a `.vsix` build — packaging is a documented
manual step, not a build requirement, so a network-restricted
environment can still build and test the compiler itself with no
impact.

## What's not here

Marketplace publishing (`vsce publish`) is out of scope — this is a
buildable artifact, not a hosted listing. Tree-sitter-based semantic
highlighting was considered and deferred (see
[`docs/authoring.md`](../../docs/authoring.md)); the TextMate grammar
covers highlighting today.
