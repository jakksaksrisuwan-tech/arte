//! `arte gate` — the merge gate: coverage + verify + contract + trace in one
//! command, one exit code. Install as a CI check.

use std::collections::HashMap;

/// The gate: runs every check, collects failures, exits 1 on any.
pub fn cmd_gate() {
    println!("── arte gate · coverage ──");
    let (intents, gaps, choles) = crate::cli::query::coverage_pass();
    println!("── arte gate · verify ──");
    let (ok, ko, vholes) = crate::cli::verify::verify_pass();
    println!("── arte gate · contract ──");
    let violations = crate::cli::verify::contract_pass();
    println!("── arte gate · trace ──");
    let (dangling, orphans) = crate::cli::verify::trace_pass();

    // staleness: a passed gate on stale green is a lie; surface the count
    // (advisory — the gate still passes; we don't fail CI for time, only
    // for broken shape).
    let nodes = crate::all_nodes();
    let mut ahead = HashMap::new();
    let stale = nodes.iter().filter(|(_, n)| crate::staleness(n, &mut ahead).is_some()).count();
    if stale > 0 {
        println!("⏱  {stale} node(s) verified against an OLDER commit than HEAD — re-run `arte verify`");
    }

    // Unproven detectors: a GREEN validation that has never been observed RED
    // is indistinguishable from a test that cannot fail. Advisory like
    // staleness — the shape is whole, but the board should not bank these
    // silently. Fix by red-first authoring, or by an adversary round:
    // inject the fault the validation claims to catch, prove it goes red with
    // the expected message, restore, prove it returns green.
    let unproven: Vec<&String> = nodes
        .iter()
        .filter(|(_, n)| n.get("role") == Some("validation") && n.get("status") == Some("ok"))
        .map(|(id, _)| id)
        .filter(|id| !crate::proven_detector(id))
        .collect();
    if !unproven.is_empty() {
        println!("⚠  {} green validation(s) NEVER observed red — unproven detectors:", unproven.len());
        for id in unproven.iter().take(10) {
            println!("      · {id}");
        }
        if unproven.len() > 10 {
            println!("      … and {} more", unproven.len() - 10);
        }
        println!("   (red-first, or run an adversary round: inject the fault, prove red, restore, prove green)");
    }

    // Test-artifact drift: a green measured against a test whose CONTENT has
    // since changed is stale, and `sha:`/git cannot see it when the test is
    // untracked. Advisory, like staleness — re-verify to re-earn it.
    let drifted: Vec<&String> = nodes
        .iter()
        .filter(|(_, n)| n.get("role") == Some("validation") && n.get("status") == Some("ok"))
        .filter(|(_, n)| match (n.get("at"), n.get("test_sha")) {
            (Some(at), Some(recorded)) => {
                let p = crate::cli::query::at_path(at).to_string();
                crate::test_sha_drift(std::path::Path::new(&p), recorded)
            }
            _ => false,
        })
        .map(|(id, _)| id)
        .collect();
    if !drifted.is_empty() {
        println!("⚠  {} green validation(s) whose TEST CONTENT changed since measurement:", drifted.len());
        for id in drifted.iter().take(10) {
            println!("      · {id}");
        }
        if drifted.len() > 10 {
            println!("      … and {} more", drifted.len() - 10);
        }
        println!("   (re-run `arte verify <id>` — the green was earned against different test content)");
    }

    let mut fails: Vec<String> = Vec::new();
    if intents == 0 { fails.push("no intents on the board — nothing to gate against".into()); }
    if gaps > 0 { fails.push(format!("{gaps} intent(s) not realized in every layer")); }
    if choles > 0 { fails.push(format!("{choles} control(s) with NO validation (reproduction holes)")); }
    if ok == 0 && intents > 0 { fails.push("no green test-backed validation — nothing is measured".into()); }
    if ko > 0 { fails.push(format!("{ko} validation(s) red")); }
    if vholes > 0 { fails.push(format!("{vholes} validation(s) whose test file is missing")); }
    if violations > 0 { fails.push(format!("{violations} contract violation(s)")); }
    if dangling > 0 { fails.push(format!("{dangling} dangling at: pointer(s) — the board points at missing files")); }
    if orphans > 0 { fails.push(format!("{orphans} orphan src file(s) — code no node claims (unspecced work)")); }
    println!("──");
    if fails.is_empty() {
        println!("✓ GATE PASSED — board whole · validations measured green · contracts hold");
    } else {
        println!("✗ GATE FAILED:");
        for f in &fails {
            println!("    · {f}");
        }
        std::process::exit(1);
    }
}