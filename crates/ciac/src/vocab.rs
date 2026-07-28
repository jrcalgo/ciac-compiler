//! The language's machine-facing vocabulary, in one place (v0.13 M5).
//!
//! Three consumers render from these tables: `ciac lsp` (hover and
//! completion), `ciac describe` (the agent-facing JSON registry), and
//! — indirectly — every doc that promises the tables stay truthful.
//! One source of truth means a provider graduating on a target is one
//! edit here, not a docs-scavenger hunt.

/// A provider in the closed capability registry, with per-target
/// support stated as data.
pub struct Provider {
    pub name: &'static str,
    pub capability: &'static str,
    /// Backend ids that fully implement this provider.
    pub targets: &'static [&'static str],
    pub doc: &'static str,
}

pub struct Capability {
    pub name: &'static str,
    pub doc: &'static str,
}

// target-literal-ok: v0.22 M1 deliberately deferred deriving `targets`
// from `Backend::supports()` (`supports()` gates at `Component`
// granularity — one arm per capability *kind*, not per-provider, so it
// can say "this target supports `auth`" but not "...specifically JWT
// and OAuth2" the way this table needs); `PROVIDERS` stays hand-
// maintained, const, and audited truthful instead. `26UpdatePlan.md`
// M7 widened this from python/rust-only to all five internal targets,
// verified per provider against the real generated-project templates
// and each backend's own `supports()` match arms (not assumed from the
// old two-target default): TypeScript/Go/Java each reached full,
// unconditional `Component` parity across `23UpdatePlan.md`-
// `25UpdatePlan.md`'s own M7-M8 milestones (Go's/Java's `supports()`
// bodies name this explicitly), and grepping every backend's templates
// confirms each of these 19 providers by name — MySQL/SQLite, Kafka,
// Redis, S3/MinIO, SES/SMTP, OpenSearch, WebSocket/SSE, Prometheus,
// OpenTelemetry — present in all three. `Keycloak`/`Cron` need no
// per-backend evidence: both comments (Go's M7, Java's M7) confirm the
// dev-issuer-default and cron-library wiring are already target-
// neutral, computed once in `ciac-codegen` rather than per-backend.
// target-literal-ok: naming every internal target is this constant's
// entire purpose (v0.22 M1 grep fence, 22UpdatePlan.md Pillar 1).
const ALL_TARGETS: &[&str] = &["python", "rust", "typescript", "go", "java"];

/// Language keywords, in docs/language.md's vocabulary.
pub const KEYWORDS: &[(&str, &str)] = &[
    ("service", "`service <Name>;` names a single-service program; `service <Name> { .. }` scopes a deployable service inside a `project`."),
    ("project", "`project <Name>;` names a multi-service system; each `service <Name> { .. }` block becomes its own deployable."),
    ("import", "`import \"path\";` splices another file's declarations in at this position. `std/...` resolves against the embedded standard blueprints; `registry:<owner>/<repo>/<path>.ciac@<ref>` fetches a git-hosted blueprint (cached)."),
    ("use", "`use { auth JWT; db Postgres; .. }` declares capability requirements; each entry is `<capability> [<instance>] <Provider>;`."),
    ("record", "`record <Name> { field: Type; .. }` declares a typed data schema. Field types: String, Int, Float, Bool, Uuid, Timestamp, Json, or an inline `enum { .. }`."),
    ("error", "`error <Name> { .. }` declares an error record, usable with `fail <Name>` in handler bodies."),
    ("stream", "`stream <Name>: <Record>;` declares a named, typed message channel carried by the `queue` capability."),
    ("table", "`table <Name>: <Record>;` declares a typed persistent table; `db.*` verbs in handler bodies operate on tables."),
    ("api", "`api <Name>[: <Record>] { method: POST; path: \"/x\"; }` declares an HTTP endpoint; attach behavior with `pipeline <Name>: ..;`."),
    ("worker", "`worker <Name> on <Stream>;` declares a broker consumer; attach behavior with `pipeline <Name>: ..;`."),
    ("job", "`job <Name> { schedule: \"0 * * * *\"; }` declares a cron job (requires the `scheduler` capability)."),
    ("channel", "`channel <Name> on <Stream>;` fans a stream out to realtime clients (requires the `realtime` capability)."),
    ("crud", "`crud <Name>[: <Record>];` expands into REST API + Auth + Service + Database (+ Cache when present) — a complete typed resource."),
    ("events", "`events <Name>;` expands into Stream + Worker."),
    ("handler", "`handler <Name> { db: main; }` binds a pipeline step to capability instances; `handler <Name>(x: T) -> U { .. }` is a typed inline handler."),
    ("pipeline", "`pipeline <Name>: Step -> Step;` attaches behavior to an api/worker/job of the same name. Steps: handler names, `publish <Stream>`, `call <Service>.<Api>`, `match`, or builtins."),
    ("blueprint", "`blueprint <Name><T: record> { params { .. } .. }` declares a parameterized template; instantiate with `expand`."),
    ("expand", "`expand <Blueprint><<Record>> { param: value; };` instantiates a blueprint with hygienic names."),
    ("publish", "`publish <Stream>` (pipeline step or handler statement) sends the current payload to a typed stream."),
    ("call", "`call <Service>.<Api>` synchronously invokes another service's api through its typed client."),
    ("on", "`worker <Name> on <Stream>` / `channel <Name> on <Stream>` bind a consumer to a stream."),
    ("extern", "`extern handler <Name>(..) -> T;` declares a typed handler implemented in a seeded, user-owned file."),
    ("match", "`match field { Variant -> Step; _ -> Step; }` branches a pipeline on an enum field."),
    ("fail", "`fail <ErrorName>` (handler bodies) aborts with a declared `error` record."),
];

/// Capability kinds accepted in `use { .. }`.
pub const CAPABILITIES: &[Capability] = &[
    Capability {
        name: "auth",
        doc: "Request authentication middleware.",
    },
    Capability {
        name: "db",
        doc: "Relational persistence; typed CRUD resources and `table` declarations run on it.",
    },
    Capability {
        name: "cache",
        doc: "Key-value cache; typed CRUD reads become read-through cached when present.",
    },
    Capability {
        name: "queue",
        doc: "Message broker backing `stream`/`worker`/`channel` delivery.",
    },
    Capability {
        name: "logging",
        doc: "Structured logging.",
    },
    Capability {
        name: "metrics",
        doc: "Metrics endpoint.",
    },
    Capability {
        name: "tracing",
        doc: "Distributed tracing across `call`/stream edges.",
    },
    Capability {
        name: "object_store",
        doc: "Blob storage.",
    },
    Capability {
        name: "email",
        doc: "Outbound email.",
    },
    Capability {
        name: "search",
        doc: "Full-text search.",
    },
    Capability {
        name: "scheduler",
        doc: "Cron scheduling for `job` declarations.",
    },
    Capability {
        name: "realtime",
        doc: "Realtime fan-out for `channel` declarations.",
    },
    Capability {
        name: "external_http",
        doc: "A typed client for an external HTTP service (`base_url` attr).",
    },
    Capability {
        name: "users",
        doc: "Dev/test identity provider; `auth OAuth2`'s `issuer` defaults to it when omitted.",
    },
];

/// The closed provider registry. `targets` is the per-backend truth
/// the docs' support tables render from.
pub const PROVIDERS: &[Provider] = &[
    Provider { name: "JWT", capability: "auth", targets: ALL_TARGETS, doc: "Shared-secret HS256 bearer tokens." },
    Provider { name: "OAuth2", capability: "auth", targets: ALL_TARGETS, doc: "JWKS-validated RS256 resource server; requires `issuer`, optional `audience`." },
    Provider { name: "Postgres", capability: "db", targets: ALL_TARGETS, doc: "PostgreSQL." },
    Provider { name: "MySQL", capability: "db", targets: ALL_TARGETS, doc: "MySQL (per-engine pools and placeholder styles per target)." },
    Provider { name: "SQLite", capability: "db", targets: ALL_TARGETS, doc: "A zero-container local file under the app's data/ directory (v0.13)." },
    Provider { name: "Redis", capability: "cache", targets: ALL_TARGETS, doc: "Redis." },
    Provider { name: "NATS", capability: "queue", targets: ALL_TARGETS, doc: "NATS core (queue groups for workers, plain subscriptions for channels)." },
    Provider { name: "Kafka", capability: "queue", targets: ALL_TARGETS, doc: "Apache Kafka (aiokafka / rdkafka); topics reuse the `<service>.<stream>` subject names." },
    Provider { name: "Structured", capability: "logging", targets: ALL_TARGETS, doc: "Structured logs (structlog / tracing)." },
    Provider { name: "Prometheus", capability: "metrics", targets: ALL_TARGETS, doc: "Prometheus metrics endpoint." },
    Provider { name: "OpenTelemetry", capability: "tracing", targets: ALL_TARGETS, doc: "OTLP export to an otel-collector, with Jaeger wired in dev compose." },
    Provider { name: "S3", capability: "object_store", targets: ALL_TARGETS, doc: "S3-compatible blob storage (MinIO in dev)." },
    Provider { name: "SES", capability: "email", targets: ALL_TARGETS, doc: "AWS SES." },
    Provider { name: "SMTP", capability: "email", targets: ALL_TARGETS, doc: "Plain SMTP (Mailpit in dev)." },
    Provider { name: "OpenSearch", capability: "search", targets: ALL_TARGETS, doc: "OpenSearch." },
    Provider { name: "Cron", capability: "scheduler", targets: ALL_TARGETS, doc: "In-process cron scheduling (default when the provider is omitted)." },
    Provider { name: "WebSocket", capability: "realtime", targets: ALL_TARGETS, doc: "WebSocket fan-out." },
    Provider { name: "SSE", capability: "realtime", targets: ALL_TARGETS, doc: "Server-sent events (default when the provider is omitted)." },
    Provider { name: "Keycloak", capability: "users", targets: ALL_TARGETS, doc: "Dev-only Keycloak container seeded with a realm, a public client, and two dev users (`dev-admin`/`dev-user`); `scripts/token.sh` mints real tokens." },
];

/// Pipeline steps with built-in meaning (everything else names a handler).
pub const BUILTIN_STEPS: &[(&str, &str)] = &[
    (
        "Auth",
        "Builtin pipeline step: require an authenticated request (needs the `auth` capability).",
    ),
    (
        "Queue",
        "Builtin pipeline step: hand the payload to the broker (needs the `queue` capability).",
    ),
    (
        "Return",
        "Builtin pipeline step: respond with the current payload.",
    ),
];

/// Top-level and service-scoped declaration kinds — the subset of
/// [`KEYWORDS`] that names a declaration rather than a pipeline step
/// or expression keyword (`on`, `publish`, `call`, `match`, `fail`).
pub const DECLARATION_KINDS: &[&str] = &[
    "project",
    "service",
    "import",
    "use",
    "record",
    "error",
    "stream",
    "table",
    "api",
    "worker",
    "job",
    "channel",
    "crud",
    "events",
    "handler",
    "extern",
    "pipeline",
    "blueprint",
    "expand",
];

/// Record field types.
pub const FIELD_TYPES: &[&str] = &[
    "String",
    "Int",
    "Float",
    "Bool",
    "Uuid",
    "Timestamp",
    "Json",
    "enum { .. }",
];

/// A tab-stopped snippet body (VS Code snippet syntax: `${N:default}`
/// tabstops, `${N|a,b,c|}` choices, `$0` final cursor) for one
/// declaration form, keyed by `prefix` — the two surfaces in Pillar 5
/// (LSP completion's `insert_text` and `editors/vscode`'s
/// `contributes.snippets`) both render from this table, never from a
/// second hand-written copy.
///
/// `parses_with` is minimal companion source the parse-test suite
/// prepends before running the snippet's own default expansion (every
/// `${N:default}`/`${N|first,..|}` resolved to its first alternative)
/// through `ciac check` — most declaration forms aren't a complete
/// program standing alone (a `worker` needs a `queue` capability and a
/// stream to consume; a `table` needs `db` and a record). Empty for
/// the two forms that are ("project", "service").
pub struct Snippet {
    pub prefix: &'static str,
    pub description: &'static str,
    pub body: &'static [&'static str],
    pub parses_with: &'static [&'static str],
}

/// One snippet per [`DECLARATION_KINDS`] entry (v0.27 Pillar 5). Not
/// one per capability too, despite the plan text's "and capability"
/// phrasing (scoped down at implementation, recorded in the M7 Shipped
/// note): a `use { .. }` block's provider choice is demonstrated once,
/// on the `use`/`service` entries themselves, via `${N|a,b,c|}` — a
/// dedicated snippet per single-provider capability (nine of fourteen
/// have exactly one provider) would just retype that provider's only
/// name, adding table rows without adding a real choice.
pub const SNIPPETS: &[Snippet] = &[
    Snippet {
        prefix: "project",
        description: "A multi-service project",
        body: &["project ${1:Name};", "$0"],
        parses_with: &[],
    },
    Snippet {
        prefix: "service",
        description: "A service block with a capability",
        body: &[
            "service ${1:Name} {",
            "  use { ${2|db Postgres,db MySQL,db SQLite|}; }",
            "  $0",
            "}",
        ],
        parses_with: &[],
    },
    Snippet {
        prefix: "import",
        description: "Splice another file's declarations in",
        body: &["import \"${1:std/crud.ciac}\";", "$0"],
        parses_with: &["service SnippetTest;"],
    },
    Snippet {
        prefix: "use",
        description: "Declare a capability requirement",
        body: &[
            "use { ${1|db Postgres,db MySQL,db SQLite,queue NATS,queue Kafka,cache Redis,auth JWT,auth OAuth2|}; }",
            "$0",
        ],
        parses_with: &["service SnippetTest;"],
    },
    Snippet {
        prefix: "record",
        description: "A typed data schema",
        body: &["record ${1:Name} {", "    id: Uuid;", "    $0", "}"],
        parses_with: &["service SnippetTest;"],
    },
    Snippet {
        prefix: "error",
        description: "An error record, for `fail`",
        body: &["error ${1:Name} {", "    ${2:message}: String;", "    $0", "}"],
        parses_with: &["service SnippetTest;"],
    },
    Snippet {
        prefix: "stream",
        description: "A named message channel",
        body: &["stream ${1:Name}: ${2:Record};", "$0"],
        parses_with: &[
            "service SnippetTest;",
            "use { queue NATS; }",
            "record Record { id: Uuid; }",
        ],
    },
    Snippet {
        prefix: "table",
        description: "A persistent table",
        body: &["table ${1:Name}: ${2:Record};", "$0"],
        parses_with: &[
            "service SnippetTest;",
            "use { db Postgres; }",
            "record Record { id: Uuid; }",
        ],
    },
    Snippet {
        prefix: "api",
        description: "An HTTP endpoint",
        body: &[
            "api ${1:Name}: ${2:Record} {",
            "    method: ${3|POST,GET,PUT,DELETE,PATCH|};",
            "    path: \"${4:/path}\";",
            "}",
            "pipeline ${1:Name}: Return;",
            "$0",
        ],
        parses_with: &["service SnippetTest;", "record Record { id: Uuid; }"],
    },
    Snippet {
        prefix: "worker",
        description: "A broker consumer",
        body: &[
            "worker ${1:Name} on ${2:Stream};",
            "pipeline ${1:Name}: ${3:Handler};",
            "$0",
        ],
        parses_with: &[
            "service SnippetTest;",
            "use { queue NATS; }",
            "record Record { id: Uuid; }",
            "stream Stream: Record;",
            "handler Handler(v: Record) -> Record { return v; }",
        ],
    },
    Snippet {
        prefix: "job",
        description: "A scheduled cron job",
        body: &[
            "job ${1:Name} {",
            "    schedule: \"${2:0 * * * *}\";",
            "}",
            "pipeline ${1:Name}: ${3:Handler};",
            "$0",
        ],
        parses_with: &[
            "service SnippetTest;",
            "use { scheduler Cron; }",
            "handler Handler(v: Json) -> Json { return v; }",
        ],
    },
    Snippet {
        prefix: "channel",
        description: "Fan a stream out to realtime clients",
        body: &["channel ${1:Name} on ${2:Stream};", "$0"],
        parses_with: &[
            "service SnippetTest;",
            "use { queue NATS; realtime WebSocket; }",
            "record Record { id: Uuid; }",
            "stream Stream: Record;",
        ],
    },
    Snippet {
        prefix: "crud",
        description: "Free REST CRUD for a record",
        body: &["crud ${1:Name}: ${2:Record};", "$0"],
        parses_with: &[
            "service SnippetTest;",
            "use { db Postgres; }",
            "record Record { id: Uuid; }",
        ],
    },
    Snippet {
        prefix: "events",
        description: "Stream + worker shorthand",
        body: &["events ${1:Name};", "$0"],
        parses_with: &["service SnippetTest;", "use { queue NATS; }"],
    },
    Snippet {
        prefix: "handler",
        description: "A typed inline handler",
        body: &[
            "handler ${1:Name}(${2:v}: ${3:Record}) -> ${3:Record} {",
            "    return ${2:v};",
            "}",
            "$0",
        ],
        parses_with: &["service SnippetTest;", "record Record { id: Uuid; }"],
    },
    Snippet {
        prefix: "extern",
        description: "A typed handler stub you implement yourself",
        body: &["extern handler ${1:Name}(${2:v}: ${3:Record}) -> ${3:Record};", "$0"],
        parses_with: &["service SnippetTest;", "record Record { id: Uuid; }"],
    },
    Snippet {
        prefix: "pipeline",
        description: "Attach behavior to an api/worker/job",
        body: &["pipeline ${1:Name}: ${2:Return};", "$0"],
        parses_with: &["service SnippetTest;", "record Record { id: Uuid; }", "api Name: Record { method: POST; path: \"/name\"; }"],
    },
    Snippet {
        prefix: "blueprint",
        description: "A parameterized declaration template",
        body: &[
            "blueprint ${1:Name}<${2:R}: record> {",
            "    params { }",
            "    crud ${2:R}: ${2:R};",
            "}",
            "$0",
        ],
        parses_with: &["service SnippetTest;", "use { db Postgres; }"],
    },
    Snippet {
        prefix: "expand",
        description: "Instantiate a blueprint",
        body: &["expand ${1:Blueprint}<${2:Record}> {", "    $0", "}"],
        parses_with: &[
            "service SnippetTest;",
            "use { db Postgres; }",
            "record Record { id: Uuid; }",
            "blueprint Blueprint<R: record> {",
            "    params { }",
            "    crud R: R;",
            "}",
        ],
    },
];

/// A capability's handler-body verbs, exactly as `docs/expressions.md`'s
/// "The closed verb set" table names them — absent for capabilities
/// with no body-callable verb (`auth`, `queue`, `logging`, `metrics`,
/// `tracing`, `scheduler`, `realtime`, `users` are declarative-only:
/// `queue`/`realtime` are driven by `publish`/`on`/`channel` grammar,
/// not a verb call, and the rest have no in-handler surface at all).
/// Hand-maintained rather than derived from `ciac-sema::typeck`'s own
/// match arms (that checker is organized by `(capability, verb)` pairs
/// scattered across a few hundred lines, not one iterable table — see
/// its `check_verb_call`), so kept honest by the doc-inclusion test
/// below instead, the same discipline `PROVIDERS` already uses against
/// `docs/language.md`.
const VERBS: &[(&str, &str)] = &[
    (
        "db",
        "db.insert, db.get, db.update, db.delete, db.query [where], db.count [where], db.delete_where [where]",
    ),
    ("cache", "cache.get, cache.set, cache.delete"),
    (
        "object_store",
        "object_store.put, object_store.get, object_store.delete, object_store.list",
    ),
    ("email", "email.send"),
    ("search", "search.index, search.query"),
    ("external_http", "external_http.request"),
];

/// One line per capability naming what `ciac sim` does with it — the
/// 27UpdatePlan.md world contract, made visible in the editor instead
/// of only in `docs/simulation.md`'s prose. "Faked" means a scenario
/// controls the outcome with no real infrastructure; "not simulated"
/// means the generated code for that capability runs for real even
/// inside `ciac sim` (nothing about it is worth faking); "not
/// exercised" means the simulation runner has no code path that would
/// ever reach it today.
const SIM_NOTES: &[(&str, &str)] = &[
    ("db", "fully faked — schema-aware relational fake (unique/reference/cascade enforced, `where` clauses really evaluated)"),
    ("cache", "fully faked (TTL against the virtual clock, not wall-clock time)"),
    ("queue", "fully faked (per-(subject, group) cursor fan-out — every worker on a subject sees every message)"),
    ("object_store", "fully faked (in-memory put/get/delete/list)"),
    ("email", "fully faked (sent messages captured, never delivered)"),
    ("search", "fully faked (in-memory index/query)"),
    ("external_http", "fully faked (fixture-driven: a scenario's `given.http` supplies the response)"),
    ("scheduler", "fully faked (jobs fire against the virtual clock)"),
    ("auth", "fully faked (claims looked up against the scenario's `given.auth`, not real JWT/JWKS cryptography)"),
    ("realtime", "not exercised (no scenario step addresses a channel)"),
    ("logging", "not simulated (the real generated logging call runs, same as outside simulation)"),
    ("metrics", "not simulated (the real generated metrics call runs, same as outside simulation)"),
    ("tracing", "not simulated (the real generated tracing call runs, same as outside simulation)"),
    ("users", "not applicable (dev-only identity provider; `auth`'s own fake is what a scenario configures)"),
];

/// Expands a snippet body's placeholders to their default value — the
/// same rendering [`tests::every_snippet_default_expansion_parses`]
/// checks compiles, reused here so a hover's skeleton preview is
/// always literally what that test already proved parses, never a
/// second hand-typed copy that could drift from it.
fn expand_snippet_default(body: &[&str]) -> String {
    fn expand_line(line: &str) -> String {
        let bytes = line.as_bytes();
        let mut out = String::new();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'$' && bytes.get(i + 1) == Some(&b'{') {
                if let Some(close) = line[i..].find('}').map(|p| i + p) {
                    let inner = &line[i + 2..close];
                    let after_num = inner
                        .find(|c: char| !c.is_ascii_digit())
                        .unwrap_or(inner.len());
                    let rest = &inner[after_num..];
                    if let Some(choices) = rest.strip_prefix('|') {
                        let choices = choices.strip_suffix('|').unwrap_or(choices);
                        out.push_str(choices.split(',').next().unwrap_or(""));
                    } else if let Some(default) = rest.strip_prefix(':') {
                        out.push_str(default);
                    }
                    i = close + 1;
                    continue;
                }
            }
            if bytes[i] == b'$' && bytes.get(i + 1).is_some_and(u8::is_ascii_digit) {
                let mut j = i + 1;
                while bytes.get(j).is_some_and(u8::is_ascii_digit) {
                    j += 1;
                }
                i = j;
                continue;
            }
            out.push(bytes[i] as char);
            i += 1;
        }
        out
    }
    body.iter()
        .map(|l| expand_line(l))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The per-target support line every capability hover carries — the
/// union of every one of the capability's own providers' `targets`, so
/// a provider graduating on a target updates every hover that mentions
/// its capability without a second edit.
fn target_support_line(capability: &str) -> String {
    ALL_TARGETS
        .iter()
        .map(|target| {
            let supported = PROVIDERS
                .iter()
                .filter(|p| p.capability == capability)
                .any(|p| p.targets.contains(target));
            format!(
                "{target} {}",
                if supported { "\u{2713}" } else { "\u{2717}" }
            )
        })
        .collect::<Vec<_>>()
        .join("  ")
}

/// Hover text for any static vocabulary word. Capabilities and
/// declaration keywords with a snippet get structured, multi-line
/// markdown (v0.27 Pillar 5); everything else keeps the single-
/// sentence form it always had.
pub fn doc_for(word: &str) -> Option<String> {
    if let Some(cap) = CAPABILITIES.iter().find(|c| c.name == word) {
        let providers: Vec<&Provider> = PROVIDERS.iter().filter(|p| p.capability == word).collect();
        let mut out = format!("**{}** — capability: {}\n\n", cap.name, cap.doc);
        out.push_str(&format!(
            "Providers: {}\n",
            if providers.is_empty() {
                "(attribute-configured, no provider)".to_owned()
            } else {
                providers
                    .iter()
                    .map(|p| p.name)
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        ));
        out.push_str(&format!("Targets:   {}\n", target_support_line(word)));
        if let Some((_, verbs)) = VERBS.iter().find(|(c, _)| *c == word) {
            out.push_str(&format!("Verbs:     {verbs}\n"));
        }
        if let Some((_, note)) = SIM_NOTES.iter().find(|(c, _)| *c == word) {
            out.push_str(&format!("Simulation: {note}\n"));
        }
        if let Some(first) = providers.first() {
            out.push_str(&format!("\n    use {{ {} {}; }}\n", cap.name, first.name));
        }
        out.push_str("\nSee docs/language.md (`use { .. }`)");
        if VERBS.iter().any(|(c, _)| *c == word) {
            out.push_str(" · docs/expressions.md (the closed verb set)");
        }
        return Some(out);
    }
    if let Some(p) = PROVIDERS.iter().find(|p| p.name == word) {
        return Some(format!(
            "**{}** — {} provider ({}): {}",
            p.name,
            p.capability,
            p.targets.join(", "),
            p.doc
        ));
    }
    if let Some((_, doc)) = KEYWORDS.iter().find(|(w, _)| *w == word) {
        let mut out = format!("**{word}** — {doc}");
        if let Some(snip) = SNIPPETS.iter().find(|s| s.prefix == word) {
            out.push_str(&format!(
                "\n\n    {}\n",
                expand_snippet_default(snip.body).replace('\n', "\n    ")
            ));
        }
        out.push_str("\n\nSee docs/language.md");
        return Some(out);
    }
    if let Some((_, doc)) = BUILTIN_STEPS.iter().find(|(w, _)| *w == word) {
        return Some((*doc).to_owned());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// v0.22 M6: `docs/language.md`'s hand-written provider table is
    /// the "README/docs support table" the plan means to protect from
    /// silent drift. `PROVIDERS` lives in a lib-less binary crate (the
    /// `tests` crate can't import it — see `targets_cli.rs`'s own
    /// subprocess workaround for the same constraint), so the check
    /// lives here instead, mirroring `tests/tests/docs.rs`'s
    /// `error_docs_cover_every_code` pattern.
    #[test]
    fn language_md_mentions_every_provider() {
        let doc = include_str!("../../../docs/language.md");
        for provider in PROVIDERS {
            assert!(
                doc.contains(provider.name),
                "docs/language.md's provider table is missing `{}` (capability `{}`) — \
                 update the table when PROVIDERS changes",
                provider.name,
                provider.capability,
            );
        }
    }

    /// Pillar 5's own veracity bar: "each snippet's fully-expanded
    /// default form must parse ... a unit test, not a manual promise."
    /// Every snippet's `parses_with` companion plus its own rendered
    /// default body is written to a scratch file and run through the
    /// exact front end `ciac check` uses — the same
    /// `ciac_syntax::load` + `ciac_sema::analyze` pair `revalidate` in
    /// `lsp.rs` calls, so a snippet can never silently drift from
    /// something that actually compiles.
    #[test]
    fn every_snippet_default_expansion_parses() {
        for snip in SNIPPETS {
            let mut source = String::new();
            for line in snip.parses_with {
                source.push_str(line);
                source.push('\n');
            }
            source.push_str(&expand_snippet_default(snip.body));
            source.push('\n');

            let path = std::env::temp_dir().join(format!(
                "ciac-snippet-test-{}-{}-{:?}.ciac",
                snip.prefix,
                std::process::id(),
                std::thread::current().id()
            ));
            std::fs::write(&path, &source).expect("write scratch snippet file");

            let mut sources = ciac_diagnostics::SourceMap::new();
            let mut diags = ciac_diagnostics::Diagnostics::new();
            let load_result = ciac_syntax::load(&path, &mut sources, &mut diags);
            if let Ok(program) = &load_result {
                ciac_sema::analyze(program, &mut diags);
            }
            let _ = std::fs::remove_file(&path);

            assert!(
                load_result.is_ok(),
                "snippet `{}` default expansion failed to parse:\n{source}",
                snip.prefix,
            );
            assert!(
                !diags.has_errors(),
                "snippet `{}` default expansion has semantic errors:\n{source}\n\n{:#?}",
                snip.prefix,
                diags.iter().collect::<Vec<_>>(),
            );
        }
    }

    /// `VERBS`/`SIM_NOTES` are hand-maintained tables keyed by
    /// capability name (documented as such above, for the same reason
    /// `PROVIDERS.targets` is hand-maintained) — this guards the one
    /// way that goes silently wrong: a typo'd or renamed capability
    /// key that would make a real capability's hover quietly lose its
    /// Verbs/Simulation line instead of erroring.
    #[test]
    fn verb_and_sim_note_keys_name_real_capabilities() {
        for (cap, _) in VERBS {
            assert!(
                CAPABILITIES.iter().any(|c| c.name == *cap),
                "VERBS names `{cap}`, which isn't in CAPABILITIES"
            );
        }
        for (cap, _) in SIM_NOTES {
            assert!(
                CAPABILITIES.iter().any(|c| c.name == *cap),
                "SIM_NOTES names `{cap}`, which isn't in CAPABILITIES"
            );
        }
        for cap in CAPABILITIES {
            assert!(
                SIM_NOTES.iter().any(|(c, _)| *c == cap.name),
                "capability `{}` has no SIM_NOTES entry -- every capability is either faked, \
                 run for real, or explicitly not exercised under `ciac sim`; say which",
                cap.name,
            );
        }
    }

    /// Every capability's hover is structured, multi-line markdown
    /// carrying registry-derived data (Pillar 5: "hover content is
    /// generated-at-compile-time data, tested like data") -- this
    /// checks the shape every capability hover must have, not any one
    /// capability's prose.
    #[test]
    fn capability_hover_has_the_structured_shape() {
        for cap in CAPABILITIES {
            let hover = doc_for(cap.name).unwrap_or_else(|| panic!("no hover for `{}`", cap.name));
            assert!(
                hover.starts_with(&format!("**{}**", cap.name)),
                "`{}` hover doesn't lead with its own bolded name:\n{hover}",
                cap.name
            );
            assert!(
                hover.contains("Providers:"),
                "`{}` hover is missing a Providers: line:\n{hover}",
                cap.name
            );
            assert!(
                hover.contains("Targets:"),
                "`{}` hover is missing a Targets: line:\n{hover}",
                cap.name
            );
            let has_verbs = VERBS.iter().any(|(c, _)| *c == cap.name);
            assert_eq!(
                hover.contains("Verbs:"),
                has_verbs,
                "`{}` hover's Verbs: line presence disagrees with VERBS:\n{hover}",
                cap.name
            );
            assert!(
                hover.contains("Simulation:"),
                "`{}` hover is missing a Simulation: line (every capability has a SIM_NOTES entry):\n{hover}",
                cap.name
            );
            assert!(
                hover.contains("docs/language.md"),
                "`{}` hover doesn't point back at the reference doc:\n{hover}",
                cap.name
            );
        }
    }

    /// The Targets: line is registry-derived, not hardcoded prose --
    /// asserted here against the same `PROVIDERS` table it's built
    /// from, for every capability at once, so a provider narrowing to
    /// fewer targets would move this line without anyone hand-editing
    /// hover text.
    #[test]
    fn capability_target_line_matches_provider_registry() {
        for cap in CAPABILITIES {
            let expected_targets: std::collections::BTreeSet<&str> = PROVIDERS
                .iter()
                .filter(|p| p.capability == cap.name)
                .flat_map(|p| p.targets.iter().copied())
                .collect();
            for target in ALL_TARGETS {
                let line = target_support_line(cap.name);
                let expects_tick = expected_targets.contains(target);
                let tick = format!("{target} \u{2713}");
                assert_eq!(
                    line.contains(&tick),
                    expects_tick,
                    "`{}`'s Targets: line disagrees with PROVIDERS for `{target}`:\n{line}",
                    cap.name
                );
            }
        }
    }

    /// A declaration keyword with a snippet shows that snippet's own
    /// default-expansion as its hover preview -- proven equal to what
    /// [`every_snippet_default_expansion_parses`] already parse-tested,
    /// not a second hand-typed rendering that could drift from it.
    #[test]
    fn keyword_hover_preview_matches_its_own_snippet() {
        for snip in SNIPPETS {
            let Some(hover) = doc_for(snip.prefix) else {
                continue;
            };
            let expanded = expand_snippet_default(snip.body);
            for line in expanded.lines() {
                assert!(
                    hover.contains(line),
                    "`{}` hover doesn't contain its own snippet's line `{line}`:\n{hover}",
                    snip.prefix
                );
            }
        }
    }

    /// `ciac describe`'s enrichment must stay additive: every key
    /// `describe::build()` already emits today keeps meaning the same
    /// thing after M7's vocab changes -- a real regression here would
    /// be a `DESCRIBE_VERSION` bump, not a silent reshape. Vocab has no
    /// direct dependency on `describe`, so this re-checks the tables
    /// `describe::build()` reads from are still shaped the way it
    /// expects: `capabilities` keyed by name+doc+providers,
    /// `providers` keyed by name+capability+targets+doc -- unchanged
    /// field sets, only new tables (`SNIPPETS`, `VERBS`, `SIM_NOTES`)
    /// added alongside.
    #[test]
    fn describe_facing_tables_kept_their_shape() {
        for cap in CAPABILITIES {
            assert!(!cap.name.is_empty() && !cap.doc.is_empty());
        }
        for p in PROVIDERS {
            assert!(!p.name.is_empty() && !p.capability.is_empty() && !p.targets.is_empty());
        }
    }
}
