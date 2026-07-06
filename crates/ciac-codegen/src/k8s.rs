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

use crate::model::{build_system, Ctx, SystemModel};
use crate::GenOptions;
use ciac_ir::NormalizedIr;

/// Builds every k8s manifest file for the program, keyed by path
/// under `k8s/`. Empty for a program with no services (never actually
/// happens — every valid program has at least one). `image_prefix`
/// defaults to the program's own project name when `None`.
pub fn build(ir: &NormalizedIr, image_prefix: Option<&str>, tag: &str) -> Vec<(String, String)> {
    let system = build_system(ir, &GenOptions::default());
    let image_prefix = image_prefix.unwrap_or(&system.project_name);
    let mut files = Vec::new();

    for ctx in &system.services {
        let name = resource_name(&system, ctx);
        let image = if system.multi {
            format!("{image_prefix}-{name}:{tag}")
        } else {
            format!("{image_prefix}:{tag}")
        };
        let env = service_env(ctx);
        files.push((
            format!("k8s/{name}.yaml"),
            render_service_manifest(&name, &image, ctx.host_port, &env),
        ));
    }

    if system.has_queue {
        files.push(("k8s/queue.yaml".to_string(), render_broker_manifest()));
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
        env.push((
            inst.env_var.clone(),
            format!(
                "postgres://postgres:postgres@{}:5432/{}",
                inst.container, inst.db_name
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
        env.push(("NATS_URL".to_owned(), "nats://queue:4222".to_owned()));
    }
    if ctx.has_auth {
        // ConfigMap only (v0.8 M6 scope) — a real deployment should
        // override this via a Secret, not ship it here verbatim.
        env.push((
            "JWT_SECRET".to_owned(),
            "change-me-override-with-a-real-secret".to_owned(),
        ));
    }
    env
}

fn render_service_manifest(name: &str, image: &str, port: u16, env: &[(String, String)]) -> String {
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
         \x20 replicas: 1\n\
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
         \x20           periodSeconds: 10\n"
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
fn render_broker_manifest() -> String {
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
     \x20         image: nats:2\n\
     \x20         ports:\n\
     \x20           - containerPort: 4222\n\
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
     \x20   - port: 4222\n"
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let files = build(&ir, Some("notes"), "latest");
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
        let files = build(&ir, Some("media"), "v1");
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
