//! Tests for the orphan-tracker invariant.
//! c-orphan-src-files-have-no-node-claim-via-at:
//! `arte gate` must report zero orphan src files — every src file should be
//! claimed by at least one node's `at:` (a directory-level `at: src/cli/`
//! claims the whole subtree). The invariant lives in the board itself; the
//! gate's `trace_pass` is the enforcement surface, and the failure line
//! "N orphan src file(s) — code no node claims (unspecced work)" is the
//! observable contract. If a control is added that governs this invariant,
//! the validation here is what proves it.

use std::process::Command;

/// Run `arte gate` and return the merged (stdout + stderr) text + exit status.
fn arte_gate_output() -> (String, std::process::ExitStatus) {
    let out = Command::new(env!("CARGO_BIN_EXE_arte"))
        .args(["gate"])
        .output()
        .expect("spawn arte gate");
    let merged = format!(
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (merged, out.status)
}

/// Extract the orphan count from the gate's output.
///
/// Contract: an ABSENT marker = "no orphans" (the gate's success path doesn't
/// print the failure bullet at all — see src/cli/gate.rs). So we return
/// `Some(0)` when the marker is missing, and `Some(n)` with the parsed count
/// when the gate is reporting orphans. This makes the test fail-for-the-right-
/// reason when the count is non-zero AND pass cleanly on the success path,
/// without a vacuous `.expect()` panic masking green.
fn orphan_count(stdout: &str) -> Option<usize> {
    // The failure bullet (only emitted when orphans > 0):
    //     "    · 6 orphan src file(s) — code no node claims (unspecced work)"
    // We anchor on the trailing phrase to avoid false matches against the
    // trace-section header ("N src file(s) NO node claims via at:") which
    // is informational, not the gate verdict.
    for line in stdout.lines() {
        let l = line.trim().trim_start_matches(['·', '*', ' ']).trim_start();
        if let Some(idx) = l.find(" orphan src file(s)") {
            let prefix = &l[..idx];
            if let Some(last_word) = prefix.split_whitespace().last() {
                if let Ok(n) = last_word.parse::<usize>() {
                    return Some(n);
                }
            }
        }
    }
    // No marker in the output → gate considered the orphan check green.
    Some(0)
}

#[test]
fn gate_reports_zero_orphan_src_files() {
    // c-orphan-src-files-have-no-node-claim-via-at:
    // every src file must be claimed by at least one node's `at:` (a
    // directory-level claim like `at: src/cli/` covers the subtree).
    // Until then `arte gate` prints the orphan count in its failure block.
    let (out, _status) = arte_gate_output();

    // We assert on stdout text, not on exit code, so the test still catches
    // the right thing if the gate's exit code ever changes for a different
    // reason (e.g. control holes are tolerated, contract drift, etc.).
    // The gate emits the verdict only when orphans exist. Silence therefore
    // represents the successful zero-orphan outcome.
    let n = orphan_count(&out).unwrap_or(0);
    assert_eq!(
        n, 0,
        "arte gate reports {n} orphan src file(s) — every src file must be claimed by \
         a node's at: (use `arte at <impl-id> src/cli/` to claim the subtree)."
    );
}

#[test]
fn gate_orphan_marker_is_a_verifiable_string() {
    // Source-string guard: the gate's failure line uses a literal phrase
    // ("orphan src file(s) — code no node claims (unspecced work)") so the
    // test can grep it deterministically. If a refactor renames the line,
    // this test catches the divergence BEFORE the behavioral test starts
    // vacuously passing.
    let src = std::fs::read_to_string("src/cli/gate.rs").expect("src/cli/gate.rs");
    assert!(
        src.contains("orphan src file(s)"),
        "gate.rs must render the 'orphan src file(s)' marker for the test to grep deterministically"
    );
    assert!(
        src.contains("unspecced work"),
        "gate.rs must include the 'unspecced work' suffix — it's part of the test contract"
    );

    // The orphan count itself comes from `trace_pass` in src/cli/verify.rs.
    // If that function is moved/renamed, the behavioral test's grep still
    // works (it scans output), but this guard pins WHERE the count comes
    // from so a refactor can't silently drop it.
    let ver = std::fs::read_to_string("src/cli/verify.rs").expect("src/cli/verify.rs");
    assert!(
        ver.contains("fn trace_pass"),
        "trace_pass must remain in src/cli/verify.rs — the orphan count is its return value"
    );
}

#[test]
fn trace_pass_marks_every_orphan_src_file_individually() {
    // Behavioural oracle mirroring trace_pass: every src file NOT claimed
    // (directly OR via a directory ancestor) must be reported. This is the
    // same logic as src/cli/verify.rs#trace_pass but kept here so a future
    // refactor of the implementation doesn't silently widen or narrow the
    // definition of "claimed".
    use std::collections::HashSet;
    use std::path::Path;

    fn walk(dir: &str, out: &mut Vec<String>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() { walk(&p.to_string_lossy(), out); }
            else { out.push(p.to_string_lossy().to_string()); }
        }
    }
    let mut srcs = Vec::new();
    walk("src", &mut srcs);
    srcs.sort();

    // Mirror the claim set: every `at:` from every node counts as a claim.
    // We rebuild the set the same way trace_pass does — file path, OR any
    // directory ancestor that starts with "<claim>/".
    let mut claimed: HashSet<String> = HashSet::new();
    let node_files = std::fs::read_dir(".truth").expect(".truth dir");
    for entry in node_files.flatten() {
        let p = entry.path();
        if p.extension().and_then(|s| s.to_str()) != Some("node") { continue; }
        let body = std::fs::read_to_string(&p).unwrap_or_default();
        for line in body.lines() {
            let line = line.split('#').next().unwrap_or("").trim();
            if let Some(rest) = line.strip_prefix("at:") {
                let at = rest.trim().trim_start_matches("./").trim();
                // Drop trailing anchor like "#some_func"
                let at_path = at.split('#').next().unwrap_or(at).trim();
                if !at_path.is_empty() {
                    claimed.insert(at_path.to_string());
                }
            }
        }
    }

    let orphans: Vec<&String> = srcs
        .iter()
        .filter(|f| !claimed.contains(*f) && !claimed.iter().any(|c| f.starts_with(&format!("{c}/"))))
        .collect();

    assert!(
        orphans.is_empty(),
        "trace invariant violated — the following src files have no node claim via at: \
         (use `arte at <impl-id> <path>` or claim a parent directory like `at: src/cli/`):\n{}",
        orphans.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("\n")
    );

    // Silence unused warning on Path (kept for future expansion: walking
    // the claimed set against non-existent parents).
    let _ = Path::new(".");
}
