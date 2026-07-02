use anyhow::{bail, Context, Result};
use ciac_codegen::{Backend, GenOptions};
use ciac_diagnostics::render::{AriadneRenderer, Render};
use ciac_diagnostics::{Diagnostics, ErrorCode, SourceMap};
use ciac_ir::NormalizedIr;
use std::io::IsTerminal;
use std::path::Path;
use std::process::ExitCode;

/// All registered code-generation backends, in stable order.
/// Adding a target is one line here plus the backend crate itself.
fn backends() -> Vec<Box<dyn Backend>> {
    vec![
        Box::new(ciac_backend_python::PythonBackend),
        Box::new(ciac_backend_rust::RustBackend),
    ]
}

/// Runs the front-end (parse + analyze) on a source file, printing all
/// diagnostics. Returns the IR when the program is valid.
fn front_end(file: &Path) -> Result<(Option<NormalizedIr>, bool)> {
    let src =
        std::fs::read_to_string(file).with_context(|| format!("cannot read {}", file.display()))?;
    let mut sources = SourceMap::new();
    let file_id = sources.add_file(file.display().to_string(), src.clone());
    let mut diags = Diagnostics::new();
    let program = ciac_syntax::parse(&src, file_id, &mut diags);
    let ir = ciac_sema::analyze(&program, &mut diags);

    diags.sort();
    let has_errors = diags.has_errors();
    let renderer = AriadneRenderer {
        color: std::io::stderr().is_terminal(),
    };
    let mut stderr = std::io::stderr().lock();
    for diag in diags.iter() {
        renderer.render(diag, &sources, &mut stderr)?;
    }
    Ok((ir, has_errors))
}

pub fn check(file: &Path) -> Result<ExitCode> {
    let (ir, has_errors) = front_end(file)?;
    if has_errors || ir.is_none() {
        return Ok(ExitCode::FAILURE);
    }
    eprintln!("{}: no errors", file.display());
    Ok(ExitCode::SUCCESS)
}

pub fn build(
    file: &Path,
    target: &str,
    out: &Path,
    force: bool,
    name: Option<String>,
) -> Result<ExitCode> {
    let all = backends();
    let Some(backend) = all.iter().find(|b| b.id() == target) else {
        let known: Vec<&str> = all.iter().map(|b| b.id()).collect();
        bail!("unknown target `{target}`; available: {}", known.join(", "));
    };

    let (ir, has_errors) = front_end(file)?;
    let Some(ir) = ir.filter(|_| !has_errors) else {
        return Ok(ExitCode::FAILURE);
    };

    if let Err(err) = ciac_codegen::check_support(backend.as_ref(), &ir) {
        // Unsupported constructs are user-facing diagnostics, not crashes.
        eprintln!("error[{}]: {err}", ErrorCode::UnsupportedConstruct);
        return Ok(ExitCode::FAILURE);
    }

    if out.exists()
        && out
            .read_dir()
            .map(|mut d| d.next().is_some())
            .unwrap_or(false)
        && !force
    {
        bail!(
            "output directory {} is not empty (pass --force to write anyway)",
            out.display()
        );
    }

    let opts = GenOptions { project_name: name };
    let project = backend.generate(&ir, &opts)?;
    project
        .write_to(out)
        .with_context(|| format!("cannot write to {}", out.display()))?;

    eprintln!(
        "generated {} files in {} ({} backend)",
        project.len(),
        out.display(),
        backend.id()
    );
    for note in &project.notes {
        eprintln!("note: {note}");
    }
    Ok(ExitCode::SUCCESS)
}

pub fn graph(file: &Path, format: &str) -> Result<ExitCode> {
    let (ir, has_errors) = front_end(file)?;
    let Some(ir) = ir.filter(|_| !has_errors) else {
        return Ok(ExitCode::FAILURE);
    };
    match format {
        "json" => println!("{}", serde_json::to_string_pretty(&ir)?),
        "dot" => print!("{}", ir.to_dot()),
        other => bail!("unknown format `{other}`"),
    }
    Ok(ExitCode::SUCCESS)
}

pub fn explain(code: &str) -> Result<ExitCode> {
    let Some(code) = ErrorCode::parse(code) else {
        bail!("unknown error code `{code}`; codes look like CIAC0001 (see docs/errors.md)");
    };
    println!("{}: {}", code.code(), code.title());
    println!();
    println!("{}", code.explanation());
    Ok(ExitCode::SUCCESS)
}

pub fn targets() -> Result<ExitCode> {
    for backend in backends() {
        println!("{:10} {}", backend.id(), backend.description());
    }
    Ok(ExitCode::SUCCESS)
}
