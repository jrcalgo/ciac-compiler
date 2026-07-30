# Security policy

## Reporting a vulnerability

Use GitHub's [private vulnerability reporting](https://github.com/jrcalgo/ciac-compiler/security/advisories/new)
for this repository rather than a public issue. That opens a private
advisory thread with the maintainer directly.

Include, where relevant:

- The `.ciac` source (or a minimized version) that reproduces the issue.
- The target(s) affected (`python`/`rust`/`typescript`/`go`/`java`) if
  the problem is in generated code rather than the compiler itself.
- Whether the issue requires an attacker-controlled `.ciac` program, or
  reproduces from ordinary/trusted input.

There is no fixed SLA — this is a single-maintainer project — but
security reports are triaged ahead of everything else in the backlog.

## Scope

**In scope:**

- The compiler itself (`crates/`): the parser, semantic analysis,
  intermediate representation, and code-generation backends.
- Code the compiler *generates* — e.g. a generated auth check that
  fails open, a generated SQL query that isn't parameterized, or a
  generated migration that corrupts data.
- The CLI (`ciac`), its MCP/LSP surfaces, and the release/CI pipeline
  itself (a compromised build artifact, a workflow permission that's
  broader than it needs to be, etc.).

**Out of scope:**

- Vulnerabilities in a *specific project* someone generated and then
  modified by hand — once generated code leaves the compiler's control,
  its security is the operator's responsibility, the same as any other
  scaffolded codebase.
- Vulnerabilities in third-party dependencies pulled into generated
  projects (Spring, FastAPI, Axum, Fastify, the Go standard library,
  etc.) — report those upstream. `deny.toml` and CI's
  `generated-audit` job track known advisories across all five
  generated ecosystems as a floor, not a guarantee.
- Missing security *features* that are legitimately unimplemented
  (tracked in `docs/backends.md`'s divergence ledger) rather than
  broken.

## What's already automated

Every push and a weekly schedule run `cargo deny check` against the
compiler's own dependency tree (`workspace-audit`) and `pip-audit` /
`cargo audit` / `npm audit` / `govulncheck` / `grype` against the
generated Python/Rust/TypeScript/Go/Java projects for a representative
example (`generated-audit`). Both are automated scanning, not an
external human audit — there has not yet been one, and that gap is
disclosed rather than implied closed.
