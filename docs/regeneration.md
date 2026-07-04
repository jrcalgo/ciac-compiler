# Regeneration

CIaC v0.6 treats generated projects as living artifacts. Re-running
`ciac build` into an existing output directory preserves user-owned code,
detects drift, and refuses to overwrite modified compiler-owned files.

## Manifest

Every build writes `.ciac/manifest.json` at the output root:

```json
{
  "compiler_version": "0.6.0",
  "source_hash": "sha256...",
  "target": "python",
  "files": {
    "app/main.py": { "role": "owned", "hash": "sha256..." },
    "app/services/store.py": { "role": "seeded", "hash": "sha256..." }
  }
}
```

Hashes are over the generated file content. The map is serialized in
sorted order so manifest bytes are deterministic.

## File roles

- **Owned** files are compiler-owned wiring: app assembly, routes,
  workers/jobs, clients, schemas, config, compose files, and docs.
  Regeneration rewrites them only when the on-disk file still matches the
  previous manifest hash.
- **Seeded** files are generated once and then owned by the user. Handler
  stubs in `app/services/` and `src/services/` are seeded.

## Conflict workflow

On rebuild, CIaC compares three states: the old manifest hash, the
current on-disk file, and the newly generated content.

| Status | Meaning |
|--------|---------|
| `unchanged` | disk already matches generated output |
| `update` | owned file was untouched and can be rewritten |
| `new` | generated file is missing and can be written |
| `conflict` | owned file was edited; CIaC writes `<file>.ciac-new` and fails with `CIAC0033` |
| `seeded-drift` | seeded file exists but the generated seed changed; CIaC writes `<file>.ciac-new` and warns with `CIAC0034` |
| `orphan` | a previously generated file is no longer produced and was left in place (`CIAC0035`) |

CIaC does not attempt textual merging in v0.6. Reconcile sidecars
manually, then rebuild.

## Commands

```sh
ciac build app.ciac --target python --out ./app
ciac diff app.ciac --target python --out ./app --patch
ciac verify app.ciac --target python --out ./app
```

Use `--adopt` once for a pre-v0.6 generated tree. Existing files are
preserved and generated replacements are written as `.ciac-new` sidecars,
then a manifest is created. Use `--force` for a blank-slate overwrite.
