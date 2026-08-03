//! v0.17 M11: a Rust-native simulation world. Unlike Python -- which
//! must restate `ciac-sim`'s primitives narrowly, since it cannot call
//! Rust code directly (`sim/pyrunner/world.py`'s own `FailureEngine`
//! class says so in its docstring) -- generated Rust code depends on
//! `ciac-sim` itself (vendored source, `ciac sim`'s own job to write
//! out; see `crates/ciac/src/commands.rs`) and drives the real
//! [`crate::failure::FailureEngine`] this crate already owns, not a
//! second copy of it.
//!
//! Schema-agnostic like Python's `FakeDatabase`: rows are
//! [`serde_json::Value`], not compile-time-typed structs, because this
//! crate has no knowledge of any particular `.ciac` program's schema.
//! The bridge from a generated project's own typed records to these
//! JSON rows is one `serde_json::to_value`/`from_value` round-trip at
//! each world-guarded call site in the generated code itself --
//! mirroring exactly how Python's fakes bridge typed SQLAlchemy models
//! to plain dicts.
//!
//! Originally narrow (17UpdatePlan.md's M11 entry): only `db.insert`
//! and broker `publish`, to drive the checkpoint's own vertical slice.
//! 27UpdatePlan.md M2 deepens the stateful core toward Python's own
//! coverage: `RelationalSchema`-aware insert/update/delete (reference
//! existence, `unique`, cascade/restrict-on-delete) via
//! `SimWorld::db_insert_checked`/`db_update_checked`/
//! `db_delete_checked`/`commit_batch_checked`, and [`BrokerLog`]'s
//! per-`(subject, group)` cursor log for independent-queue-group
//! fan-out, alongside the still-unchanged `FakeQueue`. Peripheral
//! fakes (cache/store/email/search/http/auth) remain M3's own
//! disclosed future work, not silently claimed here.

use crate::clock::{Entropy, VirtualClock};
use crate::failure::{FailureAction, FailureEngine, FailurePhase, FailureRule};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Mutex;

/// 27UpdatePlan.md M2: `on_delete`'s two shapes -- deliberately a local
/// copy of `crate::plan::SimRefAction` rather than that type itself.
/// This whole file is vendored byte-for-byte into every generated Rust
/// project via `include_str!` (`ciac-backend-rust/src/lib.rs`), which
/// has no `ciac-ir` dependency and therefore cannot resolve
/// `crate::plan` (that module's own `use ciac_ir::...` is exactly why
/// `plan.rs` is deliberately *not* vendored, per that file's own doc
/// comment) -- so `world.rs` must stay self-contained for every type it
/// names in its own public surface, not just its own definitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorldRefAction {
    Restrict,
    Cascade,
}

/// One outgoing reference field, as a schema-aware store needs it --
/// the vendorable counterpart to `plan::SimFieldType::Reference`'s
/// payload. A generated Rust project's own `sim_runner.rs` (M4) builds
/// these as literal struct-literal Rust source at codegen time, since
/// it has no `SimPlan` JSON to load at runtime either (see the
/// `VENDORED_SIM_*` doc comment); `ciac-sim`'s own callers with a real
/// `SimPlan` in hand (this crate's tests today; a future non-generated
/// driver) build them the same way, by hand, for the same reason
/// there is no runtime `plan::SimTable -> WorldTable` bridge here.
#[derive(Debug, Clone)]
pub struct WorldReference {
    pub field_name: String,
    pub target_table: Option<String>,
    pub on_delete: WorldRefAction,
    pub unique: bool,
}

/// One table's worth of schema facts a schema-aware store needs --
/// just its own outgoing references, since that is all
/// `RelationalSchema` ever asks a table for.
#[derive(Debug, Clone)]
pub struct WorldTable {
    pub name: String,
    pub references: Vec<WorldReference>,
}

#[derive(Debug, Clone, Default)]
struct RelationalSchema {
    /// table name -> its own outgoing reference fields.
    outgoing: BTreeMap<String, Vec<WorldReference>>,
}

impl RelationalSchema {
    fn from_tables(tables: &[WorldTable]) -> Self {
        let outgoing = tables
            .iter()
            .map(|table| (table.name.clone(), table.references.clone()))
            .collect();
        Self { outgoing }
    }

    fn outgoing(&self, table: &str) -> &[WorldReference] {
        self.outgoing.get(table).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Every `(referencing_table, reference_field)` whose reference
    /// targets `target_table` — the reverse index `db_delete_checked`'s
    /// plan-then-apply needs to find restrict/cascade candidates.
    fn incoming(&self, target_table: &str) -> Vec<(&str, &WorldReference)> {
        self.outgoing
            .iter()
            .flat_map(|(t, refs)| refs.iter().map(move |r| (t.as_str(), r)))
            .filter(|(_, r)| r.target_table.as_deref() == Some(target_table))
            .collect()
    }
}

/// A schema-aware store's checked operations can fail this way, distinct
/// from `FailureEngine`-injected failures — a real constraint the
/// scenario's own data violated, not a simulated infrastructure fault.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelationalError {
    MissingReference {
        table: String,
        field: String,
        target_table: String,
        target_pk: String,
    },
    UniqueViolation {
        table: String,
        field: String,
        value: String,
    },
    RestrictedDelete {
        table: String,
        pk: String,
        referencing_table: String,
        referencing_field: String,
    },
}

impl std::fmt::Display for RelationalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RelationalError::MissingReference {
                table,
                field,
                target_table,
                target_pk,
            } => write!(
                f,
                "{table}.{field} references {target_table}/{target_pk}, which does not exist"
            ),
            RelationalError::UniqueViolation {
                table,
                field,
                value,
            } => write!(f, "{table}.{field} = {value:?} is not unique"),
            RelationalError::RestrictedDelete {
                table,
                pk,
                referencing_table,
                referencing_field,
            } => write!(
                f,
                "cannot delete {table}/{pk}: restricted by {referencing_table}.{referencing_field}"
            ),
        }
    }
}

impl std::error::Error for RelationalError {}

/// One operation in an atomic `commit_batch_checked` call — the
/// transaction leaf's own accumulation shape (26UpdatePlan.md M1–M2:
/// inner db verbs inside a `transaction {}` block accumulate into a
/// batch rather than hitting their own world calls individually).
/// Deliberately does not auto-expand cascades on `Delete` — that is
/// `db_delete_checked`'s own job as a single, immediate, plan-then-
/// apply call; a caller building a batch that must satisfy a cascade
/// includes the cascaded deletes explicitly.
#[derive(Debug, Clone)]
pub enum BatchOp {
    Insert {
        table: String,
        row: Value,
    },
    Delete {
        table: String,
        pk: String,
    },
    /// 27UpdatePlan.md M4: full-record replace by `id`, the batch
    /// counterpart to `db_update_checked` -- added when Rust's
    /// transaction leaf needed to batch a `db.update` alongside
    /// `db.insert`/`db.delete` inside a `transaction {}` block, even
    /// though no checked-in example exercises this combination yet.
    /// A no-op (not an error) if `pk` names no existing row in the
    /// overlay at apply time, matching `db_update_checked`'s own
    /// "absent vs. present" contract.
    Update {
        table: String,
        pk: String,
        row: Value,
    },
}

/// An in-memory table store keyed by table name, rows as plain JSON
/// values. No constraints, no cascades, no indexes -- see the module
/// docs for what Python's fuller fake already covers that this one
/// does not yet.
#[derive(Debug, Default)]
pub struct FakeDatabase {
    tables: Mutex<BTreeMap<String, Vec<Value>>>,
}

impl FakeDatabase {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&self, table: &str, row: Value) {
        self.tables
            .lock()
            .expect("FakeDatabase mutex poisoned")
            .entry(table.to_owned())
            .or_default()
            .push(row);
    }

    /// Rows in `table` matching every key/value pair in `filter` (an
    /// empty filter matches every row in the table) -- the scenario
    /// runner's own `expect.row`/`expect.quiescence`-adjacent check,
    /// not something generated code itself ever calls.
    pub fn find_where(&self, table: &str, filter: &BTreeMap<String, Value>) -> Vec<Value> {
        self.tables
            .lock()
            .expect("FakeDatabase mutex poisoned")
            .get(table)
            .into_iter()
            .flatten()
            .filter(|row| filter.iter().all(|(k, v)| row.get(k) == Some(v)))
            .cloned()
            .collect()
    }

    pub fn count(&self, table: &str) -> usize {
        self.tables
            .lock()
            .expect("FakeDatabase mutex poisoned")
            .get(table)
            .map_or(0, Vec::len)
    }

    /// 27UpdatePlan.md M2: by-`id` lookup — `None` (absent vs. present)
    /// if no row has that key, matching production `db.get`'s own
    /// shape.
    pub fn get(&self, table: &str, pk: &str) -> Option<Value> {
        self.tables
            .lock()
            .expect("FakeDatabase mutex poisoned")
            .get(table)?
            .iter()
            .find(|row| row.get("id").and_then(Value::as_str) == Some(pk))
            .cloned()
    }

    /// 27UpdatePlan.md M2: unconditional removal by `id` — `true` if a
    /// row was actually removed. No reference/cascade checking here;
    /// that is `SimWorld::db_delete_checked`'s job, one layer up, since
    /// it needs the schema this plain store doesn't carry.
    pub fn delete(&self, table: &str, pk: &str) -> bool {
        let mut tables = self.tables.lock().expect("FakeDatabase mutex poisoned");
        match tables.get_mut(table) {
            Some(rows) => {
                let before = rows.len();
                rows.retain(|row| row.get("id").and_then(Value::as_str) != Some(pk));
                rows.len() != before
            }
            None => false,
        }
    }

    /// 27UpdatePlan.md M2: full-record replace by `id` — matching
    /// production `db.update`'s real shape (confirmed at M1: `UPDATE
    /// <table> SET <every column> WHERE id = <pk>`, not an attribute
    /// patch). `true` if a row with that `id` existed to replace.
    pub fn replace(&self, table: &str, pk: &str, record: Value) -> bool {
        let mut tables = self.tables.lock().expect("FakeDatabase mutex poisoned");
        match tables.get_mut(table) {
            Some(rows) => match rows
                .iter_mut()
                .find(|row| row.get("id").and_then(Value::as_str) == Some(pk))
            {
                Some(row) => {
                    *row = record;
                    true
                }
                None => false,
            },
            None => false,
        }
    }

    /// 27UpdatePlan.md M2: a full clone of every table -- the scratch
    /// overlay `SimWorld`'s schema-aware checked operations validate a
    /// prospective write against before touching the real store,
    /// mirroring Python's own `commit_batch`'s overlay-of-tables
    /// approach (`sim/pyrunner/world.py`).
    fn snapshot(&self) -> BTreeMap<String, Vec<Value>> {
        self.tables
            .lock()
            .expect("FakeDatabase mutex poisoned")
            .clone()
    }
}

/// Every publish, in arrival order -- the runner drains these directly
/// into a worker's own `handle_message_once`, the same "the runner
/// pumps delivery, there is no running subscription loop" architecture
/// Python's `FakeBroker` uses (v0.17 M7). No independent per-`(subject,
/// group)` cursors yet (every registered worker/job in this milestone's
/// vertical slice has exactly one consumer), so this is deliberately
/// narrower than Python's fan-out-capable broker fake.
#[derive(Debug, Default)]
pub struct FakeQueue {
    published: Mutex<Vec<(String, Vec<u8>)>>,
}

impl FakeQueue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn publish(&self, subject: &str, payload: Vec<u8>) {
        self.published
            .lock()
            .expect("FakeQueue mutex poisoned")
            .push((subject.to_owned(), payload));
    }

    /// Every message published since the last drain, removing them --
    /// the runner's own `drain` step calls this once per subject it
    /// knows how to deliver.
    pub fn take_all(&self) -> Vec<(String, Vec<u8>)> {
        std::mem::take(&mut self.published.lock().expect("FakeQueue mutex poisoned"))
    }

    /// Non-destructive peek -- `expect.quiescence` needs to observe
    /// whether anything is still undelivered without consuming it,
    /// unlike `take_all`.
    pub fn is_empty(&self) -> bool {
        self.published
            .lock()
            .expect("FakeQueue mutex poisoned")
            .is_empty()
    }
}

/// 27UpdatePlan.md M2 (Pillar 2): one ordered log per subject; every
/// consumer *group* holds its own independent cursor into that log —
/// two groups on one subject each see every message (fan-out), two
/// workers sharing one group share its cursor (queue-group semantics).
/// `take_next` peeks without consuming; only `ack` advances the cursor,
/// and `nack` is an explicit no-op — the message is redelivered on the
/// next `take_next` of that same `(subject, group)` iff it was never
/// acked, matching Pillar 1's own "redelivered iff not acked" contract.
///
/// Transitional alongside [`FakeQueue`]: `SimWorld::publish_checked`
/// writes into both so this broker is fully live and unit-tested from
/// M2 onward without disturbing the still-narrow Rust runner's existing
/// `world.queue` calls — M4 retires `FakeQueue`/`world.queue` once the
/// runner switches over to this one.
#[derive(Debug, Default)]
pub struct BrokerLog {
    logs: Mutex<BTreeMap<String, Vec<Vec<u8>>>>,
    cursors: Mutex<BTreeMap<(String, String), usize>>,
}

impl BrokerLog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn publish(&self, subject: &str, payload: Vec<u8>) {
        self.logs
            .lock()
            .expect("BrokerLog mutex poisoned")
            .entry(subject.to_owned())
            .or_default()
            .push(payload);
    }

    /// The next unconsumed message for `(subject, group)` — `None` at
    /// log head. Does not advance the cursor; call `ack` to consume it.
    pub fn take_next(&self, subject: &str, group: &str) -> Option<Vec<u8>> {
        let logs = self.logs.lock().expect("BrokerLog mutex poisoned");
        let log = logs.get(subject)?;
        let cursors = self.cursors.lock().expect("BrokerLog mutex poisoned");
        let pos = cursors
            .get(&(subject.to_owned(), group.to_owned()))
            .copied()
            .unwrap_or(0);
        log.get(pos).cloned()
    }

    /// Advances `(subject, group)`'s cursor past the message it is
    /// currently pointed at — the successful-delivery acknowledgement.
    pub fn ack(&self, subject: &str, group: &str) {
        *self
            .cursors
            .lock()
            .expect("BrokerLog mutex poisoned")
            .entry((subject.to_owned(), group.to_owned()))
            .or_insert(0) += 1;
    }

    /// Explicit no-op, named for the runner's own delivery-loop clarity
    /// (26UpdatePlan.md-style "say what you mean" over a bare `{}`):
    /// the cursor is left exactly where it was, so the same message
    /// comes back on the next `take_next` — this *is* the redelivery
    /// model, not a missing feature.
    pub fn nack(&self, _subject: &str, _group: &str) {}

    /// Every unconsumed message for `(subject, group)`, in log order,
    /// each individually acked as it is collected — equivalent to
    /// repeatedly calling `take_next`+`ack` until the log head.
    pub fn drain(&self, subject: &str, group: &str) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        while let Some(msg) = self.take_next(subject, group) {
            out.push(msg);
            self.ack(subject, group);
        }
        out
    }

    /// Unacked messages remaining for `(subject, group)`.
    pub fn pending_count(&self, subject: &str, group: &str) -> usize {
        let logs = self.logs.lock().expect("BrokerLog mutex poisoned");
        let len = logs.get(subject).map_or(0, Vec::len);
        let cursors = self.cursors.lock().expect("BrokerLog mutex poisoned");
        let pos = cursors
            .get(&(subject.to_owned(), group.to_owned()))
            .copied()
            .unwrap_or(0);
        len.saturating_sub(pos)
    }

    /// True iff every cursor this broker has ever been asked about
    /// (via `take_next`/`ack`/`drain`/`pending_count`) is caught up to
    /// its subject's current log head. A `(subject, group)` no one has
    /// asked about yet has no cursor, so it cannot be "pending" — the
    /// same scope `FakeQueue::is_empty` already holds (nothing
    /// unconsumed *that anyone is watching for*).
    pub fn queues_empty(&self) -> bool {
        let logs = self.logs.lock().expect("BrokerLog mutex poisoned");
        let cursors = self.cursors.lock().expect("BrokerLog mutex poisoned");
        cursors
            .iter()
            .all(|((subject, _), pos)| logs.get(subject).is_none_or(|log| *pos >= log.len()))
    }
}

/// 27UpdatePlan.md M3: cache-aside `cache.get`/`cache.set`/
/// `cache.delete`, TTL measured against `SimWorld`'s own
/// `VirtualClock` (never wall time) -- ported from
/// `sim/pyrunner/world.py`'s `FakeCache` (v0.17 M8). One per declared
/// `cache <Provider>` instance name, lazily created on first access,
/// exactly like Python's own `fake_cache(instance)`.
#[derive(Debug, Default)]
struct FakeCache {
    values: BTreeMap<String, String>,
    expire_at_ms: BTreeMap<String, i64>,
}

impl FakeCache {
    fn get(&mut self, key: &str, now_ms: i64) -> Option<String> {
        if let Some(&expire_at) = self.expire_at_ms.get(key) {
            if now_ms >= expire_at {
                self.values.remove(key);
                self.expire_at_ms.remove(key);
                return None;
            }
        }
        self.values.get(key).cloned()
    }

    fn set(&mut self, key: &str, value: String, ttl_ms: Option<i64>, now_ms: i64) {
        self.values.insert(key.to_owned(), value);
        match ttl_ms {
            Some(ttl) => {
                self.expire_at_ms.insert(key.to_owned(), now_ms + ttl);
            }
            None => {
                self.expire_at_ms.remove(key);
            }
        }
    }

    fn delete(&mut self, key: &str) {
        self.values.remove(key);
        self.expire_at_ms.remove(key);
    }
}

/// Enough of an S3/MinIO-shaped object store's interface
/// (`put`/`get`/`delete`/`list`) for `object_store.*` verbs -- an
/// in-memory key/bytes map, ported from Python's `FakeObjectStore`
/// (v0.17 M8).
#[derive(Debug, Default)]
struct FakeObjectStore {
    objects: BTreeMap<String, Vec<u8>>,
}

impl FakeObjectStore {
    fn put(&mut self, key: &str, body: Vec<u8>) {
        self.objects.insert(key.to_owned(), body);
    }

    fn get(&self, key: &str) -> Option<Vec<u8>> {
        self.objects.get(key).cloned()
    }

    fn delete(&mut self, key: &str) {
        self.objects.remove(key);
    }

    fn list(&self, prefix: &str) -> Vec<String> {
        self.objects
            .keys()
            .filter(|k| k.starts_with(prefix))
            .cloned()
            .collect()
    }
}

#[derive(Debug, Clone)]
struct SentEmail {
    to: String,
    subject: String,
    #[allow(dead_code)]
    body: String,
}

/// `email.send`, recording sent messages instead of talking SMTP --
/// ported from Python's `FakeEmail` (v0.17 M8). `body` is recorded
/// (matching Python's own `sent: list[dict]`, which keeps it) even
/// though `expect.email` only ever asserts `to`/`subject_contains`/
/// `count` today -- ported for parity, not for a hypothetical future
/// assertion.
#[derive(Debug, Default)]
struct FakeEmail {
    sent: Vec<SentEmail>,
}

impl FakeEmail {
    fn send(&mut self, to: &str, subject: &str, body: &str) {
        self.sent.push(SentEmail {
            to: to.to_owned(),
            subject: subject.to_owned(),
            body: body.to_owned(),
        });
    }
}

/// `search.index`/`search.query`/`search.delete` -- an in-memory
/// index/doc_id/document map with the same narrow, disclosed query
/// evaluator Python's `FakeSearch` (v0.17 M8) uses: case-insensitive
/// substring match over each document's JSON, matching the shape
/// `search.query` actually lowers to (`docs/expressions.md`'s own
/// `{"query": {"query_string": {"query": <text>}}}` note), not a
/// real query language.
#[derive(Debug, Default)]
struct FakeSearch {
    docs: BTreeMap<String, Value>,
}

impl FakeSearch {
    fn index(&mut self, doc_id: &str, document: Value) {
        self.docs.insert(doc_id.to_owned(), document);
    }

    fn query(&self, text: &str) -> Vec<Value> {
        if text.is_empty() {
            return self.docs.values().cloned().collect();
        }
        let needle = text.to_lowercase();
        self.docs
            .values()
            .filter(|doc| doc.to_string().to_lowercase().contains(&needle))
            .cloned()
            .collect()
    }

    fn delete(&mut self, doc_id: &str) {
        self.docs.remove(doc_id);
    }
}

/// A fixture-driven stand-in for an external HTTP client, consuming
/// [`crate::scenario::GivenHttpResponse`] fixtures in order -- the
/// same portable-scenario fixtures a `given.external_http` block
/// already carries drive this fake directly, not a second ad hoc
/// format, matching Python's `FakeHttpClient` (v0.17 M8).
#[derive(Debug, Default)]
struct FakeHttpClient {
    responses: Vec<crate::scenario::GivenHttpResponse>,
    next: usize,
    requests: Vec<(String, Value)>,
}

impl FakeHttpClient {
    fn post(&mut self, url: &str, json: Value) -> anyhow::Result<Value> {
        self.requests.push((url.to_owned(), json.clone()));
        let Some(response) = self.responses.get(self.next) else {
            anyhow::bail!(
                "no fixture response configured for external_http call #{} to {url:?}",
                self.next + 1
            );
        };
        self.next += 1;
        match response {
            crate::scenario::GivenHttpResponse::Error { error } => {
                anyhow::bail!("simulated external_http failure: {error}")
            }
            crate::scenario::GivenHttpResponse::Ok { json, .. } => Ok(json.clone()),
        }
    }
}

/// A bearer token had no configured claims, or its configured expiry
/// (against the virtual clock, never wall time) has passed -- mapped
/// to a 401 by the generated auth guard, matching what real JWT/JWKS
/// verification failure already does today.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthError(String);

impl std::fmt::Display for AuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for AuthError {}

/// Verifies a bearer token by direct lookup against claims the
/// runner configured ahead of time, instead of real JWT/JWKS crypto
/// -- the disclosed simplification Python's own `FakeAuth` (v0.17 M8)
/// already uses: rather than faking the JWKS HTTP round-trip, this
/// bypasses JWT/JWKS verification entirely while keeping scope
/// enforcement real. Singular (unlike cache/store/email/search/http),
/// matching Python's own `SimWorld.auth: FakeAuth` -- one identity
/// provider per world, not per-instance.
#[derive(Debug, Default)]
struct FakeAuth {
    tokens: BTreeMap<String, (Value, Option<i64>)>,
}

impl FakeAuth {
    fn issue(&mut self, token: &str, claims: Value, expire_at_ms: Option<i64>) {
        self.tokens.insert(token.to_owned(), (claims, expire_at_ms));
    }

    fn verify(&self, token: &str, now_ms: i64) -> Result<Value, AuthError> {
        let (claims, expire_at) = self
            .tokens
            .get(token)
            .ok_or_else(|| AuthError(format!("no configured claims for bearer token {token:?}")))?;
        if let Some(expire_at) = expire_at {
            if now_ms >= *expire_at {
                return Err(AuthError(format!("bearer token {token:?} has expired")));
            }
        }
        Ok(claims.clone())
    }
}

/// The simulation adapter `AppState::simulation(world)` constructs
/// from in generated Rust code -- one instance per running scenario,
/// shared (via `Arc`) across every cloned `AppState` handle exactly
/// like production's connection pools.
///
/// 27UpdatePlan.md M2: `schema`/`broker`/`clock`/`entropy` are new,
/// additive fields alongside the original `db`/`queue`/`failures` --
/// `db`, `queue`, `SimWorld::new`'s signature, and every method that
/// existed before this milestone are unchanged, per the "existing
/// public surface unbroken" constraint (the vendored
/// `sim_runner.rs.j2` template calls `SimWorld::new(...)`,
/// `world.db.insert(...)`, `world.queue.take_all()`/`is_empty()`
/// directly and cannot be disturbed before M4's runner swap-over).
/// 27UpdatePlan.md M3 adds the peripheral fakes (cache/object store/
/// email/search/http/auth) the same way -- additive fields, instance-
/// keyed maps lazily populated on first access except `auth`
/// (singular, per Python's own shape).
#[derive(Debug)]
pub struct SimWorld {
    pub db: FakeDatabase,
    pub queue: FakeQueue,
    /// 27UpdatePlan.md M2: transitional alongside `queue` -- see
    /// [`BrokerLog`]'s own docs. `publish_checked` writes into both.
    pub broker: BrokerLog,
    schema: RelationalSchema,
    failures: Mutex<FailureEngine>,
    clock: Mutex<VirtualClock>,
    entropy: Mutex<Entropy>,
    caches: Mutex<BTreeMap<String, FakeCache>>,
    object_stores: Mutex<BTreeMap<String, FakeObjectStore>>,
    emails: Mutex<BTreeMap<String, FakeEmail>>,
    searches: Mutex<BTreeMap<String, FakeSearch>>,
    http_clients: Mutex<BTreeMap<String, FakeHttpClient>>,
    auth: Mutex<FakeAuth>,
    /// 28UpdatePlan.md M2 (Pillar 3): every registered api handler,
    /// keyed by `(service, api)` -- `register_api`'s own bookkeeping.
    /// `Arc`, not `Box`: `call_checked` must clone the handler out and
    /// drop this map's lock before invoking it, since the handler may
    /// itself call back into `call_checked` (a `std::sync::Mutex` is
    /// not reentrant -- holding this lock across the call would
    /// deadlock the very first recursive call, on the same thread).
    apis: Mutex<ApiRegistry>,
    /// 28UpdatePlan.md M2: routed-call recursion depth, incremented for
    /// the duration of one `call_checked` invocation (RAII-released via
    /// `CallDepthGuard` so it unwinds correctly even if the handler
    /// panics). Not a cycle detector -- `ciac-sema`'s own
    /// `CycleDetection` pass already refuses a call cycle at compile
    /// time (`EdgeKind::ServiceCall` is one of the edge kinds its
    /// combined flow-graph check considers), so a cycle can never reach
    /// a compiled program's own routed calls. This guard exists for
    /// the case that check cannot cover -- a hand-built [`SimWorld`]
    /// (this crate's own tests, or a future non-generated driver)
    /// whose registered handlers call each other in a way the compiler
    /// never saw -- turning what would otherwise be an unrecoverable
    /// stack overflow into a graceful `Err`.
    call_depth: Mutex<u32>,
}

/// 28UpdatePlan.md M2: one registered api's logic, at the same JSON
/// boundary every other fake (`FakeHttpClient::post`, most directly)
/// already crosses -- `ciac-sim` has no knowledge of any particular
/// `.ciac` program's typed request/response shapes, so a handler is
/// exactly the bridge a world-guarded generated call site builds: one
/// `serde_json::to_value`/`from_value` round trip around the real
/// (fake-world-routed) handler logic.
pub type ApiHandler = dyn Fn(Value) -> anyhow::Result<Value> + Send + Sync;

/// A trait object has no `Debug` impl of its own, and `SimWorld` derives
/// `Debug` -- this newtype just reports how many handlers are
/// registered rather than trying to print them.
#[derive(Default)]
struct ApiRegistry(BTreeMap<(String, String), std::sync::Arc<ApiHandler>>);

impl std::fmt::Debug for ApiRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ApiRegistry({} handlers)", self.0.len())
    }
}

/// The `call_checked` recursion depth this crate refuses past --
/// generous relative to any real `.ciac` call chain (the compiler
/// already refuses cycles, so a legitimate chain is only ever as deep
/// as the program's own longest acyclic `call` path), tight enough to
/// fail long before it would risk a real stack overflow.
const MAX_CALL_DEPTH: u32 = 64;

/// RAII decrement for [`SimWorld::call_depth`] -- guarantees the depth
/// counter unwinds even if the routed handler panics, so a caught panic
/// elsewhere (a test harness, a future non-generated driver) never
/// leaves every subsequent call permanently refused.
struct CallDepthGuard<'a>(&'a Mutex<u32>);

impl Drop for CallDepthGuard<'_> {
    fn drop(&mut self) {
        *self.0.lock().expect("call depth mutex poisoned") -= 1;
    }
}

impl SimWorld {
    pub fn new(failure_rules: Vec<FailureRule>) -> Self {
        Self::with_schema(failure_rules, Vec::new())
    }

    /// 27UpdatePlan.md M2: like `new`, but with a [`WorldTable`] list to
    /// build a [`RelationalSchema`] from -- reference existence,
    /// `unique`, and cascade/restrict-on-delete checking is a no-op
    /// for any table `new()`'s empty schema doesn't know about
    /// (including every table in this milestone's own pre-existing
    /// tests), so this is additive: it cannot change the outcome of
    /// any call built through `new()`.
    pub fn with_schema(failure_rules: Vec<FailureRule>, tables: Vec<WorldTable>) -> Self {
        Self {
            db: FakeDatabase::new(),
            queue: FakeQueue::new(),
            broker: BrokerLog::new(),
            schema: RelationalSchema::from_tables(&tables),
            failures: Mutex::new(FailureEngine::new(failure_rules)),
            // Start-at-zero clock and a fixed seed: neither is yet
            // customizable per scenario (no `given.clock`/`given.seed`
            // field exists), so every world starts from the same
            // deterministic point until a later milestone wires one
            // through -- disclosed here rather than left implicit.
            clock: Mutex::new(VirtualClock::new(0)),
            entropy: Mutex::new(Entropy::new(0)),
            caches: Mutex::new(BTreeMap::new()),
            object_stores: Mutex::new(BTreeMap::new()),
            emails: Mutex::new(BTreeMap::new()),
            searches: Mutex::new(BTreeMap::new()),
            http_clients: Mutex::new(BTreeMap::new()),
            auth: Mutex::new(FakeAuth::default()),
            apis: Mutex::new(ApiRegistry::default()),
            call_depth: Mutex::new(0),
        }
    }

    /// 28UpdatePlan.md M2 (Pillar 3, "registration bookkeeping,
    /// runner-only"): registers `handler` as `service`'s `api`, the
    /// target `call_checked` resolves a routed call against. A system
    /// runner (M3+) calls this once per api, in service declaration
    /// order, before any scenario runs; re-registering the same
    /// `(service, api)` replaces the earlier handler rather than
    /// erroring, matching how a runner would simply rebuild the world
    /// (and re-register everything) between scenario runs rather than
    /// asking this registry to detect staleness itself.
    pub fn register_api(&self, service: &str, api: &str, handler: std::sync::Arc<ApiHandler>) {
        self.apis
            .lock()
            .expect("api handlers mutex poisoned")
            .0
            .insert((service.to_owned(), api.to_owned()), handler);
    }

    /// 28UpdatePlan.md M2 (Pillar 2/3): routes a `call <Service>.<Api>`
    /// step -- inline and synchronous on the caller's own logical
    /// thread, exactly as production's typed call client behaves, just
    /// against the registered handler instead of real HTTP. `call`s
    /// count against the `call.request` failure vocabulary the same
    /// way `db.commit`/`broker.publish` do; an injected failure fires
    /// before the handler ever runs, and an unregistered `(callee_service,
    /// api)` is a clear error rather than a silent no-op -- the same
    /// "fail closed on a missing seam" discipline `FakeHttpClient`
    /// already uses for an unmatched fixture. The handler's own
    /// `Ok`/`Err` passes through verbatim, matching production's own
    /// "the callee's real response or real error, not a synthesized
    /// one."
    pub fn call_checked(
        &self,
        caller: &str,
        callee_service: &str,
        api: &str,
        req: Value,
    ) -> anyhow::Result<Value> {
        if self.should_fail("call.request", Some(api))? {
            anyhow::bail!("simulated call.request failure (injected by FailureEngine)");
        }
        let handler = {
            let handlers = self.apis.lock().expect("api handlers mutex poisoned");
            handlers
                .0
                .get(&(callee_service.to_owned(), api.to_owned()))
                .cloned()
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "no handler registered for {callee_service}.{api} (called from {caller})"
                    )
                })?
        };
        {
            let mut depth = self.call_depth.lock().expect("call depth mutex poisoned");
            if *depth >= MAX_CALL_DEPTH {
                anyhow::bail!(
                    "routed call depth exceeded {MAX_CALL_DEPTH} calling {callee_service}.{api} \
                     from {caller} -- ciac-sema's own CycleDetection pass already refuses a call \
                     cycle at compile time, so this guards against a process-ending stack \
                     overflow in a hand-built world, not a reachable cycle in a compiled program"
                );
            }
            *depth += 1;
        }
        let _guard = CallDepthGuard(&self.call_depth);
        handler(req)
    }

    /// `db.commit`'s failure check, schema-aware reference/uniqueness
    /// validation, then the actual (fake) write, mirroring Python's
    /// `_FakeSession.commit()`: the failure check runs first (a
    /// matched rule raises before anything else is even attempted,
    /// same as Python's own ordering), then the row is validated
    /// against a scratch overlay of the live store before it is ever
    /// applied for real. Only the `error` failure action is
    /// implemented -- the same disclosed subset of the full
    /// `FailureAction` vocabulary Python's own restatement supports.
    pub fn db_insert_checked(&self, table: &str, row: Value) -> anyhow::Result<()> {
        if self.should_fail("db.commit", Some(table))? {
            anyhow::bail!("simulated db.commit failure (injected by FailureEngine)");
        }
        let overlay = self.db.snapshot();
        self.validate_write(&overlay, table, &row, None)?;
        self.db.insert(table, row);
        Ok(())
    }

    /// 27UpdatePlan.md M2: full-record replace by `id`, matching
    /// production `db.update`'s own shape (M1: `UPDATE <table> SET
    /// <every column> WHERE id = <pk>`) -- `Ok(None)` if no row with
    /// that `id` exists, exactly like production's `rows_affected() ==
    /// 0` case, not an error; a real constraint violation (dangling
    /// reference, unique conflict) is an error, since that is a bug in
    /// the scenario's own data, not a "not found."
    pub fn db_update_checked(
        &self,
        table: &str,
        pk: &str,
        record: Value,
    ) -> anyhow::Result<Option<Value>> {
        if self.should_fail("db.commit", Some(table))? {
            anyhow::bail!("simulated db.commit failure (injected by FailureEngine)");
        }
        if self.db.get(table, pk).is_none() {
            return Ok(None);
        }
        let overlay = self.db.snapshot();
        self.validate_write(&overlay, table, &record, Some(pk))?;
        self.db.replace(table, pk, record.clone());
        Ok(Some(record))
    }

    /// 27UpdatePlan.md M2: delete by `id`, resolving cascade/restrict
    /// references first via a plan-then-apply pass (mirroring Python's
    /// `_plan_delete`) -- `Ok(false)` if no row with that `id` exists,
    /// matching production `db.delete`'s own `rows_affected() > 0`
    /// shape; a `RestrictedDelete` is an error, since a real database
    /// would refuse the same delete with a foreign-key violation.
    pub fn db_delete_checked(&self, table: &str, pk: &str) -> anyhow::Result<bool> {
        if self.should_fail("db.commit", Some(table))? {
            anyhow::bail!("simulated db.commit failure (injected by FailureEngine)");
        }
        if self.db.get(table, pk).is_none() {
            return Ok(false);
        }
        let overlay = self.db.snapshot();
        let cascaded = self.plan_delete(&overlay, table, pk)?;
        self.db.delete(table, pk);
        for (dep_table, dep_pk) in &cascaded {
            self.db.delete(dep_table, dep_pk);
        }
        Ok(true)
    }

    /// 27UpdatePlan.md M2: the `transaction {}` leaf's own accumulation
    /// point -- every insert/delete in `ops` validates against one
    /// scratch overlay of the live store before any of it is applied
    /// for real, mirroring Python's `commit_batch`: a violation on the
    /// second op of a five-op batch leaves the store exactly as it was
    /// before the call, not partially written. Returns every `(table,
    /// pk)` actually deleted, including cascades triggered by a
    /// `Delete` op, so a caller can record each as its own transcript
    /// effect. A single `db.commit` failure-injection check covers the
    /// whole batch (Python's own "the subject is the first pending
    /// item's table" convention), not one check per op.
    pub fn commit_batch_checked(&self, ops: Vec<BatchOp>) -> anyhow::Result<Vec<(String, String)>> {
        let subject = ops.first().map(|op| match op {
            BatchOp::Insert { table, .. }
            | BatchOp::Delete { table, .. }
            | BatchOp::Update { table, .. } => table.clone(),
        });
        if let Some(subject) = &subject {
            if self.should_fail("db.commit", Some(subject))? {
                anyhow::bail!("simulated db.commit failure (injected by FailureEngine)");
            }
        }

        let mut overlay = self.db.snapshot();
        let mut all_deletes: Vec<(String, String)> = Vec::new();

        for op in &ops {
            match op {
                BatchOp::Insert { table, row } => {
                    self.validate_write(&overlay, table, row, None)?;
                    overlay.entry(table.clone()).or_default().push(row.clone());
                }
                BatchOp::Update { table, pk, row } => {
                    self.validate_write(&overlay, table, row, Some(pk))?;
                    if let Some(rows) = overlay.get_mut(table) {
                        if let Some(existing) = rows
                            .iter_mut()
                            .find(|r| r.get("id").and_then(Value::as_str) == Some(pk))
                        {
                            *existing = row.clone();
                        }
                    }
                }
                BatchOp::Delete { table, pk } => {
                    let cascaded = self.plan_delete(&overlay, table, pk)?;
                    for (dep_table, dep_pk) in
                        std::iter::once((table.clone(), pk.clone())).chain(cascaded.iter().cloned())
                    {
                        if let Some(rows) = overlay.get_mut(&dep_table) {
                            rows.retain(|row| {
                                row.get("id").and_then(Value::as_str) != Some(dep_pk.as_str())
                            });
                        }
                    }
                    all_deletes.push((table.clone(), pk.clone()));
                    all_deletes.extend(cascaded);
                }
            }
        }

        // Every op above validated against the overlay without ever
        // touching the real store -- reaching here means the whole
        // batch is safe to apply for real, all at once.
        for op in &ops {
            match op {
                BatchOp::Insert { table, row } => self.db.insert(table, row.clone()),
                BatchOp::Update { table, pk, row } => {
                    self.db.replace(table, pk, row.clone());
                }
                BatchOp::Delete { .. } => {}
            }
        }
        for (table, pk) in &all_deletes {
            self.db.delete(table, pk);
        }
        Ok(all_deletes)
    }

    /// 28UpdatePlan.md M2 (Pillar 2): the storage key a service-
    /// addressed relational call actually uses -- `service::table` when
    /// a service is given, or the bare table name for the single-
    /// service degenerate path. Un-suffixed callers (`db_insert_checked`
    /// and friends, every one of 27's own call sites) never pass a
    /// service and so always resolve to the bare key, unchanged --
    /// 27's own corpus staying green through this milestone is the
    /// proof the degenerate path really is a no-op, not just a claim.
    ///
    /// Composition matters here specifically because `ciac-sim` is
    /// usable standalone (this crate's own tests build a [`SimWorld`]
    /// directly, and a future non-generated driver could too) --
    /// `ciac-sema`'s `DuplicateDeclaration` check only rules out two
    /// same-named tables for a real compiled `.ciac` program, not for
    /// two hand-built [`WorldTable`]s a caller happens to name alike.
    pub fn namespaced_table_key(service: Option<&str>, table: &str) -> String {
        match service {
            Some(service) => format!("{service}::{table}"),
            None => table.to_owned(),
        }
    }

    /// 28UpdatePlan.md M2: [`Self::db_insert_checked`], addressed
    /// through `service` -- composes the namespaced key and delegates,
    /// so failure injection, reference/uniqueness validation, and the
    /// actual write all happen exactly once, in the un-suffixed method.
    pub fn db_insert_checked_for(
        &self,
        service: Option<&str>,
        table: &str,
        row: Value,
    ) -> anyhow::Result<()> {
        self.db_insert_checked(&Self::namespaced_table_key(service, table), row)
    }

    /// 28UpdatePlan.md M2: [`Self::db_update_checked`], addressed
    /// through `service`.
    pub fn db_update_checked_for(
        &self,
        service: Option<&str>,
        table: &str,
        pk: &str,
        record: Value,
    ) -> anyhow::Result<Option<Value>> {
        self.db_update_checked(&Self::namespaced_table_key(service, table), pk, record)
    }

    /// 28UpdatePlan.md M2: [`Self::db_delete_checked`], addressed
    /// through `service`.
    pub fn db_delete_checked_for(
        &self,
        service: Option<&str>,
        table: &str,
        pk: &str,
    ) -> anyhow::Result<bool> {
        self.db_delete_checked(&Self::namespaced_table_key(service, table), pk)
    }

    /// A prospective write's reference/uniqueness check against
    /// `overlay` (the live store, or a batch's in-progress scratch
    /// copy) rather than `self.db` directly, so a multi-op batch sees
    /// its own earlier ops' effects. `self_pk` is `Some(pk)` for an
    /// update replacing that row -- excluded from both the primary-key
    /// collision check (an update legitimately keeps its own `id`) and
    /// the unique-reference check (a row does not conflict with
    /// itself) -- and `None` for an insert, where no existing row may
    /// already claim this `id` or a `unique` reference's value.
    fn validate_write(
        &self,
        overlay: &BTreeMap<String, Vec<Value>>,
        table: &str,
        row: &Value,
        self_pk: Option<&str>,
    ) -> Result<(), RelationalError> {
        if self_pk.is_none() {
            if let Some(pk) = row.get("id").and_then(Value::as_str) {
                let collides = overlay
                    .get(table)
                    .into_iter()
                    .flatten()
                    .any(|r| r.get("id").and_then(Value::as_str) == Some(pk));
                if collides {
                    return Err(RelationalError::UniqueViolation {
                        table: table.to_owned(),
                        field: "id".to_owned(),
                        value: pk.to_owned(),
                    });
                }
            }
        }
        for field in self.schema.outgoing(table) {
            let Some(target_id) = row.get(&field.field_name).and_then(Value::as_str) else {
                continue;
            };
            // `target_table: None` means this reference's target has
            // no backing table in this plan's own frozen scope -- not
            // resolvable, so not checkable; see `SimFieldType::Reference`'s
            // own doc comment in plan.rs.
            let Some(target_table) = &field.target_table else {
                continue;
            };
            let target_exists = overlay
                .get(target_table.as_str())
                .into_iter()
                .flatten()
                .any(|r| r.get("id").and_then(Value::as_str) == Some(target_id));
            if !target_exists {
                return Err(RelationalError::MissingReference {
                    table: table.to_owned(),
                    field: field.field_name.clone(),
                    target_table: target_table.clone(),
                    target_pk: target_id.to_owned(),
                });
            }
            if field.unique {
                let conflict = overlay.get(table).into_iter().flatten().any(|r| {
                    r.get(&field.field_name).and_then(Value::as_str) == Some(target_id)
                        && r.get("id").and_then(Value::as_str) != self_pk
                });
                if conflict {
                    return Err(RelationalError::UniqueViolation {
                        table: table.to_owned(),
                        field: field.field_name.clone(),
                        value: target_id.to_owned(),
                    });
                }
            }
        }
        Ok(())
    }

    /// Every `(referencing_table, pk)` a delete of `table`/`pk` would
    /// cascade into, recursively, or `Err(RestrictedDelete)` if a
    /// `Restrict` reference blocks it -- mirroring Python's own
    /// `_plan_delete`: read-only against `overlay`, the caller applies
    /// the result.
    fn plan_delete(
        &self,
        overlay: &BTreeMap<String, Vec<Value>>,
        table: &str,
        pk: &str,
    ) -> Result<Vec<(String, String)>, RelationalError> {
        let mut planned = Vec::new();
        for (referencing_table, field) in self.schema.incoming(table) {
            let dependents: Vec<String> = overlay
                .get(referencing_table)
                .into_iter()
                .flatten()
                .filter(|row| row.get(&field.field_name).and_then(Value::as_str) == Some(pk))
                .filter_map(|row| row.get("id").and_then(Value::as_str).map(str::to_owned))
                .collect();
            if dependents.is_empty() {
                continue;
            }
            if field.on_delete == WorldRefAction::Restrict {
                return Err(RelationalError::RestrictedDelete {
                    table: table.to_owned(),
                    pk: pk.to_owned(),
                    referencing_table: referencing_table.to_owned(),
                    referencing_field: field.field_name.clone(),
                });
            }
            for dep_pk in dependents {
                planned.push((referencing_table.to_owned(), dep_pk.clone()));
                planned.extend(self.plan_delete(overlay, referencing_table, &dep_pk)?);
            }
        }
        Ok(planned)
    }

    /// `broker.publish`'s failure check plus the actual (fake) publish
    /// -- into both `queue` (unchanged, still what the runner drains
    /// today) and `broker` (new, fully live from M2 onward; M4 retires
    /// `queue` once the runner switches over).
    pub fn publish_checked(&self, subject: &str, payload: Vec<u8>) -> anyhow::Result<()> {
        if self.should_fail("broker.publish", Some(subject))? {
            anyhow::bail!("simulated broker.publish failure (injected by FailureEngine)");
        }
        self.queue.publish(subject, payload.clone());
        self.broker.publish(subject, payload);
        Ok(())
    }

    /// Current virtual time, in epoch milliseconds -- Pillar 2's "clock
    /// wired through the world rather than beside it."
    pub fn now_ms(&self) -> i64 {
        self.clock
            .lock()
            .expect("VirtualClock mutex poisoned")
            .now_ms()
    }

    /// Advances virtual time forward. See [`VirtualClock::advance_to`]
    /// for the monotonicity panic this delegates to.
    pub fn advance_clock_to(&self, to_ms: i64) {
        self.clock
            .lock()
            .expect("VirtualClock mutex poisoned")
            .advance_to(to_ms);
    }

    /// The next value from this world's seeded, deterministic UUID
    /// stream -- never the host's random source.
    pub fn next_uuid(&self) -> String {
        self.entropy
            .lock()
            .expect("Entropy mutex poisoned")
            .next_uuid()
    }

    /// 27UpdatePlan.md M3: `cache.get` -- `None` if absent or expired
    /// (against this world's own virtual clock, never wall time). No
    /// failure-injection check, matching Python's own `FakeCache`
    /// (which never calls `should_fail` either).
    pub fn cache_get(&self, instance: &str, key: &str) -> Option<String> {
        let now = self.now_ms();
        self.caches
            .lock()
            .expect("cache mutex poisoned")
            .entry(instance.to_owned())
            .or_default()
            .get(key, now)
    }

    /// `cache.set`. `ttl_ms` is `None` for no expiry -- parsing a
    /// scenario's `given.cache.ttl` duration string (`"30m"`) into
    /// milliseconds is the runner's own job (the same split
    /// `advance.by` already uses), not this method's.
    pub fn cache_set(&self, instance: &str, key: &str, value: String, ttl_ms: Option<i64>) {
        let now = self.now_ms();
        self.caches
            .lock()
            .expect("cache mutex poisoned")
            .entry(instance.to_owned())
            .or_default()
            .set(key, value, ttl_ms, now);
    }

    /// `cache.delete`.
    pub fn cache_delete(&self, instance: &str, key: &str) {
        self.caches
            .lock()
            .expect("cache mutex poisoned")
            .entry(instance.to_owned())
            .or_default()
            .delete(key);
    }

    /// `object_store.put`.
    pub fn object_put(&self, instance: &str, key: &str, body: Vec<u8>) {
        self.object_stores
            .lock()
            .expect("object store mutex poisoned")
            .entry(instance.to_owned())
            .or_default()
            .put(key, body);
    }

    /// `object_store.get` -- errors if no object exists at `key`,
    /// matching Python's `FakeObjectStore.get` (`self._objects[key]`,
    /// which raises `KeyError` on a miss rather than returning
    /// `None`).
    pub fn object_get(&self, instance: &str, key: &str) -> anyhow::Result<Vec<u8>> {
        self.object_stores
            .lock()
            .expect("object store mutex poisoned")
            .entry(instance.to_owned())
            .or_default()
            .get(key)
            .ok_or_else(|| {
                anyhow::anyhow!("object_store: no object at key {key:?} in instance {instance:?}")
            })
    }

    /// `object_store.delete`.
    pub fn object_delete(&self, instance: &str, key: &str) {
        self.object_stores
            .lock()
            .expect("object store mutex poisoned")
            .entry(instance.to_owned())
            .or_default()
            .delete(key);
    }

    /// `object_store.list` -- every key with the given prefix, sorted
    /// (matching Python's `sorted(...)`; `BTreeMap`'s own iteration
    /// order already guarantees this).
    pub fn object_list(&self, instance: &str, prefix: &str) -> Vec<String> {
        self.object_stores
            .lock()
            .expect("object store mutex poisoned")
            .entry(instance.to_owned())
            .or_default()
            .list(prefix)
    }

    /// `expect.object`'s own presence check -- distinct from
    /// `object_get`'s "found or error" shape, since a presence
    /// assertion should never itself be treated as a scenario error.
    pub fn object_exists(&self, instance: &str, key: &str) -> bool {
        self.object_stores
            .lock()
            .expect("object store mutex poisoned")
            .get(instance)
            .is_some_and(|store| store.objects.contains_key(key))
    }

    /// `email.send`.
    pub fn email_send(&self, instance: &str, to: &str, subject: &str, body: &str) {
        self.emails
            .lock()
            .expect("email mutex poisoned")
            .entry(instance.to_owned())
            .or_default()
            .send(to, subject, body);
    }

    /// Per-instance count query: every sent message on `instance`
    /// matching `to` (if given) and containing `subject_contains` (if
    /// given).
    pub fn email_sent_count(
        &self,
        instance: &str,
        to: Option<&str>,
        subject_contains: Option<&str>,
    ) -> usize {
        self.emails
            .lock()
            .expect("email mutex poisoned")
            .get(instance)
            .into_iter()
            .flat_map(|email| email.sent.iter())
            .filter(|m| to.is_none_or(|t| m.to == t))
            .filter(|m| subject_contains.is_none_or(|s| m.subject.contains(s)))
            .count()
    }

    /// `expect.email`'s own count query -- deliberately not scoped to
    /// one instance: `ciac_sim::scenario::ExpectStep::Email` carries no
    /// `instance` field, matching Python's own `_expect_email`
    /// (`sim/pyrunner/scenario_runner.py`), which counts sends across
    /// every `email` instance the world has seen.
    pub fn email_sent_count_all(&self, to: Option<&str>, subject_contains: Option<&str>) -> usize {
        self.emails
            .lock()
            .expect("email mutex poisoned")
            .values()
            .flat_map(|email| email.sent.iter())
            .filter(|m| to.is_none_or(|t| m.to == t))
            .filter(|m| subject_contains.is_none_or(|s| m.subject.contains(s)))
            .count()
    }

    /// `search.index`.
    pub fn search_index(&self, instance: &str, doc_id: &str, document: Value) {
        self.searches
            .lock()
            .expect("search mutex poisoned")
            .entry(instance.to_owned())
            .or_default()
            .index(doc_id, document);
    }

    /// `search.query` -- case-insensitive substring match over each
    /// document's JSON, matching Python's own disclosed evaluator
    /// (not a real query language; see `FakeSearch`'s own doc
    /// comment above).
    pub fn search_query(&self, instance: &str, text: &str) -> Vec<Value> {
        self.searches
            .lock()
            .expect("search mutex poisoned")
            .entry(instance.to_owned())
            .or_default()
            .query(text)
    }

    /// `search.delete`.
    pub fn search_delete(&self, instance: &str, doc_id: &str) {
        self.searches
            .lock()
            .expect("search mutex poisoned")
            .entry(instance.to_owned())
            .or_default()
            .delete(doc_id);
    }

    /// Seeds `instance`'s fixture queue from a scenario's own
    /// `given.external_http` block -- the exact
    /// [`crate::scenario::GivenHttpResponse`] shape a checked-in
    /// scenario already carries, consumed directly rather than
    /// translated into a second ad hoc format.
    pub fn seed_http_fixtures(
        &self,
        instance: &str,
        responses: Vec<crate::scenario::GivenHttpResponse>,
    ) {
        self.http_clients
            .lock()
            .expect("http mutex poisoned")
            .entry(instance.to_owned())
            .or_default()
            .responses = responses;
    }

    /// `external_http.request`'s POST call -- consumes the next
    /// unconsumed fixture in order, erroring if none remain or the
    /// fixture itself declares `{"error": ..}`.
    pub fn http_post(&self, instance: &str, url: &str, json: Value) -> anyhow::Result<Value> {
        self.http_clients
            .lock()
            .expect("http mutex poisoned")
            .entry(instance.to_owned())
            .or_default()
            .post(url, json)
    }

    /// `expect.http_calls`'s own count query.
    pub fn http_request_count(&self, instance: &str) -> usize {
        self.http_clients
            .lock()
            .expect("http mutex poisoned")
            .get(instance)
            .map_or(0, |client| client.requests.len())
    }

    /// Configures the claims (and optional expiry, against this
    /// world's own virtual clock) a bearer token verifies to --
    /// the runner's own equivalent of Python's `FakeAuth.issue`,
    /// called from a scenario's own auth setup, not from generated
    /// production code.
    pub fn auth_issue(&self, token: &str, claims: Value, expire_at_ms: Option<i64>) {
        self.auth
            .lock()
            .expect("auth mutex poisoned")
            .issue(token, claims, expire_at_ms);
    }

    /// The generated auth guard's own verification call -- claims-
    /// lookup, not real JWT/JWKS crypto, matching Python's `FakeAuth`
    /// (see its own doc comment above for the disclosed rationale).
    pub fn auth_verify(&self, token: &str) -> Result<Value, AuthError> {
        let now = self.now_ms();
        self.auth
            .lock()
            .expect("auth mutex poisoned")
            .verify(token, now)
    }

    fn should_fail(&self, effect: &str, subject: Option<&str>) -> anyhow::Result<bool> {
        let action = self
            .failures
            .lock()
            .expect("FailureEngine mutex poisoned")
            .record_occurrence(effect, subject, FailurePhase::After);
        match action {
            None => Ok(false),
            Some(FailureAction::Error) => Ok(true),
            Some(other) => anyhow::bail!(
                "failure rule for effect {effect:?} uses action {other:?}, which this Rust \
                 world's FailureEngine integration does not support yet (only `error` is \
                 implemented)"
            ),
        }
    }

    /// Failure rules the scenario declared that never matched a real
    /// occurrence -- `SIM0007` territory, surfaced to the runner rather
    /// than silently ignored.
    pub fn unmatched_failure_rules(&self) -> Vec<FailureRule> {
        self.failures
            .lock()
            .expect("FailureEngine mutex poisoned")
            .unmatched_rules()
            .into_iter()
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::failure::FailureSelector;

    #[test]
    fn insert_then_find_where_round_trips() {
        let db = FakeDatabase::new();
        db.insert("orders", serde_json::json!({"id": "1", "total": 9.5}));
        db.insert("orders", serde_json::json!({"id": "2", "total": 3.0}));
        assert_eq!(db.count("orders"), 2);
        let mut filter = BTreeMap::new();
        filter.insert("id".to_owned(), serde_json::json!("2"));
        let found = db.find_where("orders", &filter);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0]["total"], 3.0);
        assert!(db.find_where("orders", &BTreeMap::new()).len() == 2);
    }

    #[test]
    fn queue_take_all_drains_in_publish_order() {
        let queue = FakeQueue::new();
        queue.publish("a", b"1".to_vec());
        queue.publish("a", b"2".to_vec());
        let drained = queue.take_all();
        assert_eq!(
            drained,
            vec![
                ("a".to_owned(), b"1".to_vec()),
                ("a".to_owned(), b"2".to_vec())
            ]
        );
        assert!(
            queue.take_all().is_empty(),
            "a second drain sees nothing new"
        );
    }

    #[test]
    fn queue_is_empty_peeks_without_consuming() {
        let queue = FakeQueue::new();
        assert!(queue.is_empty());
        queue.publish("a", b"1".to_vec());
        assert!(!queue.is_empty());
        assert!(!queue.is_empty(), "peeking twice does not drain");
        assert_eq!(queue.take_all().len(), 1);
        assert!(queue.is_empty());
    }

    #[test]
    fn db_insert_checked_fails_on_the_matched_occurrence_and_does_not_store_the_row() {
        let rule = FailureRule {
            at: FailureSelector {
                effect: "db.commit".into(),
                subject: Some("processed_orders".into()),
                occurrence: Some(1),
                phase: FailurePhase::After,
            },
            action: FailureAction::Error,
        };
        let world = SimWorld::new(vec![rule]);
        let row = serde_json::json!({"id": "1"});
        assert!(world
            .db_insert_checked("processed_orders", row.clone())
            .is_err());
        assert_eq!(
            world.db.count("processed_orders"),
            0,
            "failed commit stores nothing"
        );
        assert!(world.db_insert_checked("processed_orders", row).is_ok());
        assert_eq!(
            world.db.count("processed_orders"),
            1,
            "the second (unmatched) attempt commits"
        );
        assert!(world.unmatched_failure_rules().is_empty());
    }

    #[test]
    fn unsupported_failure_action_is_refused_not_silently_ignored() {
        let rule = FailureRule {
            at: FailureSelector {
                effect: "db.commit".into(),
                subject: None,
                occurrence: None,
                phase: FailurePhase::After,
            },
            action: FailureAction::Lose,
        };
        let world = SimWorld::new(vec![rule]);
        let err = world
            .db_insert_checked("orders", serde_json::json!({}))
            .unwrap_err();
        assert!(err.to_string().contains("does not support"));
    }

    #[test]
    fn fake_database_get_delete_replace_round_trip() {
        let db = FakeDatabase::new();
        assert_eq!(db.get("orders", "1"), None);
        db.insert("orders", serde_json::json!({"id": "1", "total": 9.5}));
        assert_eq!(
            db.get("orders", "1"),
            Some(serde_json::json!({"id": "1", "total": 9.5}))
        );
        assert!(db.replace("orders", "1", serde_json::json!({"id": "1", "total": 20.0})));
        assert_eq!(db.get("orders", "1").unwrap()["total"], 20.0);
        assert!(!db.replace("orders", "missing", serde_json::json!({"id": "missing"})));
        assert!(db.delete("orders", "1"));
        assert!(!db.delete("orders", "1"), "a second delete finds nothing");
        assert_eq!(db.get("orders", "1"), None);
    }

    /// Two tables -- `customers` (no references) and `orders` (one
    /// `unique`-or-not reference to `customers`, `on_delete` per the
    /// caller) -- the minimal shape M2's reference/cascade/restrict/
    /// unique tests all share.
    fn customers_and_orders_schema(on_delete: WorldRefAction, unique: bool) -> Vec<WorldTable> {
        vec![
            WorldTable {
                name: "customers".into(),
                references: vec![],
            },
            WorldTable {
                name: "orders".into(),
                references: vec![WorldReference {
                    field_name: "customer_id".into(),
                    target_table: Some("customers".into()),
                    on_delete,
                    unique,
                }],
            },
        ]
    }

    #[test]
    fn db_insert_checked_refuses_a_dangling_reference() {
        let world = SimWorld::with_schema(
            Vec::new(),
            customers_and_orders_schema(WorldRefAction::Cascade, false),
        );
        let err = world
            .db_insert_checked(
                "orders",
                serde_json::json!({"id": "o1", "customer_id": "missing"}),
            )
            .unwrap_err();
        assert!(err.to_string().contains("does not exist"), "{err}");
        assert_eq!(
            world.db.count("orders"),
            0,
            "the invalid insert did not land"
        );
    }

    #[test]
    fn db_insert_checked_refuses_a_unique_reference_conflict() {
        let world = SimWorld::with_schema(
            Vec::new(),
            customers_and_orders_schema(WorldRefAction::Cascade, true),
        );
        world
            .db_insert_checked("customers", serde_json::json!({"id": "c1"}))
            .unwrap();
        world
            .db_insert_checked(
                "orders",
                serde_json::json!({"id": "o1", "customer_id": "c1"}),
            )
            .unwrap();
        let err = world
            .db_insert_checked(
                "orders",
                serde_json::json!({"id": "o2", "customer_id": "c1"}),
            )
            .unwrap_err();
        assert!(err.to_string().contains("not unique"), "{err}");
        assert_eq!(world.db.count("orders"), 1);
    }

    #[test]
    fn db_update_checked_replaces_the_full_record_and_none_for_missing() {
        let world = SimWorld::new(Vec::new());
        world
            .db_insert_checked("orders", serde_json::json!({"id": "1", "total": 9.5}))
            .unwrap();
        let updated = world
            .db_update_checked("orders", "1", serde_json::json!({"id": "1", "total": 42.0}))
            .unwrap();
        assert_eq!(updated, Some(serde_json::json!({"id": "1", "total": 42.0})));
        assert_eq!(world.db.get("orders", "1").unwrap()["total"], 42.0);
        assert_eq!(
            world
                .db_update_checked("orders", "missing", serde_json::json!({"id": "missing"}))
                .unwrap(),
            None,
            "matches production's rows_affected() == 0 shape, not an error"
        );
    }

    #[test]
    fn db_update_checked_excludes_self_from_the_unique_check() {
        let world = SimWorld::with_schema(
            Vec::new(),
            customers_and_orders_schema(WorldRefAction::Cascade, true),
        );
        world
            .db_insert_checked("customers", serde_json::json!({"id": "c1"}))
            .unwrap();
        world
            .db_insert_checked(
                "orders",
                serde_json::json!({"id": "o1", "customer_id": "c1"}),
            )
            .unwrap();
        // Re-issuing the same order's own value must not trip the
        // unique check against itself.
        assert!(world
            .db_update_checked(
                "orders",
                "o1",
                serde_json::json!({"id": "o1", "customer_id": "c1"})
            )
            .is_ok());
    }

    #[test]
    fn db_delete_checked_cascades_dependents_when_on_delete_is_cascade() {
        let world = SimWorld::with_schema(
            Vec::new(),
            customers_and_orders_schema(WorldRefAction::Cascade, false),
        );
        world
            .db_insert_checked("customers", serde_json::json!({"id": "c1"}))
            .unwrap();
        world
            .db_insert_checked(
                "orders",
                serde_json::json!({"id": "o1", "customer_id": "c1"}),
            )
            .unwrap();
        assert!(world.db_delete_checked("customers", "c1").unwrap());
        assert_eq!(world.db.get("customers", "c1"), None);
        assert_eq!(
            world.db.get("orders", "o1"),
            None,
            "the dependent order cascaded away with its customer"
        );
        assert!(
            !world.db_delete_checked("customers", "c1").unwrap(),
            "a second delete finds nothing"
        );
    }

    #[test]
    fn db_delete_checked_refuses_when_on_delete_is_restrict() {
        let world = SimWorld::with_schema(
            Vec::new(),
            customers_and_orders_schema(WorldRefAction::Restrict, false),
        );
        world
            .db_insert_checked("customers", serde_json::json!({"id": "c1"}))
            .unwrap();
        world
            .db_insert_checked(
                "orders",
                serde_json::json!({"id": "o1", "customer_id": "c1"}),
            )
            .unwrap();
        let err = world.db_delete_checked("customers", "c1").unwrap_err();
        assert!(err.to_string().contains("restricted by"), "{err}");
        assert!(
            world.db.get("customers", "c1").is_some(),
            "a refused delete leaves the row in place"
        );
    }

    #[test]
    fn commit_batch_checked_rolls_back_the_whole_batch_on_a_mid_batch_violation() {
        let world = SimWorld::with_schema(
            Vec::new(),
            customers_and_orders_schema(WorldRefAction::Cascade, false),
        );
        let ops = vec![
            BatchOp::Insert {
                table: "customers".into(),
                row: serde_json::json!({"id": "c1"}),
            },
            BatchOp::Insert {
                table: "orders".into(),
                row: serde_json::json!({"id": "o1", "customer_id": "c1"}),
            },
            BatchOp::Insert {
                table: "orders".into(),
                row: serde_json::json!({"id": "o2", "customer_id": "does-not-exist"}),
            },
        ];
        assert!(world.commit_batch_checked(ops).is_err());
        assert_eq!(
            world.db.count("customers"),
            0,
            "nothing from the batch landed"
        );
        assert_eq!(world.db.count("orders"), 0, "nothing from the batch landed");
    }

    #[test]
    fn commit_batch_checked_applies_every_op_atomically_when_the_whole_batch_validates() {
        let world = SimWorld::with_schema(
            Vec::new(),
            customers_and_orders_schema(WorldRefAction::Cascade, false),
        );
        // A batch inserting a customer and an order referencing it in
        // the same call must see its own earlier op via the overlay --
        // this is exactly the case a plain live-db check would reject.
        let ops = vec![
            BatchOp::Insert {
                table: "customers".into(),
                row: serde_json::json!({"id": "c1"}),
            },
            BatchOp::Insert {
                table: "orders".into(),
                row: serde_json::json!({"id": "o1", "customer_id": "c1"}),
            },
        ];
        let deleted = world.commit_batch_checked(ops).unwrap();
        assert!(deleted.is_empty());
        assert_eq!(world.db.count("customers"), 1);
        assert_eq!(world.db.count("orders"), 1);

        let deletes = vec![BatchOp::Delete {
            table: "customers".into(),
            pk: "c1".into(),
        }];
        let deleted = world.commit_batch_checked(deletes).unwrap();
        assert_eq!(deleted.len(), 2, "the cascade into orders is reported too");
        assert_eq!(world.db.count("customers"), 0);
        assert_eq!(world.db.count("orders"), 0);
    }

    #[test]
    fn commit_batch_checked_applies_an_update_op_and_rolls_it_back_with_the_rest_on_violation() {
        let world = SimWorld::with_schema(
            Vec::new(),
            customers_and_orders_schema(WorldRefAction::Cascade, false),
        );
        world
            .db_insert_checked("customers", serde_json::json!({"id": "c1"}))
            .unwrap();
        world
            .db_insert_checked(
                "orders",
                serde_json::json!({"id": "o1", "customer_id": "c1"}),
            )
            .unwrap();

        // A batch replacing the order alongside an insert that violates
        // a dangling reference must roll the update back too, not just
        // the insert.
        let ops = vec![
            BatchOp::Update {
                table: "orders".into(),
                pk: "o1".into(),
                row: serde_json::json!({"id": "o1", "customer_id": "c1", "note": "updated"}),
            },
            BatchOp::Insert {
                table: "orders".into(),
                row: serde_json::json!({"id": "o2", "customer_id": "does-not-exist"}),
            },
        ];
        assert!(world.commit_batch_checked(ops).is_err());
        assert_eq!(
            world.db.get("orders", "o1").unwrap().get("note"),
            None,
            "the update did not land either"
        );

        let ops = vec![BatchOp::Update {
            table: "orders".into(),
            pk: "o1".into(),
            row: serde_json::json!({"id": "o1", "customer_id": "c1", "note": "updated"}),
        }];
        world.commit_batch_checked(ops).unwrap();
        assert_eq!(
            world.db.get("orders", "o1").unwrap().get("note"),
            Some(&serde_json::json!("updated"))
        );
    }

    #[test]
    fn broker_two_groups_on_one_subject_each_see_every_message_fan_out() {
        let broker = BrokerLog::new();
        broker.publish("orders.created", b"1".to_vec());
        broker.publish("orders.created", b"2".to_vec());
        let a = broker.drain("orders.created", "billing");
        let b = broker.drain("orders.created", "shipping");
        assert_eq!(a, vec![b"1".to_vec(), b"2".to_vec()]);
        assert_eq!(
            b,
            vec![b"1".to_vec(), b"2".to_vec()],
            "a second group's independent cursor sees the same log from its own start"
        );
    }

    #[test]
    fn broker_nack_is_a_no_op_and_the_message_is_redelivered() {
        let broker = BrokerLog::new();
        broker.publish("orders.created", b"1".to_vec());
        assert_eq!(
            broker.take_next("orders.created", "billing"),
            Some(b"1".to_vec())
        );
        broker.nack("orders.created", "billing");
        assert_eq!(
            broker.take_next("orders.created", "billing"),
            Some(b"1".to_vec()),
            "nack left the cursor exactly where it was"
        );
        broker.ack("orders.created", "billing");
        assert_eq!(broker.take_next("orders.created", "billing"), None);
    }

    #[test]
    fn broker_pending_count_and_queues_empty_track_the_cursor() {
        let broker = BrokerLog::new();
        assert!(broker.queues_empty());
        broker.publish("orders.created", b"1".to_vec());
        broker.publish("orders.created", b"2".to_vec());
        assert_eq!(broker.pending_count("orders.created", "billing"), 2);
        broker.ack("orders.created", "billing");
        assert_eq!(broker.pending_count("orders.created", "billing"), 1);
        assert!(!broker.queues_empty());
        broker.ack("orders.created", "billing");
        assert_eq!(broker.pending_count("orders.created", "billing"), 0);
        assert!(broker.queues_empty());
    }

    #[test]
    fn publish_checked_writes_into_both_queue_and_broker() {
        let world = SimWorld::new(Vec::new());
        world
            .publish_checked("orders.created", b"1".to_vec())
            .unwrap();
        assert!(!world.queue.is_empty());
        assert_eq!(world.broker.pending_count("orders.created", "any-group"), 1);
    }

    #[test]
    fn clock_and_entropy_are_wired_through_the_world() {
        let world = SimWorld::new(Vec::new());
        assert_eq!(world.now_ms(), 0);
        world.advance_clock_to(5_000);
        assert_eq!(world.now_ms(), 5_000);
        let a = world.next_uuid();
        let b = world.next_uuid();
        assert_ne!(a, b, "the seeded stream advances on every call");
        assert_eq!(a.len(), 36);
    }

    #[test]
    fn cache_get_is_none_for_a_key_never_set() {
        let world = SimWorld::new(Vec::new());
        assert_eq!(world.cache_get("sessions", "u1"), None);
    }

    #[test]
    fn cache_set_then_get_round_trips_without_a_ttl() {
        let world = SimWorld::new(Vec::new());
        world.cache_set("sessions", "u1", "alice".into(), None);
        assert_eq!(world.cache_get("sessions", "u1"), Some("alice".to_owned()));
    }

    #[test]
    fn cache_ttl_expires_across_a_clock_advance_not_before() {
        let world = SimWorld::new(Vec::new());
        world.cache_set("sessions", "u1", "alice".into(), Some(30_000));
        assert_eq!(
            world.cache_get("sessions", "u1"),
            Some("alice".to_owned()),
            "not yet expired"
        );
        world.advance_clock_to(29_999);
        assert_eq!(
            world.cache_get("sessions", "u1"),
            Some("alice".to_owned()),
            "one millisecond before the boundary"
        );
        world.advance_clock_to(30_000);
        assert_eq!(
            world.cache_get("sessions", "u1"),
            None,
            "exactly at the boundary counts as expired (>=, matching Python's FakeCache)"
        );
    }

    #[test]
    fn cache_delete_removes_the_value_and_its_ttl() {
        let world = SimWorld::new(Vec::new());
        world.cache_set("sessions", "u1", "alice".into(), Some(1_000));
        world.cache_delete("sessions", "u1");
        assert_eq!(world.cache_get("sessions", "u1"), None);
    }

    #[test]
    fn cache_instances_are_independent() {
        let world = SimWorld::new(Vec::new());
        world.cache_set("sessions", "k", "a".into(), None);
        world.cache_set("ratelimits", "k", "b".into(), None);
        assert_eq!(world.cache_get("sessions", "k"), Some("a".to_owned()));
        assert_eq!(world.cache_get("ratelimits", "k"), Some("b".to_owned()));
    }

    #[test]
    fn object_store_put_get_delete_list_round_trip() {
        let world = SimWorld::new(Vec::new());
        assert!(world.object_get("uploads", "a.png").is_err());
        assert!(!world.object_exists("uploads", "a.png"));
        world.object_put("uploads", "a.png", b"first".to_vec());
        world.object_put("uploads", "b.png", b"second".to_vec());
        world.object_put("uploads", "reports/c.csv", b"third".to_vec());
        assert!(world.object_exists("uploads", "a.png"));
        assert_eq!(world.object_get("uploads", "a.png").unwrap(), b"first");
        assert_eq!(world.object_list("uploads", "a"), vec!["a.png".to_owned()]);
        assert_eq!(
            world.object_list("uploads", ""),
            vec![
                "a.png".to_owned(),
                "b.png".to_owned(),
                "reports/c.csv".to_owned()
            ],
            "list is sorted"
        );
        world.object_delete("uploads", "a.png");
        assert!(!world.object_exists("uploads", "a.png"));
    }

    #[test]
    fn email_sent_count_filters_by_to_and_subject_substring() {
        let world = SimWorld::new(Vec::new());
        world.email_send("smtp", "ops@example.com", "order reconciled", "body 1");
        world.email_send("smtp", "ops@example.com", "order failed", "body 2");
        world.email_send("smtp", "other@example.com", "order reconciled", "body 3");
        assert_eq!(world.email_sent_count("smtp", None, None), 3);
        assert_eq!(
            world.email_sent_count("smtp", Some("ops@example.com"), None),
            2
        );
        assert_eq!(world.email_sent_count("smtp", None, Some("reconciled")), 2);
        assert_eq!(
            world.email_sent_count("smtp", Some("ops@example.com"), Some("reconciled")),
            1
        );
        assert_eq!(world.email_sent_count("unused-instance", None, None), 0);
    }

    #[test]
    fn search_query_is_a_case_insensitive_substring_match_over_the_document() {
        let world = SimWorld::new(Vec::new());
        world.search_index(
            "catalog",
            "1",
            serde_json::json!({"name": "Blue Widget", "sku": "W-1"}),
        );
        world.search_index(
            "catalog",
            "2",
            serde_json::json!({"name": "Red Gadget", "sku": "G-1"}),
        );
        assert_eq!(world.search_query("catalog", "widget").len(), 1);
        assert_eq!(
            world.search_query("catalog", "WIDGET").len(),
            1,
            "case-insensitive"
        );
        assert_eq!(
            world.search_query("catalog", "").len(),
            2,
            "empty query matches everything"
        );
        assert_eq!(world.search_query("catalog", "nonexistent").len(), 0);
        world.search_delete("catalog", "1");
        assert_eq!(world.search_query("catalog", "widget").len(), 0);
    }

    #[test]
    fn http_post_consumes_fixtures_in_order_and_errors_when_exhausted() {
        let world = SimWorld::new(Vec::new());
        world.seed_http_fixtures(
            "payments",
            vec![
                crate::scenario::GivenHttpResponse::Ok {
                    status: 200,
                    json: serde_json::json!({"accepted": true}),
                },
                crate::scenario::GivenHttpResponse::Error {
                    error: "timeout".into(),
                },
            ],
        );
        let first = world
            .http_post(
                "payments",
                "https://payments.example/charge",
                serde_json::json!({}),
            )
            .unwrap();
        assert_eq!(first, serde_json::json!({"accepted": true}));
        let second = world.http_post(
            "payments",
            "https://payments.example/charge",
            serde_json::json!({}),
        );
        assert!(
            second.is_err(),
            "the fixture's own declared error propagates"
        );
        let third = world.http_post(
            "payments",
            "https://payments.example/charge",
            serde_json::json!({}),
        );
        assert!(
            third.is_err(),
            "exhausted fixtures refuse rather than guess"
        );
        assert_eq!(world.http_request_count("payments"), 3);
    }

    #[test]
    fn auth_verify_refuses_an_unconfigured_token() {
        let world = SimWorld::new(Vec::new());
        let err = world.auth_verify("no-such-token").unwrap_err();
        assert!(err.to_string().contains("no configured claims"));
    }

    #[test]
    fn auth_verify_grants_a_configured_token_and_denies_after_expiry() {
        let world = SimWorld::new(Vec::new());
        world.auth_issue(
            "tok-1",
            serde_json::json!({"sub": "u1", "scopes": ["orders:write"]}),
            Some(60_000),
        );
        let claims = world.auth_verify("tok-1").unwrap();
        assert_eq!(claims["sub"], "u1");
        world.advance_clock_to(59_999);
        assert!(world.auth_verify("tok-1").is_ok(), "not yet expired");
        world.advance_clock_to(60_000);
        let err = world.auth_verify("tok-1").unwrap_err();
        assert!(err.to_string().contains("expired"));
    }

    #[test]
    fn auth_verify_grants_a_token_with_no_expiry_forever() {
        let world = SimWorld::new(Vec::new());
        world.auth_issue("tok-1", serde_json::json!({"sub": "u1"}), None);
        world.advance_clock_to(1_000_000_000);
        assert!(world.auth_verify("tok-1").is_ok());
    }

    // 28UpdatePlan.md M2: (service, table) namespacing.

    #[test]
    fn namespaced_table_key_is_bare_for_the_single_service_degenerate_path() {
        assert_eq!(SimWorld::namespaced_table_key(None, "orders"), "orders");
        assert_eq!(
            SimWorld::namespaced_table_key(Some("Billing"), "orders"),
            "Billing::orders"
        );
    }

    #[test]
    fn two_services_naming_a_table_identically_do_not_collide() {
        // Two schemas, each with its own table literally named "orders"
        // -- exactly the case `ciac-sema`'s `DuplicateDeclaration` check
        // rules out for a real compiled program, but `ciac-sim` itself
        // must still keep separate since it is usable standalone.
        let world = SimWorld::with_schema(
            Vec::new(),
            vec![
                WorldTable {
                    name: "Billing::orders".into(),
                    references: vec![],
                },
                WorldTable {
                    name: "Shipping::orders".into(),
                    references: vec![],
                },
            ],
        );
        world
            .db_insert_checked_for(Some("Billing"), "orders", serde_json::json!({"id": "b1"}))
            .unwrap();
        world
            .db_insert_checked_for(Some("Shipping"), "orders", serde_json::json!({"id": "s1"}))
            .unwrap();
        assert_eq!(world.db.count("Billing::orders"), 1);
        assert_eq!(world.db.count("Shipping::orders"), 1);
        assert_eq!(
            world
                .db
                .get("Billing::orders", "b1")
                .unwrap()
                .get("id")
                .unwrap(),
            "b1"
        );
        // The un-suffixed path stays bare-keyed and untouched by either
        // service-addressed insert above -- the proof the degenerate
        // path really is unaffected.
        assert_eq!(world.db.count("orders"), 0);
    }

    #[test]
    fn service_addressed_update_and_delete_use_the_same_namespaced_key() {
        let world = SimWorld::with_schema(
            Vec::new(),
            vec![WorldTable {
                name: "Billing::orders".into(),
                references: vec![],
            }],
        );
        world
            .db_insert_checked_for(
                Some("Billing"),
                "orders",
                serde_json::json!({"id": "b1", "total": 1}),
            )
            .unwrap();
        let updated = world
            .db_update_checked_for(
                Some("Billing"),
                "orders",
                "b1",
                serde_json::json!({"id": "b1", "total": 2}),
            )
            .unwrap();
        assert_eq!(updated.unwrap()["total"], 2);
        assert!(world
            .db_delete_checked_for(Some("Billing"), "orders", "b1")
            .unwrap());
        assert_eq!(world.db.count("Billing::orders"), 0);
    }

    // 28UpdatePlan.md M2: the call router.

    #[test]
    fn call_checked_routes_to_the_registered_handler_and_returns_its_response() {
        let world = SimWorld::new(Vec::new());
        world.register_api(
            "Billing",
            "Charge",
            std::sync::Arc::new(|req: Value| Ok(serde_json::json!({"charged": req["amount"]}))),
        );
        let resp = world
            .call_checked(
                "Gateway",
                "Billing",
                "Charge",
                serde_json::json!({"amount": 42}),
            )
            .unwrap();
        assert_eq!(resp["charged"], 42);
    }

    #[test]
    fn call_checked_passes_through_the_handler_s_error_envelope_verbatim() {
        let world = SimWorld::new(Vec::new());
        world.register_api(
            "Billing",
            "Charge",
            std::sync::Arc::new(|_req: Value| anyhow::bail!("card declined")),
        );
        let err = world
            .call_checked("Gateway", "Billing", "Charge", Value::Null)
            .unwrap_err();
        assert!(err.to_string().contains("card declined"), "{err}");
    }

    #[test]
    fn call_checked_refuses_an_unregistered_api_with_a_clear_error() {
        let world = SimWorld::new(Vec::new());
        let err = world
            .call_checked("Gateway", "Billing", "Charge", Value::Null)
            .unwrap_err();
        assert!(err.to_string().contains("Billing.Charge"), "{err}");
    }

    #[test]
    fn call_checked_honors_an_injected_call_request_failure() {
        let world = SimWorld::new(vec![FailureRule {
            at: crate::failure::FailureSelector {
                effect: "call.request".into(),
                subject: None,
                occurrence: None,
                phase: FailurePhase::After,
            },
            action: FailureAction::Error,
        }]);
        world.register_api(
            "Billing",
            "Charge",
            std::sync::Arc::new(|_req: Value| Ok(Value::Null)),
        );
        let err = world
            .call_checked("Gateway", "Billing", "Charge", Value::Null)
            .unwrap_err();
        assert!(err.to_string().contains("simulated"), "{err}");
    }

    #[test]
    fn call_checked_depth_guard_refuses_runaway_recursion_instead_of_overflowing_the_stack() {
        // A handler that unconditionally calls itself back through the
        // world -- not a shape a compiled `.ciac` program can produce
        // (`ciac-sema`'s `CycleDetection` refuses the cycle first), but
        // exactly the hand-built-world case the depth guard exists for.
        let world = std::sync::Arc::new(SimWorld::new(Vec::new()));
        let recursive = {
            let world = std::sync::Arc::clone(&world);
            move |req: Value| world.call_checked("Self", "Loop", "Again", req)
        };
        world.register_api("Loop", "Again", std::sync::Arc::new(recursive));
        let err = world
            .call_checked("Gateway", "Loop", "Again", Value::Null)
            .unwrap_err();
        assert!(err.to_string().contains("depth"), "{err}");
    }
}
