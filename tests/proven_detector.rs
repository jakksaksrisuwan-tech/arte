//! v-proven-detector-is-derived-from-run-history-and-unproven-green
//! c-arte-reports-which-validations-have-never-been-observed-red-pr
//!
//! A validation that has NEVER been observed red is indistinguishable from a
//! validation that cannot fail. Measured on the subject repo: of 83 greens, 34
//! had never failed; the first one fault-injected turned out to be fake (it
//! read cross-shop rows through the service-role endpoint, so RLS was never
//! under test). arte therefore derives PROVEN-ness from run history and
//! surfaces the unproven ones, so the board can't quietly bank untested greens.
//!
//! FALSIFYING: a validation whose runs are all passes must NOT read as proven;
//! one with any recorded fail must; and `arte gate` must name the unproven set.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use arte::proven_detector;

const BIN: &str = env!("CARGO_BIN_EXE_arte");

fn scratch(label: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("arte-proven-{label}-{}-{nanos}", std::process::id()));
    fs::create_dir_all(dir.join("truth")).expect("truth dir");
    fs::create_dir_all(dir.join("runs")).expect("runs dir");
    fs::create_dir_all(dir.join("tests")).expect("tests dir");
    fs::write(dir.join("arte.toml"), "chain = [\"intent\", \"impl\", \"control\", \"validation\"]\n\n[lanes]\nproof = \"true\"\n").expect("toml");
    dir
}

fn seed_validation(dir: &PathBuf, id: &str, results: &[&str]) {
    fs::write(dir.join("tests").join(format!("{id}.rs")), "// scratch\n").expect("test file");
    fs::write(
        dir.join("truth").join(format!("{id}.node")),
        format!("id: {id}\nrole: validation\nsubset: proof\ntitle: {id}\nat: tests/{id}.rs\nstatus: ok\n"),
    )
    .expect("node");
    for (i, r) in results.iter().enumerate() {
        fs::write(
            dir.join("runs").join(format!("{id}.{:03}.run", i + 1)),
            format!("validation: {id}\nresult: {r}\n"),
        )
        .expect("run record");
    }
}

/// Run a subcommand with the scratch board's dirs bound.
fn run(dir: &PathBuf, args: &[&str]) -> (String, bool) {
    let out = Command::new(BIN)
        .args(args)
        .current_dir(dir)
        .env("ARTE_TRUTH_DIR", dir.join("truth"))
        .env("ARTE_RUNS_DIR", dir.join("runs"))
        .output()
        .expect("spawn arte");
    (
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
        out.status.success(),
    )
}

#[test]
fn proven_detector_derivation_and_stickiness() {
    provenness_survives_run_history_pruning();
    let dir = scratch("derive");
    seed_validation(&dir, "v-never-red", &["pass", "pass", "pass"]);
    seed_validation(&dir, "v-once-red", &["fail", "pass"]);
    // proven_detector reads the runs dir, so bind it for this process
    std::env::set_var("ARTE_RUNS_DIR", dir.join("runs"));
    assert!(
        !proven_detector("v-never-red"),
        "a validation whose every run passed must NOT read as a proven detector — \
         that is exactly the never-observed-red blind spot"
    );
    assert!(
        proven_detector("v-once-red"),
        "a validation with a recorded fail IS a proven detector (it has demonstrated it can go red)"
    );
    assert!(
        !proven_detector("v-no-history-at-all"),
        "no run history means unproven, never proven-by-default"
    );
    let _ = fs::remove_dir_all(&dir);
}

// NOTE: the in-process assertions live in ONE test on purpose — cargo runs
// tests as parallel threads in a single process, and ARTE_TRUTH_DIR /
// ARTE_RUNS_DIR are process-global, so splitting them raced (measured: the
// history-derivation asserts failed because a sibling test repointed the dirs).
fn provenness_survives_run_history_pruning() {
    // The defect this guards: run history is pruned to the stable-pass window,
    // so a recorded red ages out after a few green runs and proven-ness would
    // silently evaporate — measured live, 8 freshly fault-proven validations
    // all read unproven again after their re-derivations. The node stamp is
    // the durable record.
    let dir = scratch("sticky");
    seed_validation(&dir, "v-aged-out", &["pass", "pass", "pass", "pass", "pass"]);
    // its red is gone from history, but the node carries the earned stamp
    let node = dir.join("truth").join("v-aged-out.node");
    let txt = fs::read_to_string(&node).expect("read node");
    fs::write(&node, format!("{txt}proven: observed-red\n")).expect("stamp node");
    std::env::set_var("ARTE_RUNS_DIR", dir.join("runs"));
    std::env::set_var("ARTE_TRUTH_DIR", dir.join("truth"));
    assert!(
        proven_detector("v-aged-out"),
        "proven-ness did not survive history pruning — the stamp on the node must be durable, \
         otherwise every adversary round's evidence evaporates after 5 green runs"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn gate_names_the_unproven_validations() {
    let dir = scratch("gate");
    seed_validation(&dir, "v-never-red", &["pass"]);
    seed_validation(&dir, "v-once-red", &["fail", "pass"]);
    let (out, _ok) = run(&dir, &["gate"]);
    assert!(
        out.contains("v-never-red"),
        "gate must NAME each never-observed-red validation so the board cannot bank untested greens.\n--- gate output ---\n{out}"
    );
    assert!(
        !out.contains("v-once-red") || out.matches("v-once-red").count() < out.matches("v-never-red").count(),
        "gate must not flag a validation that has already been observed red.\n--- gate output ---\n{out}"
    );
    let _ = fs::remove_dir_all(&dir);
}
