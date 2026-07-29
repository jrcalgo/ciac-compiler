# Guide 1 — Your first service

*Reader: builder, first hour. Time: ~20 minutes. You need: a
terminal and nothing else — no database, no broker, no Docker.*

This guide, and the two after it, build one continuous example: a
`Ping` service that starts as a single echo endpoint and ends up with
real persistence, a transaction, a stream, and a worker. Every command
below is executed for real by `scripts/check-guides.sh` against the
real binary — if a block here stopped working, this guide's own CI job
would go red before you ever saw it.

## 1. Install

<!-- ciac-verify:skip id=install reason="needs a cut GitHub release; the harness already has ciac on PATH from its own build step" -->
```sh
curl -fsSL https://raw.githubusercontent.com/jrcalgo/ciac/main/install.sh | sh
# or: cargo install --path crates/ciac   (needs a Rust toolchain; ~2 minutes)
```
<!-- ciac-verify:end -->

## 2. Create a project

<!-- ciac-verify:start id=new -->
```sh
ciac new my-app --template minimal
cd my-app
```
<!-- ciac-verify:end -->

Read what it printed. `ciac new` doesn't just scaffold — it tells you
the next command to run (`ciac check main.ciac`), because a scaffold
that leaves you guessing is a bug, not a feature.

## 3. The anatomy

Three files: `main.ciac` (the whole architecture — everything else is
generated *from* this), `README.md` (the next steps, specific to the
template you picked), and `AGENTS.md` (the same next steps, phrased
for an agent instead of a human — see
[docs/agents.md](../agents.md)). Nothing else exists yet; `ciac build`
is what creates a runnable project, and it never overwrites `main.ciac`
itself. The full manifest discipline — what's compiler-owned, what's
yours, how regeneration tells them apart — is
[docs/regeneration.md](../regeneration.md); three sentences are enough
to start: generated files carry a manifest hash; a file you've edited
is detected as edited; `ciac build`/`ciac diff` never silently clobber
your changes.

## 4. Make it yours

The scaffold is a minimal echo service — one record, one API, a
pipeline that returns what it's given. Add a field:

<!-- ciac-verify:file id=main-ciac-v1 path=main.ciac -->
```text
service Ping;

record Message {
    id: Uuid;
    text: String;
    sent_at: String;
}

api Echo: Message {
    method: POST;
    path: "/echo";
}

pipeline Echo: Return;
```
<!-- ciac-verify:end -->

<!-- ciac-verify:start id=check -->
```sh
ciac check main.ciac
```
<!-- ciac-verify:end -->

`ciac check` after every edit — that's the loop this whole guide
series teaches, not any one language feature: change the source,
check it, and only then build.

## 5. Build and read

<!-- ciac-verify:start id=build -->
```sh
ciac build main.ciac --target python --out ./build
```
<!-- ciac-verify:end -->

Any of the five targets works the same way — swap `python` for
`rust`/`typescript`/`go`/`java`. Now open two files and read them —
this is the point, not a detour:

`build/app/schemas.py` — your record, as a real Pydantic model:

```python
class Message(BaseModel):
    id: str
    text: str
    sent_at: str
```

`build/app/api/echo.py` — your API, as a real FastAPI route. No
framework magic hidden behind a decorator you can't see through:
the whole request/response shape is sitting right there, generated
once and yours to read (and, once you outgrow the generated stub,
yours to extend — guide 3 shows how).

## 6. Verify

<!-- ciac-verify:start id=verify -->
```sh
ciac verify main.ciac --target python --out ./build
```
<!-- ciac-verify:end -->

`ciac verify` regenerates from `main.ciac` and confirms the result
still matches (no drift) and the generated project's own lints and
tests pass. What it does *not* prove: that the service behaves
correctly against a real Postgres/NATS/Redis in production — that's
`ciac verify --system` (Docker required, [docs/deployment.md](../deployment.md))
or a real deploy. Stating the boundary here, not hiding it, is
deliberate — see [docs/backends.md](../backends.md) for the same
honesty applied across all five targets.

## Checkpoint

You should see:

```text
main.ciac: no errors
generated 16 files in ./build (python backend)
All checks passed!
```

A green `ciac verify` on a service with one record and one API. From
here: [guide 2](02-records-and-crud.md) adds real persistence, or —
if failure-injected simulation is what hooked you — jump straight to
[guide 5](05-simulation.md); this series is meant to be entered from
wherever your curiosity actually is.

## A short glossary

A few pairs of words that sound interchangeable across this
project's docs but aren't — recorded once here so the rest of the
series (and the reference docs) can use them without re-explaining:

- **target** vs **backend** — *target* is what you ask for
  (`--target python`); *backend* is the crate that implements
  code-generation for that target ([docs/backends.md](../backends.md)
  is about writing one). A reader picks a target; a contributor
  writes a backend.
- **capability** vs **component** — *capability* is the specific,
  named ontology this language models (`db`, `queue`, `auth`, `cache`,
  ...; see [docs/language.md](../language.md)). *Component* is the
  broader, architecture-graph sense used in a few error messages
  (e.g. CIAC0007, "unreachable component") — any node in the graph,
  not only a capability.
- **example** vs **program** — *example* names a specific checked-in
  `.ciac` file in this repository (`examples/quickstart.ciac`);
  *program* is the general term for whatever a `.ciac` file
  describes, whether or not it's checked in anywhere.
