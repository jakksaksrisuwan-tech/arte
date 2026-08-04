//! v-selective-verify-runs-only-the-named-validation-and-leaves-oth
//! c-arte-verify-with-explicit-validation-ids-re-runs-only-those-la
//!
//! Lesson from the raanyang subject rounds: one flaky validation forced a
//! full-board regrind (70+ e2e lanes, shared-fixture interference re-rolled
//! on every run) because `arte verify` had no way to re-derive a single
//! validation. FALSIFYING checks:
//!   1. `arte verify <id>` runs only that lane and writes only that status —
//!      the unnamed validation's status line stays absent.
//!   2. `arte verify <unknown-id>` refuses (non-zero) rather than silently
//!      verifying nothing/everything.
//!   3. bare `arte verify` still derives every validation.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_arte");

/// Scratch board: two validations, each stamped at its own trivially-green
/// test file, with a cwd-local arte.toml whose lane command is `true` (exits
/// 0 regardless of the file argument appended).
fn scratch_board(label: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("arte-selver-{label}-{}-{nanos}", std::process::id()));
    let truth = dir.join("truth");
    fs::create_dir_all(&truth).expect("scratch dirs");
    fs::create_dir_all(dir.join("tests")).expect("tests dir");
    fs::write(dir.join("arte.toml"), "chain = [\"intent\", \"impl\", \"control\", \"validation\"]\n\n[lanes]\nproof = \"true\"\n").expect("toml");
    for v in ["v-one", "v-two"] {
        fs::write(dir.join("tests").join(format!("{v}.rs")), "// scratch test file\n").expect("test file");
        fs::write(
            truth.join(format!("{v}.node")),
            format!("id: {v}\nrole: validation\nsubset: proof\ntitle: {v}\nat: tests/{v}.rs\n"),
        )
        .expect("node");
    }
    dir
}

fn run_verify(dir: &PathBuf, args: &[&str]) -> (String, String, bool) {
    let mut cmd = Command::new(BIN);
    cmd.arg("verify").args(args)
        .current_dir(dir)
        .env("ARTE_TRUTH_DIR", dir.join("truth"))
        .env_remove("ARTE_RUNS_DIR");
    let out = cmd.output().expect("spawn arte verify");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.success(),
    )
}

fn status_of(dir: &PathBuf, v: &str) -> Option<String> {
    let txt = fs::read_to_string(dir.join("truth").join(format!("{v}.node"))).expect("read node");
    txt.lines()
        .find(|l| l.starts_with("status:"))
        .map(|l| l.trim_start_matches("status:").trim().to_string())
}

#[test]
fn named_id_verifies_only_that_validation() {
    let dir = scratch_board("named");
    let (stdout, stderr, ok) = run_verify(&dir, &["v-one"]);
    assert!(ok, "arte verify v-one failed:\n{stdout}\n{stderr}");
    assert_eq!(
        status_of(&dir, "v-one").as_deref(),
        Some("ok"),
        "named validation was not derived ok.\n--- stdout ---\n{stdout}"
    );
    assert_eq!(
        status_of(&dir, "v-two"),
        None,
        "UNNAMED validation gained a status — selective verify leaked onto the rest of the board.\n--- stdout ---\n{stdout}"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn unknown_id_is_refused() {
    let dir = scratch_board("unknown");
    let (stdout, stderr, ok) = run_verify(&dir, &["v-nope"]);
    assert!(
        !ok,
        "arte verify with an unknown id exited 0 — a typo would silently verify nothing.\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}"
    );
    assert_eq!(status_of(&dir, "v-one"), None, "unknown-id run still wrote statuses");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn bare_verify_still_derives_everything() {
    let dir = scratch_board("bare");
    let (stdout, _stderr, ok) = run_verify(&dir, &[]);
    assert!(ok, "bare arte verify failed:\n{stdout}");
    assert_eq!(status_of(&dir, "v-one").as_deref(), Some("ok"));
    assert_eq!(status_of(&dir, "v-two").as_deref(), Some("ok"));
    let _ = fs::remove_dir_all(&dir);
}
