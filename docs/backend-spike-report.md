# Third-backend spike report (v0.8 M6)

08UpdatePlan.md's secondary item: "Third backend spike (Go or
TypeScript): not to ship — to prove the `Backend` seam and the v0.7
emitter abstraction hold for a genuinely different host before the
interface calcifies. Timebox; outcome is a report + seam fixes,
shipping optional." This is that report. **No Go backend was added to
the workspace or registered with the CLI** — the spike code lived in a
throwaway crate outside the repo and no longer exists.

## What was built

A minimal `ciac_codegen::Backend` implementation for Go, targeting
`net/http` with zero framework (picked over TypeScript: Go's static
structs + JSON tags map onto the existing Pydantic/serde-shaped
`RecordCtx`/`FieldCtx` model far more directly than TypeScript's
structural typing would, so the spike stresses the *seam* rather than
introducing a second templating paradigm to reason about). Scope was
deliberately narrow — a single `api` + `record`, no capabilities at
all:

```ciac
service Ping;
record Message { id: Uuid; text: String; }
api Echo: Message { method: POST; path: "/echo"; }
pipeline Echo: Return;
```

The spike backend consumed `ciac_codegen::model::build_system`'s
`Ctx`/`RecordCtx`/`ApiCtx` — the same shared, language-neutral model
Python and Rust already build against — and hand-emitted `go.mod` +
`main.go` (route registration, a JSON-tagged struct per record, a
`/health` handler, a decode-and-echo handler per api).

**The generated output actually ran**: `go vet` and `go build` both
succeeded with no changes needed anywhere in `ciac-codegen`/`ciac-ir`,
and the resulting binary served real requests:

```
$ curl -s localhost:8000/health
{"status":"ok"}
$ curl -s -X POST localhost:8000/echo -d '{"id":"abc","text":"hello"}'
{"data":{"id":"abc","text":"hello"},"status":"accepted"}
```

## Findings

**The seam holds for this slice, with zero required changes.**
`Backend::{id, description, supports, generate}`, `GenOptions`,
`GeneratedProject`, and `ciac_codegen::check_support` all worked
exactly as designed — `check_support` correctly gated the whole IR
against the spike's intentionally narrow `supports()` (api + service
only), and nothing about the trait shape assumed Python or Rust
specifically.

**One real, if minor, smell**: `FieldCtx` (`ciac-codegen/src/model.rs`)
carries `py_type`, `rust_type`, `db_rust_type`, and `sql_type` as
named fields — baked-in knowledge of exactly two target languages
rather than an open-ended map. The spike backend had to fall back to
reading `rust_type` and hand-translating the handful of values it
needed (`"String"` → `go`'s `string`, with an unhandled fallback that
would silently emit a bogus Go type for anything else) since there's
no `go_type` field and no generic type-mapping hook. This is a real
gap a genuine third backend would hit immediately, but a **narrow,
mechanical one** — adding a target doesn't require re-deriving field
type mappings from a graph query, just adding one more named field (or
switching to a small enum-keyed map) alongside the existing two.

**Nothing else needed adjusting.** Route paths/methods, record field
lists, and project naming (`Ctx::package`) were all directly usable
as-is; `heck` (already a `ciac-codegen` dependency) handled the
Go-idiom requirement that struct fields be exported (capitalized) for
`encoding/json` to (de)serialize them at all, with no new dependency.

## What the spike did *not* test (disclosed, not a finding)

Scope was capabilities-free on purpose, to keep this genuinely
timeboxed. A fuller Go backend exercising `db`/`cache`/`queue` would
hit real, harder questions this spike has no evidence on either way:
Go's idiomatic error-return style versus Python/Rust's exception/
`Result`-based capability verbs the model's `Ctx` doesn't currently
parameterize over, and whether `InstanceCtx`/session-oriented fields
(shaped around SQLAlchemy/SQLx's connection-per-request pattern) carry
assumptions a `database/sql`-based Go client would need to route
around. Genuinely unknown until someone builds that far — flagged as
the natural next spike, not guessed at here.

## Recommendation

**Ship as-is; no seam fix needed for this milestone.** The one gap
found (no generic field-type-mapping hook) is small enough to fix
*when* a second real consumer needs it, not speculatively now for a
backend that doesn't exist — consistent with this codebase's own
"don't design for hypothetical future requirements" discipline. If a
real third backend is greenlit later, start there.
