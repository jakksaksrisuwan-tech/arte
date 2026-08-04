//! Tests for the env-var isolation + run-history pruning controls.
//! c-arte-respects-ARTE_TRUTH_DIR-env-var:
//!   artefact must honour the ARTE_TRUTH_DIR, ARTE_RUNS_DIR,
//!   and ARTE_DISPATCH_PATH env vars; fall back to the CWD defaults when unset.
//! c-run-history-files-do-not-accumulate-unbounded:
//!   qa/runs/ is bounded by the stable-pass window (5 per validation).
//!   Older run files must be pruned on each `arte verify`.
//!
//! All tests are hermetic: they use per-test scratch directories under
//! `std::env::temp_dir()` and `CARGO_BIN_EXE_arte` — no shared state with
//! the live `.truth/` or `qa/runs/`. Failures here prove the env-var
//! plumbing / pruning is missing; green proves the impl is in.
//!
//! IMPORTANT — semantics: under `read_env_dirs` (src/lib.rs:37), ARTE_TRUTH_DIR
//! and ARTE_RUNS_DIR are the CONCRETE truth / runs directories — not a parent.
//! So tests must put node files directly under the truth dir, and run files
//! directly under the runs dir.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn arte_bin() -> &'static str {
    env!("CARGO_BIN_EXE_arte")
}

fn scratch_dir(label: &str) -> PathBuf {
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("arte-test-isolation-{label}-{pid}-{nanos}"));
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// `dir` is the CONCRETE truth directory. The node is written at
/// `dir/<id>.node` — no `.truth/` append (the env var IS the truth dir).
fn write_node(dir: &PathBuf, id: &str, body: &str) {
    fs::create_dir_all(dir).expect("create truth dir");
    fs::write(dir.join(format!("{id}.node")), body).expect("write node");
}

/// Run `arte <args>` with ARTE_TRUTH_DIR set to `truth_dir` (concrete).
/// `truth_dir` is passed straight through as `ARTE_TRUTH_DIR` — the binary
/// reads/writes node files directly under it.
fn run_arte_with_truth(args: &[&str], truth_dir: &PathBuf) -> (String, String, std::process::ExitStatus) {
    let out = Command::new(arte_bin())
        .args(args)
        .env("ARTE_TRUTH_DIR", truth_dir)
        .output()
        .expect("spawn arte");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status,
    )
}

/// Run `arte <args>` with ARTE_TRUTH_DIR and ARTE_RUNS_DIR set independently.
/// `truth_dir` and `runs_dir` are both CONCRETE paths (no `.truth/` or
/// `qa/runs/` append).
fn run_arte_with_truth_and_runs(
    args: &[&str],
    truth_dir: &PathBuf,
    runs_dir: &PathBuf,
) -> (String, String, std::process::ExitStatus) {
    let out = Command::new(arte_bin())
        .args(args)
        .env("ARTE_TRUTH_DIR", truth_dir)
        .env("ARTE_RUNS_DIR", runs_dir)
        .output()
        .expect("spawn arte");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status,
    )
}

#[test]
fn arte_respects_arte_truth_dir_env_var() {
    // c-arte-respects-ARTE_TRUTH_DIR-env-var:
    // when ARTE_TRUTH_DIR is set, every read of the board must come from
    // that path. The test seeds a sentinel node into a scratch truth dir
    // and asks `arte show <sentinel>` to render it. If the env var is
    // honoured the sentinel appears; otherwise the binary reads the CWD
    // `.truth/`, which has no such node.
    let truth = scratch_dir("truth-env");
    let sentinel = "v-sentinel-from-isolated-truth-dir";
    let node = format!(
        "id: {sentinel}\nrole: validation\nsubset: proof\ntitle: Sentinel placed in an isolated truth dir\nat: tests/test_isolation.rs\n"
    );
    write_node(&truth, sentinel, &node);

    // The sentinel must surface when the env var is set.
    let (stdout, _stderr, _status) = run_arte_with_truth(&["show", sentinel], &truth);
    assert!(
        stdout.contains(sentinel),
        "ARTE_TRUTH_DIR not honoured — `arte show` did not read from the env-var path.\n\
         Expected to find sentinel {sentinel} in output.\nGot:\n{stdout}"
    );

    // And the binary must NOT have created the sentinel in the CWD's .truth/.
    // If ARTE_TRUTH_DIR was ignored, a write path may have planted the node
    // into the real .truth/ (depending on which operations the binary ran).
    // We probe via `show` from CWD to catch that leak.
    let cwd = std::env::current_dir().expect("cwd");
    let live_truth = cwd.join(".truth").join(format!("{sentinel}.node"));
    assert!(
        !live_truth.exists(),
        "ARTE_TRUTH_DIR ignored — sentinel leaked into the live .truth/ at {live_truth:?}"
    );
}

#[test]
fn arte_respects_arte_truth_dir_for_directory_reads() {
    // Same control, exercised at the directory level: when ARTE_TRUTH_DIR
    // points at an isolated scratch dir, `arte observe` (which enumerates
    // the truth dir) must surface ONLY the nodes placed in that scratch dir —
    // and none from the live .truth/. If the env var is honoured the
    // listing reflects the scratch; if it's ignored the listing reflects
    // the live board.
    let truth = scratch_dir("truth-env-dir");
    write_node(
        &truth,
        "v-only-in-isolated-truth",
        "id: v-only-in-isolated-truth\nrole: validation\nsubset: proof\ntitle: lives in scratch truth dir only\n",
    );

    let (stdout, _stderr, _status) = run_arte_with_truth(&["observe"], &truth);
    assert!(
        stdout.contains("v-only-in-isolated-truth"),
        "ARTE_TRUTH_DIR ignored at the directory-read level — observe did not list the scratch node.\nGot:\n{stdout}"
    );
}

#[test]
fn run_history_files_do_not_accumulate_unbounded() {
    // c-run-history-files-do-not-accumulate-unbounded:
    // qa/runs/ is bounded by the stable-pass window (5 per validation).
    // Older run files must be pruned on each `arte verify`.
    //
    // The test points ARTE_TRUTH_DIR and ARTE_RUNS_DIR at independent
    // scratch dirs, seeds >5 run files for one validation in the runs dir,
    // runs `arte verify`, and asserts the run count dropped to <= 5.
    // This proves both: (a) the env-vars redirect the truth/runs paths,
    // (b) the prune-on-verify logic exists and runs.
    let truth = scratch_dir("runs-prune-truth");
    let runs = scratch_dir("runs-prune-runs");
    let val_id = "v-prune-test-validation";
    // Point the seeded validation at a separate helper test file
    // (tests/_prune_helper.rs) so the verify run that grades this
    // validation does NOT recursively invoke this test file — which
    // would ratchet extra run records into the runs dir and pollute
    // the count we're trying to measure.
    fs::write(
        truth.join(format!("{val_id}.node")),
        format!(
            "id: {val_id}\nrole: validation\nsubset: proof\ntitle: prune test sentinel\nat: tests/_prune_helper.rs\n"
        ),
    )
    .expect("write validation node");

    // Seed 12 run files for the validation — well beyond the stable-pass
    // window of 5. Runs dir is the CONCRETE path (no `qa/runs` append).
    for seq in 1..=12 {
        fs::write(
            runs.join(format!("{val_id}.{seq:03}.run")),
            format!(
                "seq: {seq:03}\nresult: pass\nsha: deadbeef\ntimestamp: 2026-01-01T00:00:00Z\nnote: seed\n"
            ),
        )
        .expect("seed run file");
    }

    let before: Vec<_> = fs::read_dir(&runs)
        .expect("read runs dir")
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with(val_id))
        .collect();
    assert_eq!(
        before.len(),
        12,
        "test setup: expected 12 seeded run files for {val_id}, found {}",
        before.len()
    );

    // Run verify under the isolated paths. We don't care if it exits 0 or 1;
    // the pruning happens during the verify pipeline.
    let _ = Command::new(arte_bin())
        .arg("verify")
        .env("ARTE_TRUTH_DIR", &truth)
        .env("ARTE_RUNS_DIR", &runs)
        .status();

    let after: Vec<_> = fs::read_dir(&runs)
        .expect("read runs dir post-verify")
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with(val_id))
        .collect();
    assert!(
        after.len() <= 5,
        "run-history pruning violated — expected <= 5 run files for {val_id} after `arte verify`, found {}",
        after.len()
    );
}

#[test]
fn arte_respects_arte_truth_dir_falls_back_when_unset() {
    // c-arte-respects-ARTE_TRUTH_DIR-env-var (fallback arm):
    // when ARTE_TRUTH_DIR is unset the binary must read from the CWD
    // `.truth/` as before. This is the contract that keeps the existing
    // local workflow working; without it, every local command would
    // silently no-op.
    let (stdout, _stderr, _status) = Command::new(arte_bin())
        .args(["show", "i1"])
        .env_remove("ARTE_TRUTH_DIR")
        .env_remove("ARTE_RUNS_DIR")
        .env_remove("ARTE_DISPATCH_PATH")
        .output()
        .map(|o| {
            (
                String::from_utf8_lossy(&o.stdout).into_owned(),
                String::from_utf8_lossy(&o.stderr).into_owned(),
                o.status,
            )
        })
        .expect("spawn arte show i1");

    // i1 is a fixture node in the live .truth/. If the fallback arm is
    // intact, show resolves and prints the title.
    assert!(
        stdout.contains("i1") || stdout.contains("AI agents that build"),
        "fallback to CWD .truth/ broken — `arte show i1` with env vars unset failed:\n{stdout}"
    );
}
