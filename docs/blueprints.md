# Blueprints (v0.8, extended v0.14 M5)

Modules (`import "path";`, v0.8 M1) let a system be split across
files; blueprints are the other half of "compose programs from
reusable, parameterized parts" — a way to write a pattern (an audited
CRUD resource, a webhook receiver shape) once and instantiate it
against different record types, instead of hand-copying a few
declarations per service.

```ciac
blueprint AuditedCrud<R: record> {
    params { prefix: String; }
    use { db main Postgres; }
    crud Resource: R;
    stream Audited: AuditEvent;
    handler AfterWrite(r: R) -> R {
        return r;
    }
}

service Catalog  { expand AuditedCrud<Video> { prefix: "/v1"; } }
service Accounts { expand AuditedCrud<User>  { prefix: "/v1"; } }
```

Each `expand` produces its own `Resource`/`Audited`/`AfterWrite`,
suffixed with the concrete record's name (`ResourceVideo`/`ResourceUser`,
...) so the two expansions never collide even though they're
textually identical apart from the type argument — see Hygiene below.

## Grammar

```ebnf
blueprint-decl = "blueprint" IDENT "<" IDENT ":" "record" ">"
                 "{" "params" "{" { field } "}"
                     { blueprint-item } "}" ;
blueprint-item = use-block | crud-decl | stream-decl | handler-decl
                | record-decl | error-decl | table-decl
                | api-decl | worker-decl | pipeline-decl ;
expand-stmt    = "expand" IDENT "<" IDENT ">" decl-tail ;
```

`expand` is both a top-level `Item` (single-service programs) and a
`service`-block item (`ServiceItem::Expand`) — the same statement,
just at whichever scope it's written in.

- **One generic type parameter, constrained to `record`.** `<R: record>`
  is checked against the record names declared anywhere in the
  resolved program; `expand Blueprint<NotARecord> { .. };` is
  `CIAC0050` (blueprint constraint violation).
- **A closed body**: `use` / `crud` / `stream` / `handler` / `record` /
  `error` / `table` / `api` / `worker` / `pipeline` — still not the
  full `Item`/`ServiceItem` grammar (no nested `blueprint`/`expand`/
  `import`/`job`/`channel`/`events`). v0.8 M2 shipped just
  `use`/`crud`/`stream`/`handler`; v0.14 M5 grew this to the rest so a
  body can declare a *complete, self-contained* api-plus-pipeline
  shape — see "The `std/` blueprint library" below for two real ones.
  A `pipeline` inside a body may only attach to an `api`/`worker`
  declared in the *same* body (both get renamed under the same
  hygiene substitution, so the pair stays attached post-expansion);
  nothing mechanically stops a step from naming something in the
  enclosing scope, but the expanded program still has to pass the
  same graph-build/typeck resolution any hand-written program does,
  so a dangling or accidental cross-scope reference surfaces as the
  ordinary "unknown X" diagnostic it would outside a blueprint.
- **Scalar `params { name: Type; }`**: `String`/`Int` only, matching
  `attr-value`'s existing closed set. Every param is required — no
  optional/defaultable params yet (see Non-goals in
  `08UpdatePlan.md`'s v0.8 M3 plan). An `expand` site's args must name
  every declared param with a value of the declared type, or it's
  `CIAC0049` (blueprint arity mismatch) — missing, unknown, or
  wrong-typed. A `params` value reaches three places in the body:
  attribute values (`path: path;`), a declared name it exactly matches
  (the param-driven hygiene exception below), and — since v0.14 M5 —
  bare identifiers inside a handler-body expression (`if accepted >=
  limit { .. }`), substituted in as the literal `Str`/`Int`/`Bool`
  value.
- **Unknown blueprint name**: `expand` naming a blueprint nothing
  declares (or that isn't `import`ed) is `CIAC0048`.

## Expansion happens before semantic analysis

`ciac_sema::blueprints::expand()` runs as the *first* step of
`ciac_sema::analyze()`, before graph building — the same "resolve at
the AST level before sema sees it" trick modules use for `import`. By
the time duplicate-name checks, graph building, and every validation
pass run, an expanded program is indistinguishable from one where the
same declarations were hand-written per service. Nothing downstream
needs to know blueprints exist.

## Hygiene

Every name a blueprint body declares gets rewritten at each `expand`
site, so two expansions of the same blueprint never collide:

- **Default**: suffixed with the concrete type argument's name —
  `AfterWrite` expanded with `<Video>` becomes `AfterWriteVideo`.
- **Param-driven exception** (v0.8 M3): when a declared name's text
  exactly matches a declared `params` entry whose value is a `String`,
  the literal param value is substituted instead of the suffix. This
  is what lets a blueprint faithfully reproduce a caller-chosen exact
  name — see `std/crud.ciac` below, where `crud name: R;` alongside
  `params { name: String; }` means `expand Crud<Video> { name:
  "Videos"; };` produces a resource literally named `Videos`, not
  `VideosVideo`.
- Expanding the same blueprint with the same type argument (or the
  same explicit param name) twice still collides — it falls through to
  the ordinary `CIAC0003` (duplicate declaration) check, the same as
  two hand-written declarations sharing a name.

## The `std/` blueprint library

`import "std/<name>.ciac";` resolves against a small library embedded
in the compiler at build time (`crates/ciac-syntax/std/`), not the
filesystem — a reserved namespace that works regardless of the
importing file's location or the user's own directory layout.

`std/crud.ciac` (v0.8 M3):

```ciac
blueprint Crud<R: record> {
    params { name: String; }
    crud name: R;
}
```

`crud` stays a compiler **primitive** — it also populates
`graph.resources`, which drives dedicated typed-CRUD-REST codegen
(create/read/update/delete, pagination, cache-aside) that nothing
expressible in a blueprint body can construct from smaller pieces yet.
`std.Crud` *wraps* the primitive faithfully rather than replacing it:
`import "std/crud.ciac"; expand Crud<Video> { name: "Videos"; };`
generates **byte-identical** output to hand-written `crud Videos:
Video;`, proven by `tests/tests/blueprints.rs`'s
`std_crud_blueprint_is_byte_identical_to_hand_written_crud`. `crud X;`
does not become sugar for `expand std.Crud<X>;` — that would require
verb vocabulary (`query`, pagination, cache-aside as expressible
handler-body operations) the language doesn't have yet, a
substantially larger feature than a std library on its own.

`std/webhook.ciac` (v0.14 M5) — a webhook receiver: a typed POST
endpoint that republishes its validated payload onto a stream for a
worker elsewhere to react to:

```ciac
blueprint Webhook<Payload: record> {
    params { path: String; }
    stream Received: Payload;
    handler Receive(payload: Payload) -> Payload {
        publish Received(payload);
        return payload;
    }
    api Ingest: Payload { method: POST; path: path; }
    pipeline Ingest: Receive -> Return;
}
```

`std/rate-limited-api.ciac` (v0.14 M5) — a request-capped typed API.
The original forecast for this blueprint (see `14UpdatePlan.md`)
sketched a cache-backed counter inspected with `if cache.get k as V
is v { .. }`-style optional unwrapping; that construct doesn't exist
yet (v0.14 M1 deliberately deferred `is`-unwrap over verb-returned
optionals — see `docs/expressions.md`), so this blueprint reaches the
same request-capping effect with what's actually expressible today: a
`Ticket` row per accepted request and `db.count` against the running
total — a cumulative quota (accept up to `limit` requests, ever)
rather than a decaying time window. `limit` is supplied per `expand`
site and reaches the handler body directly, proving `params`
substitution now covers expression code, not just attributes and
declaration-name overrides:

```ciac
expand RateLimitedApi<Comment> { path: "/comments"; limit: 100; }
```

Both are proven by `tests/tests/blueprints.rs`'s
`std_webhook_blueprint_generates_on_both_backends` and
`std_rate_limited_api_blueprint_generates_on_both_backends` — the
behavioral half of the "byte-equivalence-or-behavioral" discipline
`std/crud.ciac` established, since neither has a hand-written
primitive to diff against. `std/outbox.ciac` (also sketched in the
v0.14 forecast) stays out of the library: a transactional-outbox
worker that *drains* a table — process every pending row, publish
each — needs a loop, and the language has none (a handler body is a
straight-line effect script by design, see the v0.14 design stance in
`14UpdatePlan.md`); `db.query`'s result is an opaque list a body can
only pass through or store whole, never iterate, so "publish one
event per pending row" isn't expressible as a single handler
invocation. A future version with a bounded iteration construct could
revisit this.

## `registry:` imports (v0.12 M3)

`import "registry:<owner>/<repo>/<path>.ciac@<ref>";` fetches a
blueprint from a plain git-hosted directory (default base:
`raw.githubusercontent.com`; override with `$CIAC_REGISTRY`), caches
it under `$XDG_CACHE_HOME/ciac/registry/`, and splices it in through
the identical parse → expansion → validation path as local and `std/`
imports. Pin an immutable ref and resolution is reproducible and
offline after the first fetch. Details, editor tooling, and the trust
boundary live in [authoring.md](authoring.md).

## Worked example

`examples/audited-crud.ciac` is the M2 flagship: one blueprint,
expanded for two different records in two different services, each
expansion's names hygienically suffixed so they never collide.
`examples/multi-service-media.ciac` alongside `import
"std/crud.ciac";` (see `tests/tests/blueprints.rs`) is the M3 proof
that a blueprint can faithfully stand in for hand-written `crud`.
