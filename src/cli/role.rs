//! Role isolation + the implementer loop.
//! `arte role X -- <cmd>` wraps a command in the OS sandbox (sandbox-exec on
//! macOS) with the role's deny-list; `arte implement [--watch]` runs the
//! headless implementer agent with one repair pass.

use std::env;
use std::hash::{Hash, Hasher};
use std::process::Command;
use std::time::Duration;

use crate::*;

/// `arte view` — delegate to the artefact-tui binary. Companion tool, optional.
pub fn cmd_view() {
    let dir = env::args().nth(2).unwrap_or_else(|| ".".into());
    match Command::new("arte-tui").arg("arte").arg(&dir).status() {
        Ok(s) => std::process::exit(s.code().unwrap_or(0)),
        Err(_) => {
            eprintln!("viewer not installed — `arte view` delegates to the artefact-tui binary.");
            eprintln!("get it: cargo install --path viewer/tui  (from the artefact repo, or the release bundle)");
            std::process::exit(2);
        }
    }
}

/// Per-role write isolation (review #2): run a command that PHYSICALLY cannot
/// write a role's forbidden dirs — separation of powers as an OS property,
/// not a prose brief. macOS `sandbox-exec`; warns + runs unisolated if
/// unavailable.
fn sandbox_profile(deny_dirs: &[&str]) -> Option<String> {
    let cwd = env::current_dir().ok()?;
    let subs: String = deny_dirs.iter().map(|d| format!("(subpath \"{}\")", cwd.join(d).display())).collect::<Vec<_>>().join(" ");
    Some(format!("(version 1)(allow default)(deny file-write* {subs})"))
}

pub fn run_isolated(deny_dirs: &[&str], program: &str, args: &[&str]) -> std::io::Result<std::process::ExitStatus> {
    run_isolated_as(None, deny_dirs, program, args)
}

/// Like `run_isolated`, but also announces the role to the child via ARTE_ROLE
/// — so an agent launched inside `arte role X -- <agent>` KNOWS its lane
/// instead of discovering it by hitting `Operation not permitted`.
pub fn run_isolated_as(role: Option<&str>, deny_dirs: &[&str], program: &str, args: &[&str]) -> std::io::Result<std::process::ExitStatus> {
    if let Some(profile) = sandbox_profile(deny_dirs) {
        let mut c = Command::new("sandbox-exec");
        c.arg("-p").arg(&profile).arg(program).args(args);
        if let Some(r) = role { c.env("ARTE_ROLE", r); }
        if let Ok(s) = c.status() { return Ok(s); }
    }
    eprintln!("warn: sandbox-exec unavailable — running WITHOUT capability isolation");
    let mut c = Command::new(program);
    c.args(args);
    if let Some(r) = role { c.env("ARTE_ROLE", r); }
    c.status()
}

/// `arte role <role> -- <command...>` — run a command under that role's write
/// isolation (e.g. `arte role implementer -- <agent>` cannot touch test/ or .truth/).
pub fn cmd_role() {
    let args: Vec<String> = env::args().skip(2).collect();
    let (Some(role), Some(sep)) = (args.first().cloned(), args.iter().position(|a| a == "--")) else {
        eprintln!("usage: arte role <role> -- <command...>   (specifier|test-author|implementer|qa|meta-planner)");
        std::process::exit(2);
    };
    let deny = role_deny_dirs(&role);
    if deny.is_empty() {
        eprintln!("unknown role '{role}' — pick one: specifier | test-author | implementer | qa | meta-planner");
        std::process::exit(2);
    }
    let cmd = args[sep + 1..].join(" ");
    let code = run_isolated_as(Some(&role), deny, "sh", &["-c", &cmd]).ok().and_then(|s| s.code()).unwrap_or(1);
    std::process::exit(code);
}

/// `arte implement [--watch]` — fires the headless implementer agent once
/// (default) or repeatedly when the spec fingerprint changes (--watch).
pub fn cmd_implement() {
    let watch = env::args().any(|a| a == "--watch");
    let (test, agent) = crate::cli::verify::impl_conf();
    if !watch {
        implement_pass(&test, &agent);
        return;
    }
    println!("arte implement --watch — watching {}/ + test/ (ctrl-c to stop)", truth_dir());
    implement_pass(&test, &agent);
    let mut last = impl_fingerprint();
    loop {
        std::thread::sleep(Duration::from_secs(2));
        let now = impl_fingerprint();
        if now != last {
            last = now;
            println!("\n▶ spec changed — implementer running");
            implement_pass(&test, &agent);
        }
    }
}

/// Fingerprint of the SPEC sources (`.truth/` + `test/`) — not `src/`, so the
/// implementer's own edits never re-trigger it (no feedback loop).
pub fn impl_fingerprint() -> u64 {
    fn walk(dir: &str, out: &mut Vec<(String, u64, u64)>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p.to_string_lossy(), out);
            } else if let Ok(md) = e.metadata() {
                let m = md.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
                out.push((p.to_string_lossy().to_string(), m, md.len()));
            }
        }
    }
    let mut entries = Vec::new();
    walk(&truth_dir(), &mut entries);
    walk("test", &mut entries);
    entries.sort();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    entries.hash(&mut h);
    h.finish()
}

fn implement_pass(test: &str, agent: &str) {
    println!("• implementer working  ({agent})");
    let parts: Vec<&str> = agent.split_whitespace().collect();
    let Some((bin, args)) = parts.split_first() else { return };
    let run = |brief: &str| {
        let mut argv: Vec<&str> = args.to_vec();
        argv.push(brief);
        // Capture the implementer's exit status — a crashed agent (nonzero)
        // means the gate pass/fail report that follows is untrustworthy.
        match run_isolated(role_deny_dirs("implementer"), bin, &argv) {
            Ok(s) if !s.success() => eprintln!("warn: implementer agent exited with {s:?} — gate verdict below may be unreliable"),
            Err(e) => eprintln!("warn: implementer agent failed to run: {e} — gate verdict below may be unreliable"),
            _ => {}
        }
    };
    let brief1 = format!("{IMPLEMENTER_HEADLESS}\n\nTest command to make pass: `{test}`\n\n{}", role_lane("implementer"));
    run(&brief1);
    println!("• gate (first judgment)");
    if let Err(out) = self_gate() {
        println!("• gate red — one repair pass");
        let brief2 = format!("{brief1}\n\nTHE GATE IS RED. Fix every failure below that is fixable from src/ (make red tests pass, satisfy declared contracts). Report anything your lane cannot fix (e.g. board stamps).\n\n{out}");
        run(&brief2);
    }
    match self_gate() {
        Ok(_) => println!("✓ gate green — board whole, validations measured green"),
        Err(_) => println!("✗ gate still red — a spec needs work, or the fix is outside the implementer's lane (run `arte gate`)"),
    }
}

/// Run our own `gate` as a subprocess, capturing its report. Ok on green,
/// Err on red — the report is what gets fed back to a repair pass.
fn self_gate() -> Result<String, String> {
    let exe = env::current_exe().unwrap_or_else(|_| "arte".into());
    let out = Command::new(exe).arg("gate").output();
    match out {
        Ok(o) => {
            let text = format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
            if o.status.success() { Ok(text) } else { Err(text) }
        }
        Err(e) => Err(format!("gate failed to run: {e}")),
    }
}