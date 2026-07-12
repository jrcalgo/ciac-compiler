//! v0.13 M4: `ciac dev`, exercised as a real watch session against
//! the spawned binary in `--no-docker --poll` mode: initial generate,
//! an edit that breaks the compile (diagnostics appear, the loop
//! survives), and a fix that regenerates with the new declaration.

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ciac-dev-cli-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Waits until the collected stderr contains `needle` at least
/// `count` times.
fn wait_for(log: &Arc<Mutex<String>>, needle: &str, count: usize, budget: Duration) -> bool {
    let deadline = Instant::now() + budget;
    while Instant::now() < deadline {
        if log.lock().unwrap().matches(needle).count() >= count {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

#[test]
fn dev_survives_compile_errors_and_regenerates_on_fix() {
    let dir = temp_dir("session");
    let source = dir.join("main.ciac");
    let out = dir.join("build");
    std::fs::write(
        &source,
        "service DevProbe;\n\nrecord Ping { id: Uuid; }\n\napi Echo: Ping {\n    method: POST;\n    path: \"/echo\";\n}\npipeline Echo: Return;\n",
    )
    .unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_ciac"))
        .args([
            "dev",
            source.to_str().unwrap(),
            "-t",
            "python",
            "-o",
            out.to_str().unwrap(),
            "--no-docker",
            "--poll",
        ])
        .stderr(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .expect("ciac dev starts");

    let log = Arc::new(Mutex::new(String::new()));
    {
        let log = Arc::clone(&log);
        let stderr = child.stderr.take().expect("stderr");
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                let mut log = log.lock().unwrap();
                log.push_str(&line);
                log.push('\n');
            }
        });
    }

    // Initial cycle: generated and — crucially — *watching*: the
    // watches register after the first build, so an edit made before
    // this point would predate the watcher's baseline and never fire.
    assert!(
        wait_for(&log, "regenerated", 1, Duration::from_secs(30)),
        "initial generate: {}",
        log.lock().unwrap()
    );
    assert!(
        wait_for(&log, "watching", 1, Duration::from_secs(10)),
        "watches must register: {}",
        log.lock().unwrap()
    );
    assert!(out.join("app/api/echo.py").is_file());
    // The polling watcher establishes its baseline on the first tick
    // after registration (500ms interval); an edit inside that window
    // would *become* the baseline instead of a change.
    std::thread::sleep(Duration::from_millis(1500));

    // Break the compile: the loop must report and survive.
    std::fs::write(&source, "service DevProbe;\n\npipeline Nope: Work;\n").unwrap();
    assert!(
        wait_for(&log, "compile errors", 1, Duration::from_secs(60)),
        "broken save should surface diagnostics: {}",
        log.lock().unwrap()
    );
    // Same poll-tick edge as above: give the watcher a full tick to
    // settle before the next save, so the fix isn't coalesced into
    // whatever polling window just closed over the broken save.
    std::thread::sleep(Duration::from_millis(600));

    // Fix it with a *new* api: the loop must regenerate the new route.
    std::fs::write(
        &source,
        "service DevProbe;\n\nrecord Ping { id: Uuid; }\n\napi Renamed: Ping {\n    method: POST;\n    path: \"/renamed\";\n}\npipeline Renamed: Return;\n",
    )
    .unwrap();
    assert!(
        wait_for(&log, "regenerated", 2, Duration::from_secs(60)),
        "fixed save should regenerate: {}",
        log.lock().unwrap()
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while !out.join("app/api/renamed.py").is_file() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(
        out.join("app/api/renamed.py").is_file(),
        "the new api's route module must exist after the fix"
    );

    child.kill().expect("stop the session");
    let _ = child.wait();
    std::fs::remove_dir_all(&dir).ok();
}
