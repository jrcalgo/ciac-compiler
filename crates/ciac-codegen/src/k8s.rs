//! v0.8 M6: Kubernetes manifest emission (`ciac build --deploy k8s`).
//!
//! Unlike docker-compose (duplicated per backend today, since a
//! compose file's `build: .` references that backend's own
//! Dockerfile), a k8s manifest only needs an image reference, a port,
//! and an environment variable list — and every env var name
//! (`InstanceCtx::env_var`, `CfgFieldCtx::env`, `CallTargetCtx::env_var`)
//! is already computed once in the language-neutral [`crate::model`],
//! byte-identical between `config.py.j2` and `config.rs.j2`. So this
//! is one shared generator, not one per backend — the same shape
//! `crate::system_tests` already established for this crate.
//!
//! Scoped to what 08UpdatePlan.md's v0.8 M6 literally asks for:
//! a Deployment + Service (+ ConfigMap) per declared service, and one
//! StatefulSet for the message broker. Stateful infra capabilities
//! (db/cache/object_store/email/search) are **not** emitted as k8s
//! resources — they stay externally provisioned, addressed through the
//! same hostnames docker-compose's own dev containers already use, so
//! a deploying team either names their own Services to match or
//! overrides the ConfigMap. See `docs/deployment.md`.

use crate::model::{Ctx, SystemModel};

/// Builds every k8s manifest file for the program, keyed by path
/// under `k8s/`. Empty for a program with no services (never actually
/// happens — every valid program has at least one). `image_prefix`
/// defaults to the program's own project name when `None`.
///
/// `32UpdatePlan.md` M6: takes an already-built [`SystemModel`] rather
/// than a [`NormalizedIr`] and building its own — `ciac/src/
/// commands.rs`'s `generate` builds one `GenOptions::default()`-keyed
/// model and shares it across this, [`crate::terraform::build`] and
/// [`crate::ci::build`], which all three called `build_system` on
/// identical inputs independently before this milestone.
pub fn build(
    system: &SystemModel,
    image_prefix: Option<&str>,
    tag: &str,
    profile: crate::Profile,
    secrets: bool,
) -> Vec<(String, String)> {
    let image_prefix = image_prefix.unwrap_or(&system.project_name);
    let mut files = Vec::new();

    for ctx in &system.services {
        let name = resource_name(system, ctx);
        let image = if system.multi {
            format!("{image_prefix}-{name}:{tag}")
        } else {
            format!("{image_prefix}:{tag}")
        };
        let env = service_env(ctx);
        // `--secrets` (v0.11 M5): secret-shaped values move out of the
        // ConfigMap into a Secret manifest wired via envFrom, so the
        // ConfigMap never carries them verbatim. Placeholder values are
        // documented as override-before-apply, same as before.
        let (secret_env, plain_env): (Vec<_>, Vec<_>) = if secrets {
            env.into_iter().partition(|(key, _)| key == "JWT_SECRET")
        } else {
            (Vec::new(), env)
        };
        files.push((
            format!("k8s/{name}.yaml"),
            render_service_manifest(
                &name,
                &image,
                ctx.host_port,
                &plain_env,
                profile,
                !secret_env.is_empty(),
            ),
        ));
        if !secret_env.is_empty() {
            files.push((
                format!("k8s/{name}-secrets.yaml"),
                render_secret_manifest(&name, &secret_env),
            ));
        }
    }

    if system.has_queue {
        files.push((
            "k8s/queue.yaml".to_string(),
            render_broker_manifest(system.queue_engine.as_deref()),
        ));
    }

    files
}

/// The k8s resource name for a service: its compose-style kebab `dir`
/// name in a multi-service system (matching `CallTargetCtx::kebab`, so
/// cross-service call URLs resolve via k8s's own Service DNS exactly
/// like they resolve via compose's container DNS today), or the
/// project name for a single-deployable program (`dir` is empty then).
fn resource_name(system: &SystemModel, ctx: &Ctx) -> String {
    if ctx.dir.is_empty() {
        system.project_name.clone()
    } else {
        ctx.dir.clone()
    }
}

/// The same environment variables `docker-compose.yml.j2` /
/// `system-compose.yml.j2` already set for this service, so a
/// generated app reads identical configuration under either posture —
/// only the values differ where compose points at a same-namespace
/// container and k8s needs a Service to exist under that name instead.
fn service_env(ctx: &Ctx) -> Vec<(String, String)> {
    let mut env = Vec::new();
    for inst in &ctx.db_instances {
        if inst.db_engine == "sqlite" {
            // Pod-local file (v0.13 M3): no Service to point at. Dev
            // and light production only -- the file lives and dies
            // with the pod unless a PersistentVolume is added by hand.
            env.push((
                inst.env_var.clone(),
                format!("sqlite://data/{}.db?mode=rwc", inst.db_name),
            ));
            continue;
        }
        let scheme = if inst.db_engine == "mysql" {
            "mysql"
        } else {
            "postgres"
        };
        env.push((
            inst.env_var.clone(),
            format!(
                "{scheme}://{user}:{user}@{container}:{port}/{db}",
                user = inst.db_user,
                container = inst.container,
                port = inst.db_container_port,
                db = inst.db_name
            ),
        ));
    }
    for inst in &ctx.cache_instances {
        env.push((
            inst.env_var.clone(),
            format!("redis://{}:6379/0", inst.container),
        ));
    }
    for inst in ctx
        .object_store_instances
        .iter()
        .chain(&ctx.email_instances)
        .chain(&ctx.search_instances)
    {
        for f in &inst.cfg {
            if let Some(value) = &f.compose_value {
                env.push((f.env.clone(), value.clone()));
            }
        }
    }
    for target in &ctx.call_targets {
        env.push((
            target.env_var.clone(),
            format!("http://{}:8000", target.kebab),
        ));
    }
    if ctx.has_queue {
        if ctx.queue_engine.as_deref() == Some("kafka") {
            env.push(("KAFKA_URL".to_owned(), "queue:9092".to_owned()));
        } else {
            env.push(("NATS_URL".to_owned(), "nats://queue:4222".to_owned()));
        }
    }
    if ctx.auth_scheme == "oauth2" {
        let issuer = if ctx.has_users {
            // `users Keycloak` (v0.15 M6) only ever runs in dev compose
            // -- there's no Service in the cluster to resolve
            // `KEYCLOAK_DEV_ISSUER` against, so the ConfigMap ships an
            // explicit placeholder instead of a dev URL that would
            // silently fail to resolve against a real deployment.
            "https://REPLACE-ME.example.com/realms/prod".to_owned()
        } else {
            ctx.auth_issuer.clone()
        };
        env.push(("OAUTH_ISSUER".to_owned(), issuer));
        if !ctx.auth_audience.is_empty() {
            env.push(("OAUTH_AUDIENCE".to_owned(), ctx.auth_audience.clone()));
        }
    } else if ctx.has_auth {
        // ConfigMap only (v0.8 M6 scope) — a real deployment should
        // override this via a Secret, not ship it here verbatim.
        env.push((
            "JWT_SECRET".to_owned(),
            "change-me-override-with-a-real-secret".to_owned(),
        ));
    }
    env
}

fn render_secret_manifest(name: &str, env: &[(String, String)]) -> String {
    let mut out = String::from("apiVersion: v1\nkind: Secret\nmetadata:\n");
    out.push_str(&format!("  name: {name}-secrets\n"));
    out.push_str("type: Opaque\nstringData:\n");
    for (key, value) in env {
        out.push_str(&format!("  {key}: {value:?}\n"));
    }
    out
}

fn render_service_manifest(
    name: &str,
    image: &str,
    port: u16,
    env: &[(String, String)],
    profile: crate::Profile,
    with_secret_ref: bool,
) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: {name}-config\ndata:\n"
    ));
    for (key, value) in env {
        out.push_str(&format!("  {key}: {value:?}\n"));
    }
    if env.is_empty() {
        out.push_str("  {}\n");
    }

    out.push_str(&format!(
        "---\n\
         apiVersion: apps/v1\n\
         kind: Deployment\n\
         metadata:\n\
         \x20 name: {name}\n\
         spec:\n\
         \x20 replicas: {replicas}\n\
         \x20 selector:\n\
         \x20   matchLabels:\n\
         \x20     app: {name}\n\
         \x20 template:\n\
         \x20   metadata:\n\
         \x20     labels:\n\
         \x20       app: {name}\n\
         \x20   spec:\n\
         \x20     containers:\n\
         \x20       - name: {name}\n\
         \x20         image: {image}\n\
         \x20         ports:\n\
         \x20           - containerPort: 8000\n\
         \x20         envFrom:\n\
         \x20           - configMapRef:\n\
         \x20               name: {name}-config\n\
         {secret_ref}\
         \x20         readinessProbe:\n\
         \x20           httpGet:\n\
         \x20             path: /health\n\
         \x20             port: 8000\n\
         \x20           initialDelaySeconds: 2\n\
         \x20           periodSeconds: 5\n\
         \x20         livenessProbe:\n\
         \x20           httpGet:\n\
         \x20             path: /health\n\
         \x20             port: 8000\n\
         \x20           initialDelaySeconds: 5\n\
         \x20           periodSeconds: 10\n\
         {resources}",
        replicas = profile.replicas(),
        secret_ref = if with_secret_ref {
            "\x20           - secretRef:\n\x20               name: {name}-secrets\n"
                .replace("{name}", name)
        } else {
            String::new()
        },
        resources = if profile.is_dev() {
            String::new()
        } else {
            "\x20         resources:\n\
             \x20           requests:\n\
             \x20             cpu: 250m\n\
             \x20             memory: 256Mi\n\
             \x20           limits:\n\
             \x20             cpu: \"1\"\n\
             \x20             memory: 512Mi\n"
                .to_owned()
        },
    ));

    out.push_str(&format!(
        "---\n\
         apiVersion: v1\n\
         kind: Service\n\
         metadata:\n\
         \x20 name: {name}\n\
         spec:\n\
         \x20 selector:\n\
         \x20   app: {name}\n\
         \x20 ports:\n\
         \x20   - port: {port}\n\
         \x20     targetPort: 8000\n"
    ));

    out
}

/// One StatefulSet + headless Service for the NATS broker, named
/// `queue` to match the hostname `docker-compose.yml.j2` already wires
/// every `NATS_URL` to (`nats://queue:4222`) — no per-service env
/// difference between compose and k8s for this one.
fn render_broker_manifest(queue_engine: Option<&str>) -> String {
    let (image, port) = if queue_engine == Some("kafka") {
        ("apache/kafka:3.8.0", 9092)
    } else {
        ("nats:2", 4222)
    };
    format!(
        "apiVersion: apps/v1\n\
     kind: StatefulSet\n\
     metadata:\n\
     \x20 name: queue\n\
     spec:\n\
     \x20 serviceName: queue\n\
     \x20 replicas: 1\n\
     \x20 selector:\n\
     \x20   matchLabels:\n\
     \x20     app: queue\n\
     \x20 template:\n\
     \x20   metadata:\n\
     \x20     labels:\n\
     \x20       app: queue\n\
     \x20   spec:\n\
     \x20     containers:\n\
     \x20       - name: queue\n\
     \x20         image: {image}\n\
     \x20         ports:\n\
     \x20           - containerPort: {port}\n\
     ---\n\
     apiVersion: v1\n\
     kind: Service\n\
     metadata:\n\
     \x20 name: queue\n\
     spec:\n\
     \x20 clusterIP: None\n\
     \x20 selector:\n\
     \x20   app: queue\n\
     \x20 ports:\n\
     \x20   - port: {port}\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::build_system;
    use crate::GenOptions;
    use ciac_ir::NormalizedIr;

    fn compile(src: &str) -> NormalizedIr {
        let mut sources = ciac_diagnostics::SourceMap::new();
        let file = sources.add_file("test.ciac", src);
        let mut diags = ciac_diagnostics::Diagnostics::new();
        let program = ciac_syntax::parse(src, file, &mut diags);
        ciac_sema::analyze(&program, &mut diags)
            .unwrap_or_else(|| panic!("compiles: {:?}", diags.codes()))
    }

    #[test]
    fn single_service_no_queue_has_no_broker() {
        let ir = compile(
            "service Notes;\nuse { db Postgres; }\nrecord Note { id: Uuid; }\ncrud Note;\n",
        );
        let system = build_system(&ir, &GenOptions::default());
        let files = build(&system, Some("notes"), "latest", crate::Profile::Dev, false);
        let paths: Vec<&str> = files.iter().map(|(p, _)| p.as_str()).collect();
        assert!(paths.contains(&"k8s/notes.yaml"));
        assert!(!paths.iter().any(|p| p.contains("queue")));
        let (_, content) = files.iter().find(|(p, _)| p == "k8s/notes.yaml").unwrap();
        assert!(content.contains("image: notes:latest"));
        assert!(content.contains("path: /health"));
        assert!(content.contains("DATABASE_URL"));
    }

    #[test]
    fn multi_service_with_queue_gets_broker_and_per_service_images() {
        let src = r#"
project MediaSystem;
record Video { id: Uuid; title: String; }
stream Uploaded: Video;

service Billing {
    api Charge: Video { method: POST; path: "/charge"; }
    pipeline Charge: CapturePayment -> Return;
}

service UploadApi {
    use { queue bus NATS; }
    api Upload: Video { method: PUT; path: "/videos"; }
    pipeline Upload: call Billing.Charge -> StoreVideo -> publish Uploaded -> Return;
}

service Transcoder {
    use { queue bus NATS; }
    worker Transcode on Uploaded;
    pipeline Transcode: TranscodeVideo;
}
"#;
        let ir = compile(src);
        let system = build_system(&ir, &GenOptions::default());
        let files = build(&system, Some("media"), "v1", crate::Profile::Dev, false);
        let paths: Vec<&str> = files.iter().map(|(p, _)| p.as_str()).collect();
        assert!(paths.contains(&"k8s/billing.yaml"));
        assert!(paths.contains(&"k8s/upload-api.yaml"));
        assert!(paths.contains(&"k8s/transcoder.yaml"));
        assert!(paths.contains(&"k8s/queue.yaml"));

        let (_, upload) = files
            .iter()
            .find(|(p, _)| p == "k8s/upload-api.yaml")
            .unwrap();
        assert!(upload.contains("image: media-upload-api:v1"));
        assert!(upload.contains("http://billing:8000"));
        assert!(upload.contains("nats://queue:4222"));

        let (_, queue) = files.iter().find(|(p, _)| p == "k8s/queue.yaml").unwrap();
        assert!(queue.contains("kind: StatefulSet"));
        assert!(queue.contains("clusterIP: None"));
    }
}
