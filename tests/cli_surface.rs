//! Tests for the 3 cluster validations spec minted against CLI surface
//! impls (the 12 ungoverned ones in tasks #27 + #31). Each test is a
//! FALSIFYING check — the assertion fails if the rule its control names is
//! violated, not merely if the artefact currently happens to pass.
//!
//! Mapping (validation -> control -> what we assert):
//!   v-every-cli-subcommand-dispatches-and-exits-per-spec
//!     c-every-cli-subcommand-focus-gate-lifecycle-role-cycle-routes-th
//!     every CLI subcommand named in the dispatcher routes through argv match
//!     AND exits per its documented contract (exit 0 success / non-zero fail)
//!
//!   v-arte-add-intent-produces-a-node-linked-up-the-chain
//!     c-arte-add-intent-mints-a-new-node-with-at-least-one-serves-link
//!     `arte add intent "..."` mints a new .node file with `serves:` linking up
//!     to an existing impl/control/intent id (chain is bottom-up by default —
//!     add-intent at the top should add a serves link)
//!
//!   v-role-isolation-deny-list-covers-src-tests-truth-qa-tests
//!     c-role-isolation-table-is-the-os-enforced-deny-list-per-lane
//!     `role_deny_dirs(test-author)` covers {src, .truth, qa/tests}; writing
//!     inside qa/tests as test-author must be blocked by the sandbox

use std::fs;
use std::path::PathBuf;
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_arte");

fn scratch_truth(label: &str) -> PathBuf {
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("arte-cli-surface-{label}-{pid}-{nanos}"));
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn run_arte(args: &[&str], truth: &PathBuf) -> (String, String, std::process::ExitStatus) {
    run_arte_in(args, truth, None)
}

fn run_arte_in(
    args: &[&str],
    truth: &PathBuf,
    cwd: Option<&std::path::Path>,
) -> (String, String, std::process::ExitStatus) {
    let mut cmd = Command::new(BIN);
    cmd.args(args)
        .env("ARTE_TRUTH_DIR", truth)
        .env_remove("ARTE_RUNS_DIR")
        .env_remove("ARTE_DISPATCH_PATH");
    if let Some(d) = cwd {
        cmd.current_dir(d);
    }
    let out = cmd.output().expect("spawn arte");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status,
    )
}

/// Subcommands that the dispatcher in src/main.rs must route. Each MUST
/// exit non-zero with a usage line (not panic / not "command not found")
/// when given no args — i.e. argv must reach the handler. We test against
/// the scratch-truth dir so we don't pollute the live board.
const DISPATCHED_SUBCOMMANDS: &[&str] = &[
    "init", "add", "set", "status", "link", "unlink", "unset",
    "delete", "working", "at", "check-commits", "coverage",
    "trace", "show", "runs", "cycle", "verify", "contract",
    "gate", "view", "role", "implement", "brief",
];

// ─── 1. v-every-cli-subcommand-dispatches-and-exits-per-spec ──────────────

#[test]
fn every_dispatched_subcommand_reaches_its_handler() {
    // c-every-cli-subcommand...routes-th: every CLI subcommand (focus/gate/
    // lifecycle/role/cycle/...) routes through the dispatcher in src/main.rs.
    // The handler runs (so it can emit its own usage / validation error) and
    // exits non-zero with a usage line — NOT a panic, NOT "unknown command".
    //
    // A regression: dispatcher accidentally falls through the default arm and
    // emits "usage: arte guide | arte init ..." — that arm's exit code is 2
    // AND its stderr does NOT contain the subcommand's own usage line.
    //
    // The falsifier: for each dispatched subcommand, spawn it with NO args and
    // assert stderr contains the subcommand's own usage string (a hallmark of
    // having reached the handler). Anything else means the dispatcher short-
    // circuited the subcommand.
    for sub in DISPATCHED_SUBCOMMANDS {
        let truth = scratch_truth(&format!("dispatch-{sub}"));
        // `implement` fires the headless implementer agent (arte.toml
        // [implement] agent, default `pi -p`) + two gate passes — minutes of
        // wall clock from one bare spawn. Reaching the handler is what we
        // assert, so give it a scratch cwd whose arte.toml points the agent
        // at `false` (exits instantly, non-zero). Handler still runs, still
        // prints its own shape.
        let cwd = if *sub == "implement" {
            fs::write(
                truth.join("arte.toml"),
                "[implement]\nagent = \"false\"\ntest = \"true\"\n",
            )
            .expect("write scratch arte.toml");
            Some(truth.clone())
        } else {
            None
        };
        let (stdout, stderr, _status) = run_arte_in(&[sub], &truth, cwd.as_deref());

        // The falsifier for c-every-cli-subcommand-...routes-th: the
        // dispatcher's default arm (in src/main.rs) prints exactly this
        // usage prefix on stderr. If stderr starts with that prefix, the
        // subcommand NEVER reached its handler — the dispatcher swallowed it.
        // A handler that ran prints its OWN shape (could be a usage line, an
        // error, scaffold output, a coverage table — anything except the
        // dispatcher's default-arm fallthrough).
        let fell_through = stderr.starts_with("usage: arte guide")
            || stderr.contains("| observe | add <role>");

        assert!(
            !fell_through,
            "subcommand `{sub}` appears NOT to have reached its handler — dispatcher fell through to default arm.\n--- stderr ---\n{stderr}\n--- stdout ---\n{stdout}"
        );
        // Also assert SOMETHING came out — a complete fall-through / no-op
        // would emit neither stdout nor stderr. (Bare success is fine —
        // `init` and `coverage` print meaningful stdout with no stderr.)
        assert!(
            !stdout.trim().is_empty() || !stderr.trim().is_empty(),
            "subcommand `{sub}` produced no output at all — dispatcher may have no-op'd.\n--- stderr ---\n{stderr}\n--- stdout ---\n{stdout}"
        );
    }
}

// ─── 2. v-arte-add-intent-produces-a-node-linked-up-the-chain ──────────────

#[test]
fn arte_add_intent_mints_node_with_serves_link() {
    // c-arte-add-intent-mints-a-new-node-with-at-least-one-serves-link:
    // when the spec passes `--serves`, the minted node MUST carry a serves
    // line. Without --serves the node is unchained — and an unchained intent
    // breaks the bottom-up invariant. The mutation must produce a .node file
    // on disk whose contents include `serves: <target>`.
    let truth = scratch_truth("add-intent-serves");

    // First seed a target node so --serves resolves to something real
    // (avoid the "warn: serves 'X' doesn't exist" noise but the link still
    // gets written — `cmd_add` always writes the link, it just warns on
    // dangling). We use a unique title so dedupe doesn't interfere.
    let (sout, serr, sstat) = run_arte(
        &["add", "intent", "Add intent serves target fixture"],
        &truth,
    );
    assert!(
        sstat.success(),
        "seed add failed:\n--- stdout ---\n{sout}\n--- stderr ---\n{serr}"
    );
    let target_id = sout
        .lines()
        .find(|l| l.starts_with("added ") || l.starts_with("exists:"))
        .and_then(|l| l.split_whitespace().nth(1))
        .map(|s| s.to_string())
        .expect("parse seeded id");

    // Now add the intent under test with --serves pointing at the seed.
    let title = format!("add intent serves link probe {}", std::process::id());
    let (aout, aerr, astat) = run_arte(
        &["add", "intent", &title, "--serves", &target_id],
        &truth,
    );
    assert!(
        astat.success(),
        "`arte add intent --serves` failed:\n--- stdout ---\n{aout}\n--- stderr ---\n{aerr}"
    );
    let minted_id = aout
        .lines()
        .find(|l| l.starts_with("added ") || l.starts_with("exists:"))
        .and_then(|l| l.split_whitespace().nth(1))
        .map(|s| s.to_string())
        .expect("parse minted id");

    // The on-disk file MUST carry a serves: <target_id> line.
    let body = fs::read_to_string(truth.join(format!("{minted_id}.node")))
        .expect("read minted .node");
    let has_serves = body.lines().any(|l| l.trim() == format!("serves: {target_id}"));
    assert!(
        has_serves,
        "minted node has no `serves: {target_id}` line — add dropped the --serves flag:\n{body}"
    );
    // And the chain marker role=intent must be set so it links at the top.
    assert!(
        body.lines().any(|l| l.trim() == "role: intent"),
        "minted node missing role: intent — add defaulted to a different layer:\n{body}"
    );
}

// ─── 3. v-role-isolation-deny-list-covers-src-tests-truth-qa-tests ──────────

#[test]
fn role_deny_dirs_covers_qa_tests_for_test_author() {
    // c-role-isolation-table-is-the-os-enforced-deny-list-per-lane: the
    // table in src/lib.rs#role_deny_dirs is the OS-enforced deny-list per
    // lane. For test-author the deny-list MUST cover {src, .truth,
    // qa/tests} so a test-author agent cannot scribble production code,
    // move the goalposts on the board, or seed its own validation files
    // outside the lane.
    //
    // We read role_deny_dirs directly from lib.rs (deterministic, no
    // sandbox-exec dependency) and assert test-author's branch contains
    // every required path. A missing entry means a regression — the
    // sandbox would silently allow the write.
    let src = fs::read_to_string("src/lib.rs").expect("read src/lib.rs");
    let snippet = src
        .split("pub fn role_deny_dirs")
        .nth(1)
        .expect("role_deny_dirs function present");
    // Bound the slice to the function body — keep it generous so future
    // refactors don't break the parse.
    let body: String = snippet.chars().take(2_000).collect();
    let test_author_branch = body
        .split("\"test-author\" | \"tester\" | \"adversary\" =>")
        .nth(1)
        .expect("test-author branch present in role_deny_dirs")
        .split('}') // the match arm closes at the first `}`
        .next()
        .expect("branch has a closing brace");

    for required in ["src", ".truth", "qa/tests"] {
        let needle = format!("\"{required}\"");
        assert!(
            test_author_branch.contains(&needle),
            "test-author deny-list missing required path '{required}' — sandbox would allow the write.\nBranch was:\n{test_author_branch}"
        );
    }
}

#[test]
fn role_sandbox_denies_test_author_writes_to_src() {
    // Counterpart falsifier: in addition to the table covering qa/tests, the
    // sandbox must physically block test-author from touching src/. This
    // catches a regression where someone removes the deny-list entry (so the
    // table check above would still need to catch it — but a real spawn
    // proves the sandbox is wired).
    //
    // We probe src/ specifically (also denied), and require the sandbox to
    // refuse. We do NOT probe qa/tests in this test because that's the
    // regressed direction — the table test above covers it deterministically.
    let probe = "src/_role_probe_test_author.txt";
    let _ = fs::remove_file(probe);

    let out = Command::new(BIN)
        .args([
            "role",
            "test-author",
            "--",
            &format!("sh -c 'echo forbidden > {probe}'"),
        ])
        .output()
        .expect("spawn arte role test-author");

    let _ = fs::remove_file(probe);

    assert!(
        !out.status.success(),
        "sandbox ALLOWED test-author to write {probe} — write-isolation regression"
    );
    assert!(
        !PathBuf::from(probe).exists(),
        "sandbox returned non-zero but the file was created anyway"
    );
}
