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
// from `Backend::supports()` to M4 (`22UpdatePlan.md` Pillar 1/4) —
// `supports()` is unconditionally `true` on both bundled backends
// today (no per-component discrimination to derive from yet), so a
// derivation now would carry zero information over this literal and
// would force `PROVIDERS` off `const` (every consumer touches it as
// compile-time data). M4's `ciac targets --json` + docs-drift-test
// milestone is where this becomes real, registry-derived data.
const BOTH: &[&str] = &["python", "rust"];

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
    Provider { name: "JWT", capability: "auth", targets: BOTH, doc: "Shared-secret HS256 bearer tokens." },
    Provider { name: "OAuth2", capability: "auth", targets: BOTH, doc: "JWKS-validated RS256 resource server; requires `issuer`, optional `audience`." },
    Provider { name: "Postgres", capability: "db", targets: BOTH, doc: "PostgreSQL." },
    Provider { name: "MySQL", capability: "db", targets: BOTH, doc: "MySQL (v0.13: per-engine pools and placeholder styles on both targets)." },
    Provider { name: "SQLite", capability: "db", targets: BOTH, doc: "A zero-container local file under the app's data/ directory (v0.13)." },
    Provider { name: "Redis", capability: "cache", targets: BOTH, doc: "Redis." },
    Provider { name: "NATS", capability: "queue", targets: BOTH, doc: "NATS core (queue groups for workers, plain subscriptions for channels)." },
    Provider { name: "Kafka", capability: "queue", targets: BOTH, doc: "Apache Kafka (aiokafka / rdkafka); topics reuse the `<service>.<stream>` subject names." },
    Provider { name: "Structured", capability: "logging", targets: BOTH, doc: "Structured logs (structlog / tracing)." },
    Provider { name: "Prometheus", capability: "metrics", targets: BOTH, doc: "Prometheus metrics endpoint." },
    Provider { name: "OpenTelemetry", capability: "tracing", targets: BOTH, doc: "OTLP export to an otel-collector, with Jaeger wired in dev compose." },
    Provider { name: "S3", capability: "object_store", targets: BOTH, doc: "S3-compatible blob storage (MinIO in dev)." },
    Provider { name: "SES", capability: "email", targets: BOTH, doc: "AWS SES." },
    Provider { name: "SMTP", capability: "email", targets: BOTH, doc: "Plain SMTP (Mailpit in dev)." },
    Provider { name: "OpenSearch", capability: "search", targets: BOTH, doc: "OpenSearch." },
    Provider { name: "Cron", capability: "scheduler", targets: BOTH, doc: "In-process cron scheduling (default when the provider is omitted)." },
    Provider { name: "WebSocket", capability: "realtime", targets: BOTH, doc: "WebSocket fan-out." },
    Provider { name: "SSE", capability: "realtime", targets: BOTH, doc: "Server-sent events (default when the provider is omitted)." },
    Provider { name: "Keycloak", capability: "users", targets: BOTH, doc: "Dev-only Keycloak container seeded with a realm, a public client, and two dev users (`dev-admin`/`dev-user`); `scripts/token.sh` mints real tokens." },
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

/// Hover text for any static vocabulary word.
pub fn doc_for(word: &str) -> Option<String> {
    if let Some((_, doc)) = KEYWORDS.iter().find(|(w, _)| *w == word) {
        return Some((*doc).to_owned());
    }
    if let Some(cap) = CAPABILITIES.iter().find(|c| c.name == word) {
        let providers: Vec<&str> = PROVIDERS
            .iter()
            .filter(|p| p.capability == word)
            .map(|p| p.name)
            .collect();
        return Some(format!(
            "Capability: {} Providers: {}.",
            cap.doc,
            if providers.is_empty() {
                "(attribute-configured)".to_owned()
            } else {
                providers.join(", ")
            }
        ));
    }
    if let Some(p) = PROVIDERS.iter().find(|p| p.name == word) {
        return Some(format!(
            "{} provider ({}): {}",
            p.capability,
            p.targets.join(", "),
            p.doc
        ));
    }
    if let Some((_, doc)) = BUILTIN_STEPS.iter().find(|(w, _)| *w == word) {
        return Some((*doc).to_owned());
    }
    None
}
