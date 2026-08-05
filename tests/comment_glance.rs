//! v-comment-holds-the-failing-line-on-red-and-is-absent-on-green
//! c-verify-writes-the-failing-reason-to-comment-on-red-and-clears
//!
//! Reported from use: agents work a board for hours and no `comment` ever
//! appears. It was wired nowhere — the CLI never wrote one (zero mentions), and
//! the viewer reserves a comment column whose importer drops the field. So a red
//! validation showed a status and nothing about WHY, and every explanation had
//! to be hand-written as a `note`.
//!
//! The spec (user's, and it is the right one): green carries no comment; red
//! carries the single most useful line, at a glance.
//!
//! FALSIFYING: a failing lane must leave the failure's own words on the node;
//! a passing lane must leave no comment at all — including one it wrote earlier.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_arte");

/// Scratch board whose lane command is supplied by the caller, so one test can
/// drive a red run and then a green run over the same node.
fn board(label: &str, lane: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("arte-comment-{label}-{}-{nanos}", std::process::id()));
    fs::create_dir_all(dir.join("truth")).expect("truth");
    fs::create_dir_all(dir.join("runs")).expect("runs");
    fs::create_dir_all(dir.join("tests")).expect("tests");
    fs::write(dir.join("tests").join("probe.rs"), "// scratch\n").expect("test file");
    fs::write(
        dir.join("truth").join("v-probe.node"),
        "id: v-probe\nrole: validation\nsubset: proof\ntitle: probe\nat: tests/probe.rs\n",
    )
    .expect("node");
    fs::write(
        dir.join("arte.toml"),
        format!("chain = [\"intent\", \"impl\", \"control\", \"validation\"]\n\n[lanes]\nproof = \"{lane}\"\n"),
    )
    .expect("toml");
    dir
}

fn verify(dir: &PathBuf) {
    let _ = Command::new(BIN)
        .args(["verify", "v-probe"])
        .current_dir(dir)
        .env("ARTE_TRUTH_DIR", dir.join("truth"))
        .env("ARTE_RUNS_DIR", dir.join("runs"))
        .output()
        .expect("spawn arte verify");
}

fn field(dir: &PathBuf, key: &str) -> Option<String> {
    let txt = fs::read_to_string(dir.join("truth").join("v-probe.node")).expect("read node");
    txt.lines()
        .find(|l| l.starts_with(&format!("{key}:")))
        .map(|l| l[key.len() + 1..].trim().to_string())
}

#[test]
fn red_writes_the_failing_line_green_clears_it() {
    // A lane that fails while printing something a human would want to read.
    let dir = board("red", "sh -c 'echo building; echo \\\"assertion failed: qty was 10, expected 7\\\"; exit 1' --");
    verify(&dir);
    assert_eq!(field(&dir, "status").as_deref(), Some("ko"), "the red lane must derive ko");
    let comment = field(&dir, "comment").unwrap_or_default();
    assert!(
        comment.contains("assertion failed") && comment.contains("expected 7"),
        "a red validation must carry the failure's OWN words on the board — got {comment:?}. \
         Without it the board says a thing is broken and nothing about why."
    );
    assert!(
        !comment.contains("building"),
        "the comment must be the failing line, not the first line of build noise — got {comment:?}"
    );

    // Same node, now passing: the stale explanation must not survive.
    fs::write(
        dir.join("arte.toml"),
        "chain = [\"intent\", \"impl\", \"control\", \"validation\"]\n\n[lanes]\nproof = \"true\"\n",
    )
    .expect("toml");
    verify(&dir);
    assert_eq!(field(&dir, "status").as_deref(), Some("ok"), "the green lane must derive ok");
    assert_eq!(
        field(&dir, "comment"),
        None,
        "a green validation must carry NO comment — a stale failure line on a passing row is \
         worse than none, because the board then reads as broken when it is not"
    );
    let _ = fs::remove_dir_all(&dir);
}
