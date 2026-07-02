//! Shared minijinja setup for backends.
//!
//! Backends embed their templates with `include_dir!` and load them into a
//! [`minijinja::Environment`] via [`environment`], which also installs the
//! identifier-casing filters every target needs (`snake_case`,
//! `pascal_case`, `kebab_case`, `shouty_snake_case`).

use heck::{ToKebabCase, ToPascalCase, ToShoutySnakeCase, ToSnakeCase};
use minijinja::Environment;

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
    for (name, source) in templates {
        env.add_template(name, source)?;
    }
    Ok(env)
}

#[cfg(test)]
mod tests {
    use super::*;

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
