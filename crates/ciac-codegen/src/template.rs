//! Shared minijinja setup for backends.
//!
//! Backends embed their templates with `include_dir!` and load them into a
//! [`minijinja::Environment`] via [`environment`], which also installs the
//! identifier-casing filters every target needs (`snake_case`,
//! `pascal_case`, `kebab_case`, `shouty_snake_case`).

use heck::{ToKebabCase, ToPascalCase, ToShoutySnakeCase, ToSnakeCase};
use minijinja::Environment;
use std::sync::OnceLock;

/// Builds a strict environment from `(name, source)` template pairs.
///
/// The environment is configured to fail on undefined variables rather
/// than silently emitting empty strings — template bugs should fail
/// generation, not corrupt output.
pub fn environment<'a>(
    templates: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> Result<Environment<'a>, minijinja::Error> {
    let mut env = Environment::new();
    env.set_undefined_behavior(minijinja::UndefinedBehavior::Strict);
    env.set_keep_trailing_newline(true);
    env.add_filter("snake_case", |s: &str| s.to_snake_case());
    env.add_filter("pascal_case", |s: &str| s.to_pascal_case());
    env.add_filter("kebab_case", |s: &str| s.to_kebab_case());
    env.add_filter("shouty_snake_case", |s: &str| s.to_shouty_snake_case());
    // v0.13 M1: SQL placeholder style per db engine. Generated SQL is
    // written with Postgres-style `$N` numbered in bind order; engines
    // with purely positional placeholders (MySQL, SQLite) substitute
    // each `$N` with `?` — order-preserving by construction.
    env.add_filter("sqlph", |sql: &str, engine: &str| sqlph(sql, engine));
    env.add_function("ph", |engine: &str, n: u32| {
        if question_placeholders(engine) {
            "?".to_string()
        } else {
            format!("${n}")
        }
    });
    for (name, source) in templates {
        env.add_template(name, source)?;
    }
    Ok(env)
}

/// Fetches (or builds once and caches) a backend's full template
/// environment (`30UpdatePlan.md` M4).
///
/// Every `generate()` call previously rebuilt a fresh [`Environment`]
/// from scratch — re-parsing every one of a backend's embedded
/// templates on every call, including the 290 redundant calls
/// `determinism.rs`'s own deliberate double-generate makes across the
/// full example corpus. Each backend owns its own `static
/// OnceLock<Environment<'static>>` — **never shared across backends**,
/// since `add_filters` registers backend-specific filters — and passes
/// it here; the environment is built once, on the first `generate()`
/// call for that backend within a process, and every later call reuses
/// the same `&'static Environment`. Building only ever happens once
/// per process regardless of how many threads call `generate()`
/// concurrently (the test suite does exactly that): [`OnceLock::
/// get_or_init`] guarantees the initializer runs at most once, with
/// every other concurrent caller blocking until it completes rather
/// than racing to build a second copy. **This caches parsed templates
/// only, never rendered output** — every `generate()` call still walks
/// the model and renders every template fresh, so this is a startup-
/// cost optimization, not a correctness-risking cache: `determinism.
/// rs`'s own double-`generate()` proof stays meaningful, since nothing
/// about a render's *result* is memoized here, only the machinery that
/// parses `{{ ... }}` once instead of once per call.
pub fn cached_environment(
    cache: &'static OnceLock<Environment<'static>>,
    templates: impl IntoIterator<Item = (&'static str, &'static str)>,
    add_filters: impl FnOnce(&mut Environment<'static>),
) -> &'static Environment<'static> {
    cache.get_or_init(|| {
        let mut env = environment(templates).expect(
            "backend's own embedded templates are valid minijinja sources, \
             already exercised by every golden/conformance test",
        );
        add_filters(&mut env);
        env
    })
}

/// Whether `engine` binds with positional `?` placeholders instead of
/// Postgres-style `$N`.
pub fn question_placeholders(engine: &str) -> bool {
    matches!(engine, "mysql" | "sqlite")
}

/// Rewrites every `$<digits>` placeholder to `?` for
/// [`question_placeholders`] engines; other engines keep the SQL
/// untouched. Only safe because generated SQL numbers placeholders in
/// bind order (see `RecordCtx`'s field docs).
pub fn sqlph(sql: &str, engine: &str) -> String {
    if !question_placeholders(engine) {
        return sql.to_string();
    }
    let mut out = String::with_capacity(sql.len());
    let mut chars = sql.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '$' && chars.peek().is_some_and(|n| n.is_ascii_digit()) {
            out.push('?');
            while chars.peek().is_some_and(|n| n.is_ascii_digit()) {
                chars.next();
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sqlph_rewrites_only_for_question_engines() {
        let sql = "UPDATE t SET a = $1, b = $12 WHERE id = $13";
        assert_eq!(sqlph(sql, "postgres"), sql);
        assert_eq!(
            sqlph(sql, "mysql"),
            "UPDATE t SET a = ?, b = ? WHERE id = ?"
        );
        assert_eq!(
            sqlph("no placeholders, $ alone", "sqlite"),
            "no placeholders, $ alone"
        );
    }

    #[test]
    fn casing_filters_work() {
        let env = environment([(
            "t",
            "{{ name | snake_case }} {{ name | pascal_case }} {{ name | kebab_case }}",
        )])
        .expect("valid template");
        let out = env
            .get_template("t")
            .expect("registered")
            .render(minijinja::context! { name => "StoreVideo" })
            .expect("renders");
        assert_eq!(out, "store_video StoreVideo store-video");
    }

    #[test]
    fn undefined_variables_fail() {
        let env = environment([("t", "{{ missing }}")]).expect("valid template");
        assert!(env
            .get_template("t")
            .expect("registered")
            .render(())
            .is_err());
    }
}
