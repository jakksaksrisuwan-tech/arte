//! `arte cycle` — the tetrahedron loop driver: PULL / CLASSIFY / DISPATCH /
//! VERIFY / PROMOTE. Reads `qa/runs/*.run`, decides the next move, writes
//! `.loop-dispatch` so a meta-planner (Hermes, Claude Code, Codex) can read it.

use std::collections::HashMap;
use std::env;
use std::fs;

use crate::*;

/// `arte cycle --once` — drive one round of the loop.
/// `--forever` loops until CLASSIFY returns `next: none` for two consecutive
/// rounds (no further progress to make).
pub fn cmd_cycle() {
    let args: Vec<String> = env::args().collect();
    let forever = has_flag(&args, "--forever");
    let mut idle_rounds = 0usize;
    loop {
        println!("\n── cycle round ──");
        // PULL — collapse per validation, look at the recent window
        let mut entries: Vec<(String, RunRec)> = Vec::new();
        if let Ok(rd) = fs::read_dir(runs_dir()) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if !name.ends_with(".run") { continue; }
                // Strip the trailing .NNN.run to recover the v_id (which may
                // itself contain dots — don't split on the first dot, that's
                // lossy).
                let v_id = name
                    .strip_suffix(".run")
                    .and_then(|s| s.rsplit_once('.').map(|(id, _seq)| id.to_string()))
                    .unwrap_or_default();
                if v_id.is_empty() { continue; }
                if let Ok(text) = fs::read_to_string(e.path()) {
                    let n = Node::parse(&text);
                    let rec = RunRec {
                        seq: name.clone(),
                        result: n.get("result").unwrap_or("").to_string(),
                        sha: n.get("sha").unwrap_or("").to_string(),
                        timestamp: n.get("timestamp").unwrap_or("").to_string(),
                        note: n.get("note").unwrap_or("").to_string(),
                    };
                    entries.push((v_id, rec));
                }
            }
        }
        let mut per: HashMap<String, Vec<RunRec>> = HashMap::new();
        for (id, rec) in entries { per.entry(id).or_default().push(rec); }
        for v in per.values_mut() { v.sort_by(|a, b| b.seq.cmp(&a.seq)); }
        // CLASSIFY — naive today (failing → implement). The next incremental
        // (default classifier) will bucket into spec/test/impl via phrase match.
        let mut next = "none".to_string();
        let mut target = "—".to_string();
        let mut reason = String::new();
        for (id, runs) in &per {
            let window = &runs[..runs.len().min(STABLE_PASS_WINDOW)];
            let passes = window.iter().filter(|r| r.result == "pass").count();
            let fails = window.len() - passes;
            if fails > passes && next == "none" {
                next = "implement".into();
                target = id.clone();
                reason = format!("{fails} of last {} failed", window.len());
            }
        }
        // PROMOTE — any validation with stable-pass that is NOT currently ok
        let mut promote: Vec<String> = Vec::new();
        let nodes = all_nodes();
        for (id, runs) in &per {
            let (_passed, stable) = stable_pass(runs);
            if !stable { continue; }
            if let Some(n) = nodes.iter().find(|(nid, _)| nid == id).map(|(_, n)| n) {
                if n.get("status") != Some("ok") { promote.push(id.clone()); }
            }
        }
        // DISPATCH
        let mut disp = Node { fields: Vec::new() };
        disp.push_field("next", &next);
        disp.push_field("target", &target);
        if !reason.is_empty() { disp.push_field("reason", &reason); }
        let dp = dispatch_path();
        if let Err(e) = fs::write(&dp, disp.to_text()) {
            eprintln!("could not write {dp}: {e} — orchestrator will see stale dispatch");
        }
        println!("pull: {} validation(s) with run history", per.len());
        println!("classify: next={next} target={target}{}", if reason.is_empty() { String::new() } else { format!(" ({reason})") });
        println!("promote: {} eligible", promote.len());
        // Cold-start fast-path: nothing to dispatch AND nothing to promote — don't
        // pay the subprocess cost on every cycle round.
        if per.is_empty() && promote.is_empty() && next == "none" {
            println!("nothing to do — board empty");
            if !forever { return; }
            // In --forever mode, idle rounds will accumulate and exit via the loop tail.
            idle_rounds += 1;
            if idle_rounds >= 2 { return; }
            continue;
        }
        // VERIFY — re-run our own verify so any drift gets caught. Capture the
        // exit code: failed verify (red or spawn failure) means we CANNOT
        // promote this round — otherwise status: ok gets written without a
        // matching green run, which is exactly the lie the stable-pass gate exists
        // to prevent.
        println!("verify: running `arte verify`…");
        let verify_ok = match std::process::Command::new(env::current_exe().unwrap_or_else(|_| "arte".into()))
            .arg("verify").output()
        {
            Ok(o) => {
                print!("{}", String::from_utf8_lossy(&o.stdout));
                if o.status.success() {
                    true
                } else {
                    eprintln!("verify red in this round — promotion deferred to next cycle");
                    false
                }
            }
            Err(e) => {
                eprintln!("verify failed to spawn: {e} — promotion deferred");
                false
            }
        };
        // PROMOTE — gated on verify green. Refuse to write status: ok otherwise.
        if verify_ok {
            for id in &promote {
                let Some(mut nn) = load_node(id) else { continue };
                nn.set_field("status", "ok");
                save_node(id, &nn);
                println!("promote: {id} → ok");
            }
        } else {
            println!("promote: skipped (verify not green) — {} eligible deferred", promote.len());
        }
        if !forever { return; }
        if next == "none" {
            idle_rounds += 1;
            if idle_rounds >= 2 {
                println!("cycle: two idle rounds — nothing left to dispatch, exiting");
                return;
            }
        } else {
            idle_rounds = 0;
        }
    }
}