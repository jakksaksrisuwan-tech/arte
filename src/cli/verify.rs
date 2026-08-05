//! Verify + contract + trace: the measurement layer.
//! `arte verify` runs each validation's own test and derives status. `arte contract`
//! pins the impl's declared public symbols. `trace_pass` is gate-internal hygiene.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::process::Command;

use crate::*;

// ─── Configs ────────────────────────────────────────────────────────────────

/// `[verify]` + `[lanes]` from arte.toml — the ENVIRONMENT contract for green:
/// setup/teardown hooks (arte owns the sandbox lifecycle), pristine paths (leak
/// check), and per-subset lane commands ("skip" = not auto-run, e.g. human qa).
pub struct VerifyConf {
    pub setup: Option<String>,
    pub teardown: Option<String>,
    pub pristine: Vec<String>,
    pub lanes: HashMap<String, String>,
}

pub fn verify_conf() -> VerifyConf {
    let mut c = VerifyConf { setup: None, teardown: None, pristine: Vec::new(), lanes: HashMap::new() };
    if let Ok(txt) = fs::read_to_string("arte.toml") {
        let mut sect = String::new();
        for raw in txt.lines() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if let Some(inner) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
                sect = inner.trim().to_string();
            } else if let Some((k, v)) = line.split_once('=') {
                let (k, v) = (k.trim(), v.trim().trim_matches('"').to_string());
                match (sect.as_str(), k) {
                    ("verify", "setup") => c.setup = Some(v),
                    ("verify", "teardown") => c.teardown = Some(v),
                    ("verify", "pristine") => {
                        c.pristine = v.trim_matches(['[',']']).split(',').map(|x| x.trim().trim_matches('"').to_string()).filter(|x| !x.is_empty()).collect()
                    }
                    ("lanes", sub) => { c.lanes.insert(sub.to_string(), v); }
                    _ => {}
                }
            }
        }
    }
    c
}

/// `[implement]` from arte.toml — test command + implementer agent (with defaults).
pub fn impl_conf() -> (String, String) {
    let (mut test, mut agent) = ("npx vitest run".to_string(), "pi -p".to_string());
    if let Ok(txt) = fs::read_to_string("arte.toml") {
        let mut in_impl = false;
        for raw in txt.lines() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if let Some(inner) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
                in_impl = inner.trim() == "implement";
            } else if in_impl {
                if let Some((k, v)) = line.split_once('=') {
                    let v = v.trim().trim_matches('"').to_string();
                    match k.trim() {
                        "test" => test = v,
                        "agent" => agent = v,
                        _ => {}
                    }
                }
            }
        }
    }
    (test, agent)
}

// ─── Verify: per-validation measurement ─────────────────────────────────────

/// `arte verify` — run each validation's OWN test and DERIVE that validation's
/// status from IT, not from whole-suite green. A validation whose `at:` test
/// file is missing is flagged a HOLE, never blessed. Per-test, not per-suite.
pub fn cmd_verify() {
    // `arte verify [id...]` — with ids, re-derive ONLY those validations
    // (one flaky lane must not force a full-board regrind).
    let ids: Vec<String> = std::env::args().skip(2).filter(|a| !a.starts_with('-')).collect();
    let (_, ko, holes) = verify_pass_filtered(&ids);
    if ko > 0 || holes > 0 {
        std::process::exit(1);
    }
}

/// The verify run, returning (ok, ko, missing-test holes) so `arte gate` can
/// judge what this prints. Writes each validation's measured status back.
pub fn verify_pass() -> (usize, usize, usize) {
    verify_pass_filtered(&[])
}

/// Like `verify_pass`, but a non-empty `only` restricts the run to those
/// validation ids: only their lanes execute, only their statuses are written.
/// An unknown id is refused loudly — a typo must never read as "verified".
pub fn verify_pass_filtered(only: &[String]) -> (usize, usize, usize) {
    let (test, _) = impl_conf();
    let vconf = verify_conf();
    let nodes = all_nodes();
    if !only.is_empty() {
        for id in only {
            if !nodes.iter().any(|(nid, n)| nid == id && n.get("role") == Some("validation")) {
                eprintln!("✗ unknown validation id: {id} — nothing verified");
                std::process::exit(2);
            }
        }
    }
    // each validation → the test file it points at (`at:` ending in a *.test.* / test/ path)
    let vals: Vec<(String, String, String, String, String)> = nodes
        .iter()
        .filter(|(_, n)| n.get("role") == Some("validation"))
        .filter(|(id, _)| only.is_empty() || only.iter().any(|o| o == id))
        .filter_map(|(id, n)| {
            n.all("at")
                .into_iter()
                .find(|a| {
                    let a = a.trim_start_matches("./");
                    a.contains(".test.")
                        || a.starts_with("test/")
                        || a.starts_with("qa/tests/")
                        || a.starts_with("tests/")
                        || a.starts_with("__tests__/")
                })
                .map(|a| {
                    let path = crate::cli::query::at_path(a).to_string();
                    let anchor = a[path.len()..].trim_start_matches(['#', ':']).to_string();
                    (id.clone(), path, anchor, n.get("subset").unwrap_or("").to_string(), a.to_string())
                })
        })
        .collect();
    if vals.is_empty() {
        println!("no test-backed validations yet — write a test, then `arte at <validation> <test-file>`");
        return (0, 0, 0);
    }
    let scratch = std::env::temp_dir().join(format!("arte-verify-{}", std::process::id()));
    let _ = fs::create_dir_all(&scratch);
    let sh_env = |cmd: &str| {
        Command::new("sh").arg("-c").arg(cmd).env("ARTE_VERIFY_TMP", &scratch).status().map(|st| st.success()).unwrap_or(false)
    };
    if let Some(up) = &vconf.setup {
        println!("• setup: {up}");
        if !sh_env(up) {
            eprintln!("✗ verify setup failed — refusing to grade against an unknown environment");
            let _ = fs::remove_dir_all(&scratch);
            std::process::exit(1);
        }
    }
    let before = snap_pristine(&vconf.pristine);
    let lane_cmd = |subset: &str| -> String { vconf.lanes.get(subset).cloned().unwrap_or_else(|| test.clone()) };
    // Full sha-1: the point of recording it is that someone can `git checkout`
    // this exact commit and re-run the test.
    let head: Option<String> = crate::git_head_sha1();
    let mut file_pass: HashMap<String, Option<bool>> = HashMap::new();
    // key -> the one line worth putting on the board when that lane fails
    let mut failure_line: HashMap<String, String> = HashMap::new();
    let mut written: HashSet<String> = HashSet::new();
    let mut skipped = 0usize;
    let (mut ok, mut ko, mut holes) = (0usize, 0usize, 0usize);
    // FIRST LOOP — run tests AND flush each verdict to disk before starting
    // the next one. A stall on test N can no longer erase tests 0..N-1.
    for (id, f, anchor, subset, a_full) in &vals {
        let cmd_base = lane_cmd(subset);
        if cmd_base == "skip" { continue; }
        let name_filter = cmd_base.contains("vitest") || cmd_base.contains("jest");
        let key = if name_filter && !anchor.is_empty() { format!("{cmd_base}\x00{f}\x00{anchor}") } else { format!("{cmd_base}\x00{f}") };
        if !file_pass.contains_key(&key) {
            let res = if Path::new(f).exists() {
                let cmd = if name_filter && !anchor.is_empty() {
                    let esc = anchor.replace('\\', "\\\\").replace('"', "\\\"").replace('$', "\\$").replace('`', "\\`");
                    format!("{cmd_base} {f} -t \"{esc}\"")
                } else {
                    format!("{cmd_base} {a_full}")
                };
                println!("• {cmd}");
                // Capture rather than stream: a red validation has to be able to
                // say WHY on the board, and that means holding on to the words
                // the test printed. Output is echoed straight after, so nothing
                // is hidden — it just arrives per-test instead of per-line.
                let out = Command::new("sh").arg("-c").arg(&cmd).env("ARTE_VERIFY_TMP", &scratch).output();
                match out {
                    Ok(o) => {
                        let text = format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
                        print!("{text}");
                        let passed = o.status.success();
                        if !passed { failure_line.insert(key.clone(), crate::salient_failure(&text)); }
                        Some(passed)
                    }
                    Err(e) => {
                        println!("  ⚠ could not run: {e}");
                        failure_line.insert(key.clone(), format!("could not run lane command: {e}"));
                        Some(false)
                    }
                }
            } else {
                None
            };
            file_pass.insert(key.clone(), res);
        }
        match file_pass.get(&key).copied().flatten() {
            Some(true) => {
                record_status(id, true, &head, &cmd_base);
                ok += 1;
                written.insert(id.clone());
            }
            Some(false) => {
                record_status_with(id, false, &head, &cmd_base, failure_line.get(&key).map(String::as_str));
                ko += 1;
                written.insert(id.clone());
            }
            None => {
                println!("  ⚠ {id}: at: {f} — no such test file (a validation with no real test)");
                holes += 1;
                written.insert(id.clone());
            }
        }
    }
    // SECOND LOOP — fallback for skipped lanes.
    for (id, f, anchor, subset, _a_full) in &vals {
        if written.contains(id) { continue; }
        let cmd_base = lane_cmd(subset);
        if cmd_base == "skip" { skipped += 1; continue; }
        let name_filter = cmd_base.contains("vitest") || cmd_base.contains("jest");
        let key = if name_filter && !anchor.is_empty() { format!("{cmd_base}\x00{f}\x00{anchor}") } else { format!("{cmd_base}\x00{f}") };
        match file_pass.get(&key).copied().flatten() {
            Some(true) => { record_status(id, true, &head, &cmd_base); ok += 1; }
            Some(false) => { record_status_with(id, false, &head, &cmd_base, failure_line.get(&key).map(String::as_str)); ko += 1; }
            None => { holes += 1; }
        }
    }
    let run_dir = runs_dir();
    prune_runs(Path::new(&run_dir), STABLE_PASS_WINDOW);
    let after = snap_pristine(&vconf.pristine);
    let leaks = if before != after {
        let b: HashMap<&String, &u64> = before.iter().map(|(p, h)| (p, h)).collect();
        let changed: Vec<&String> = after.iter().filter(|(p, h)| b.get(p).map(|x| *x != h).unwrap_or(true)).map(|(p, _)| p).collect();
        let removed: Vec<&String> = before.iter().filter(|(p, _)| !after.iter().any(|(q, _)| q == p)).map(|(p, _)| p).collect();
        println!("✗ ENVIRONMENT LEAK: the suite changed declared-pristine paths:");
        for p in changed.iter().chain(removed.iter()).take(6) {
            println!("    · {p}");
        }
        changed.len() + removed.len()
    } else { 0 };
    if let Some(down) = &vconf.teardown {
        println!("• teardown: {down}");
        let _ = sh_env(down);
    }
    let _ = fs::remove_dir_all(&scratch);
    if skipped > 0 {
        println!("({skipped} validation(s) in \"skip\" lanes not auto-run — their statuses are signed, not measured)");
    }
    println!("verify: {ok} ok · {ko} ko · {holes} missing-test hole(s) · {leaks} leak(s)");
    if leaks > 0 { ko += leaks; }
    // ROLL UP one layer: a control's status is MEASURED from the validations
    // that prove it (worst-wins). Mixed/pending is left alone — `justified`
    // stays a human's call.
    let fresh = all_nodes();
    let conf = read_conf();
    let crole = if conf.chain.len() >= 2 { conf.chain[conf.chain.len() - 2].clone() } else { "control".into() };
    let vrole = conf.chain.last().cloned().unwrap_or_else(|| "validation".into());
    let (mut c_ok, mut c_ko) = (0usize, 0usize);
    for (cid, cnode) in fresh.iter().filter(|(_, n)| n.get("role") == Some(crole.as_str())) {
        let proofs: Vec<&str> = fresh
            .iter()
            .filter(|(_, v)| v.get("role") == Some(vrole.as_str()) && v.all("serves").contains(&cid.as_str()))
            .filter_map(|(_, v)| v.get("status"))
            .collect();
        if proofs.is_empty() { continue; }
        let rolled = if proofs.contains(&"ko") { "ko" }
            else if proofs.iter().all(|s| *s == "ok") { "ok" }
            else { continue };
        if cnode.get("status") != Some("justified") && cnode.get("status") != Some(rolled) {
            let mut n = load_or_exit(cid);
            n.set_field("status", rolled);
            save_node(cid, &n);
        }
        if rolled == "ok" { c_ok += 1 } else { c_ko += 1 }
    }
    if c_ok + c_ko > 0 {
        println!("controls: {c_ok} ok · {c_ko} ko  (rolled up from their validations)");
    }
    (ok, ko, holes)
}

/// Write a validation's status to its .node file + append a run record.
/// Hoisted out of the verify loop so callers (cmd_cycle) can also flush
/// per-test verdicts when they need to.
pub fn record_status(id: &str, pass: bool, head: &Option<String>, cmd: &str) {
    record_status_with(id, pass, head, cmd, None)
}

/// As `record_status`, plus the failing line to put on the board. `comment` is
/// the at-a-glance answer to "why is this red" — written on red, CLEARED on
/// green (a stale failure line on a passing row reads as broken when it is not).
pub fn record_status_with(id: &str, pass: bool, head: &Option<String>, cmd: &str, failure: Option<&str>) {
    let mut n = load_or_exit(id);
    n.set_field("status", if pass { "ok" } else { "ko" });
    if let Some(h) = head { n.set_field("sha", &h); }
    // PROVEN-ness must be STICKY: run history is pruned to the stable-pass
    // window, so a recorded red ages out after a few green runs and the proof
    // that this test CAN fail would silently evaporate. Stamp it on the node
    // the moment a red is observed — earned once, kept.
    if !pass && n.get("proven").is_none() {
        n.set_field("proven", "observed-red");
    }
    match (pass, failure) {
        (false, Some(line)) if !line.is_empty() => n.set_field("comment", line),
        (false, _) => { n.unset_field("comment"); }
        (true, _) => { n.unset_field("comment"); }
    }
    // The tested sha-1: the git-blob digest of the test artifact this status was
    // measured against. `sha:` is git HEAD, which says nothing when the test is
    // untracked — the subject repo's whole spec suite is gitignored, so specs
    // were rewritten dozens of times under an unchanged HEAD. Reproduce with
    // `git hash-object <file>`.
    if let Some(at) = n.get("at").map(str::to_string) {
        let path = crate::cli::query::at_path(&at).to_string();
        if let Some(digest) = crate::git_blob_sha1(std::path::Path::new(&path)) {
            n.set_field("test_sha", &digest);
        }
    }
    save_node(id, &n);
    let rec = RunRec {
        seq: String::new(),
        result: if pass { "pass".into() } else { "fail".into() },
        sha: head.clone().unwrap_or_default(),
        timestamp: now_rfc3339(),
        note: cmd.to_string(),
    };
    if let Err(e) = write_run(id, &rec) {
        eprintln!("warn: could not write qa/runs/{id}.*.run: {e}");
    }
}

/// Pristine-path snapshot (path, content-hash). The leak check compares
/// before-and-after to detect a test that pollutes declared pristine paths.
fn snap_pristine(paths: &[String]) -> Vec<(String, u64)> {
    fn walk(p: &Path, out: &mut Vec<(String, u64)>) {
        if p.is_dir() {
            if let Ok(rd) = fs::read_dir(p) {
                for e in rd.flatten() { walk(&e.path(), out); }
            }
        } else if let Ok(md) = p.metadata() {
            let m = md.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
            out.push((p.to_string_lossy().to_string(), m ^ md.len().rotate_left(17)));
        }
    }
    let mut out = Vec::new();
    for p in paths { walk(Path::new(p), &mut out); }
    out.sort();
    out
}

// ─── Contract: declared public symbols vs source ────────────────────────────

/// `arte contract` — an impl may declare `contract:` lines (public exports).
/// This checks each declared symbol actually appears in the impl's source `at:`
/// — so a reproduction that renames the public API is CAUGHT.
pub fn cmd_contract() {
    if contract_pass() > 0 { std::process::exit(1); }
}

pub fn contract_pass() -> usize {
    let (mut checked, mut violations) = (0usize, 0usize);
    for (id, n) in all_nodes() {
        if n.get("role") != Some("impl") { continue; }
        let contracts = n.all("contract");
        if contracts.is_empty() { continue; }
        let srcs: Vec<String> = n.all("at").into_iter().filter(|a| !a.contains(".test.") && !a.starts_with("test/")).map(String::from).collect();
        if srcs.is_empty() {
            println!("  ⚠ {id}: declares a contract but has no source `at:`");
            violations += 1;
            continue;
        }
        let body: String = srcs.iter().filter_map(|s| fs::read_to_string(s).ok()).collect::<Vec<_>>().join("\n");
        for c in &contracts {
            let sym = c.split(['(', ':', ' ', '<']).next().unwrap_or("").trim();
            checked += 1;
            if !sym.is_empty() && !body.contains(sym) {
                println!("  ✗ {id}: public symbol '{sym}' NOT found in {}", srcs.join(", "));
                violations += 1;
            }
        }
    }
    if violations == 0 {
        println!("✓ contracts hold — {checked} declared public symbol(s) present in source");
    } else {
        println!("✗ {violations} contract violation(s) — the code diverged from the declared interface");
    }
    violations
}

// ─── Trace hygiene (gate-internal) ──────────────────────────────────────────

/// (1) DANGLING `at:` — board pointer at a file that no longer exists.
/// (2) ORPHAN src files — code no node claims via `at:` (unspecced work).
/// Returns (dangling, orphans).
pub fn trace_pass() -> (usize, usize) {
    let nodes = all_nodes();
    let claimed: HashSet<String> =
        nodes.iter().flat_map(|(_, n)| n.all("at").into_iter().map(|a| crate::cli::query::at_path(a).to_string())).collect();
    let mut dangling = 0usize;
    for (id, n) in &nodes {
        for a in n.all("at") {
            let p = crate::cli::query::at_path(a);
            if !Path::new(p).exists() {
                println!("  ✗ {id}: at: {p} — no such file (dangling trace)");
                dangling += 1;
            }
        }
    }
    fn walk(dir: &str, out: &mut Vec<String>) {
        let Ok(rd) = fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() { walk(&p.to_string_lossy(), out); }
            else { out.push(p.to_string_lossy().to_string()); }
        }
    }
    let mut srcs = Vec::new();
    walk("src", &mut srcs);
    srcs.sort();
    let orphans: Vec<&String> =
        srcs.iter().filter(|f| !claimed.contains(*f) && !claimed.iter().any(|c| f.starts_with(&format!("{}/", c.trim_end_matches('/'))))).collect();
    if !orphans.is_empty() {
        println!("  ⚠ {} src file(s) NO node claims via at: (unspecced code):", orphans.len());
        for f in orphans.iter().take(8) {
            println!("      · {f}");
        }
        if orphans.len() > 8 {
            println!("      … and {} more", orphans.len() - 8);
        }
    }
    if dangling == 0 && orphans.is_empty() {
        println!("✓ traces hold — every at: resolves, every src file is claimed");
    }
    (dangling, orphans.len())
}