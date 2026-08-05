//! Read-side commands: observe, show, trace, coverage, runs.
//! All derived — never store what you can compute from the graph.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::env;
use std::fs;
use std::path::Path;

use crate::*;

// ─── helpers ────────────────────────────────────────────────────────────────

/// Worst-wins roll-up: ko > pending > justified > ok > (none).
fn rollup(st: &[Option<String>]) -> Option<String> {
    let rank = |s: &Option<String>| match s.as_deref() {
        Some("ko") | Some("fail") => 4,
        Some("pending") => 3,
        Some("justified") => 2,
        Some("ok") => 1,
        _ => 0,
    };
    let best = st.iter().max_by_key(|s| rank(s))?;
    if rank(best) > 0 { best.clone() } else { None }
}

pub fn trunc(s: &str, n: usize) -> String {
    // Re-exported via lib.rs (was duplicated; kept here as a passthrough to keep
    // call sites inside query.rs unchanged).
    crate::trunc(s, n)
}

/// The file path part of an `at:` value — anchors after `#` (FORMAT's
/// `file#unit`) or `:` (grep/vitest habit, `file:line` / `file:test name`)
/// are stripped. Postel: testers in the wild stamp both; both must resolve.
pub fn at_path(a: &str) -> &str {
    a.split(['#', ':']).next().unwrap_or(a)
}

/// True when test/ changed after the newest status write.
fn tests_stale() -> bool {
    fn newest(d: &Path, node_only: bool, out: &mut u64) {
        if let Ok(rd) = fs::read_dir(d) {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    newest(&p, node_only, out);
                } else if !node_only || p.extension().is_some_and(|x| x == "node") {
                    if let Ok(md) = e.metadata() {
                        if let Ok(m) = md.modified().map(|t| t.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)) {
                            *out = (*out).max(m);
                        }
                    }
                }
            }
        }
    }
    let (mut tests_m, mut truth_m) = (0u64, 0u64);
    newest(Path::new("test"), false, &mut tests_m);
    newest(Path::new(&truth_dir()), true, &mut truth_m);
    tests_m > truth_m
}

/// True when src/ changed after the newest status write.
fn src_stale() -> bool {
    fn newest(d: &Path, out: &mut u64) {
        if let Ok(rd) = fs::read_dir(d) {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    newest(&p, out);
                } else if let Ok(md) = e.metadata() {
                    if let Ok(m) = md.modified().map(|t| t.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)) {
                        *out = (*out).max(m);
                    }
                }
            }
        }
    }
    fn newest_node(out: &mut u64) {
        if let Ok(rd) = fs::read_dir(truth_dir()) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().is_some_and(|x| x == "node") {
                    if let Ok(md) = e.metadata() {
                        if let Ok(m) = md.modified().map(|t| t.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)) {
                            *out = (*out).max(m);
                        }
                    }
                }
            }
        }
    }
    let (mut src_m, mut truth_m) = (0u64, 0u64);
    newest(Path::new("src"), &mut src_m);
    newest_node(&mut truth_m);
    src_m > truth_m
}

/// Ancestors of a node, walking `serves` UP transitively.
pub fn closure_up(start: &Node, by_id: &HashMap<&str, &Node>) -> HashSet<String> {
    let mut seen = HashSet::new();
    let mut stack: Vec<String> = start.all("serves").iter().map(|s| s.to_string()).collect();
    while let Some(x) = stack.pop() {
        if seen.insert(x.clone()) {
            if let Some(p) = by_id.get(x.as_str()) {
                stack.extend(p.all("serves").iter().map(|s| s.to_string()));
            }
        }
    }
    seen
}

/// Every ancestor's notes up the serves chain (cycles safe).
fn inherited_notes(n: &Node, by_id: &HashMap<&str, &Node>) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut stack: Vec<String> = n.all("serves").iter().map(|s| s.to_string()).collect();
    let mut seen: HashSet<String> = HashSet::new();
    while let Some(pid) = stack.pop() {
        if !seen.insert(pid.clone()) { continue; }
        if let Some(p) = by_id.get(pid.as_str()) {
            for note in p.all("note") {
                out.push((pid.clone(), note.to_string()));
            }
            stack.extend(p.all("serves").iter().map(|s| s.to_string()));
        }
    }
    out
}

// ─── observe ────────────────────────────────────────────────────────────────

#[derive(Default)]
struct Disp {
    id: String,
    title: String,
    serves: Vec<String>,
    at: Vec<String>,
    parent: String,
    status: Option<String>,
}

pub fn observe() {
    let conf = read_conf();
    match env::var("ARTE_ROLE") {
        Ok(role) => {
            let deny = role_deny_dirs(&role);
            println!("▶ your role: {role}  (write-isolated from: {})\n", if deny.is_empty() { "—".into() } else { deny.join(", ") });
        }
        Err(_) => println!("▶ your role: specifier (default — no ARTE_ROLE set). Map the board first;\n  hand test-writing and src changes to their roles (subagents or later phases).\n"),
    }
    let nodes = all_nodes();
    if nodes.is_empty() {
        crate::cli::lifecycle::suggest_bootstrap();
        return;
    }
    if tests_stale() {
        println!("⚠ statuses may be STALE (test/ changed since the last verify) — run `arte verify`\n");
    }
    let focus = crate::cli::focus::read_focus();
    if !focus.is_empty() {
        let titles: Vec<String> = focus
            .iter()
            .map(|fid| nodes.iter().find(|(id, _)| id == fid).and_then(|(_, n)| n.get("title")).unwrap_or(fid).to_string())
            .collect();
        println!("▶ working on: {}\n", titles.join(" · "));
    }
    let all_ids: HashSet<String> = nodes.iter().map(|(id, _)| id.clone()).collect();
    let mut g: BTreeMap<String, BTreeMap<String, Vec<Disp>>> = BTreeMap::new();
    for (id, n) in &nodes {
        let role = n.get("role").unwrap_or("?").to_string();
        let subset = n.get("subset").unwrap_or("general").to_string();
        g.entry(role).or_default().entry(subset).or_default().push(Disp {
            id: id.clone(),
            title: n.get("title").unwrap_or("").to_string(),
            serves: n.all("serves").iter().map(|s| s.to_string()).collect(),
            at: n.all("at").iter().map(|s| s.to_string()).collect(),
            parent: n.get("parent").unwrap_or("").to_string(),
            status: n.get("status").map(String::from),
        });
    }
    let mut roles: Vec<String> = conf.chain.clone();
    for r in g.keys() {
        if !roles.contains(r) { roles.push(r.clone()); }
    }
    for role in roles {
        let Some(subsets) = g.get(&role) else { continue };
        let axis = conf.subset_axis.get(&role).map(|a| format!("  ({a})")).unwrap_or_default();
        println!("[{role}]{axis}");
        let mut order: Vec<String> = conf.subsets.get(&role).cloned().unwrap_or_default();
        for f in subsets.keys() {
            if !order.contains(f) { order.push(f.clone()); }
        }
        for subset in order {
            let Some(items) = subsets.get(&subset) else { continue };
            println!("  {subset}/");
            let ids: HashSet<&str> = items.iter().map(|d| d.id.as_str()).collect();
            let mut roots: Vec<&Disp> = items.iter().filter(|d| d.parent.is_empty() || !ids.contains(d.parent.as_str())).collect();
            roots.sort_by(|a, b| a.id.cmp(&b.id));
            for d in roots {
                print_node(d, items, &all_ids, 2);
            }
        }
    }
    // staleness summary — same signal as coverage's, lifted to the front door
    let mut ahead = HashMap::new();
    let stale = nodes.iter().filter(|(_, n)| staleness(n, &mut ahead).is_some()).count();
    if stale > 0 {
        println!("\n⏱  {stale} node(s) verified against an OLDER commit than HEAD — re-run `arte verify`");
    }
}

fn print_node(d: &Disp, all: &[Disp], all_ids: &HashSet<String>, depth: usize) {
    let pad = "  ".repeat(depth);
    let status = d.status.as_deref().map(|s| format!("  [{s}]")).unwrap_or_default();
    let up = if d.serves.is_empty() {
        String::new()
    } else {
        let parts: Vec<String> = d.serves.iter().map(|s| if all_ids.contains(s) { s.clone() } else { format!("⚠{s}(missing)") }).collect();
        format!("  ↑{}", parts.join(","))
    };
    let at = if d.at.is_empty() { String::new() } else { format!("  @{}", d.at.join(",")) };
    let dangling_parent = if !d.parent.is_empty() && !all_ids.contains(&d.parent) { format!("  ⚠parent {}(missing)", d.parent) } else { String::new() };
    println!("{pad}{}  {}{status}{up}{at}{dangling_parent}", d.id, d.title);
    let mut kids: Vec<&Disp> = all.iter().filter(|c| c.parent == d.id).collect();
    kids.sort_by(|a, b| a.id.cmp(&b.id));
    for c in kids {
        print_node(c, all, all_ids, depth + 1);
    }
}

// ─── show ───────────────────────────────────────────────────────────────────

/// `arte show <id>` — one node WITH its inherited context (ancestor notes,
/// walked up the serves chain). The read verb for role agents.
pub fn cmd_show() {
    let Some(id) = env::args().nth(2) else {
        eprintln!("usage: arte show <id>");
        std::process::exit(2);
    };
    let nodes = all_nodes();
    let by_id: HashMap<&str, &Node> = nodes.iter().map(|(id, n)| (id.as_str(), n)).collect();
    let Some(n) = by_id.get(id.as_str()) else {
        eprintln!("no node '{id}'");
        std::process::exit(2);
    };
    for (k, v) in &n.fields {
        println!("{k}: {v}");
    }
    let mut ahead = HashMap::new();
    if let Some(note) = staleness(n, &mut ahead) {
        println!();
        println!("{note}");
    }
    let inherited = inherited_notes(n, &by_id);
    if !inherited.is_empty() {
        println!("\ninherited context (ancestor notes — BINDING for this node):");
        for (pid, note) in inherited {
            println!("  ↑ {pid}: {note}");
        }
    }
}

// ─── trace ───────────────────────────────────────────────────────────────────

/// `arte trace <id>` — walk `serves` UP to the intent(s) the node satisfies.
pub fn cmd_trace() {
    let Some(id) = env::args().nth(2) else {
        eprintln!("usage: arte trace <id>");
        std::process::exit(2);
    };
    let nodes = all_nodes();
    let by_id: HashMap<&str, &Node> = nodes.iter().map(|(id, n)| (id.as_str(), n)).collect();
    if !by_id.contains_key(id.as_str()) {
        eprintln!("no node '{id}'");
        std::process::exit(2);
    }
    let mut ahead = HashMap::new();
    trace_up(&id, &by_id, 0, &mut ahead);
}

fn trace_up(id: &str, by_id: &HashMap<&str, &Node>, depth: usize, ahead: &mut HashMap<String, i64>) {
    let pad = "  ".repeat(depth);
    let arrow = if depth == 0 { "" } else { "↑ " };
    match by_id.get(id) {
        Some(n) => {
            let st = n.get("status").map(|s| format!("  [{s}]")).unwrap_or_default();
            println!("{pad}{arrow}{id}  ({}){st}  {}", n.get("role").unwrap_or("?"), n.get("title").unwrap_or(""));
            if let Some(note) = staleness(n, ahead) {
                for line in note.lines() {
                    println!("{pad}  {line}");
                }
            }
            for s in n.all("serves") {
                trace_up(s, by_id, depth + 1, ahead);
            }
        }
        None => println!("{pad}{arrow}{id}  ⚠(missing)"),
    }
}

// ─── coverage ───────────────────────────────────────────────────────────────

/// `arte coverage` — per intent: realized in every downstream layer? + status roll-up.
pub fn cmd_coverage() {
    coverage_pass();
}

/// The coverage report, returning (intents, gaps, control-holes) so `arte gate`
/// can judge what this prints.
pub fn coverage_pass() -> (usize, usize, usize) {
    let conf = read_conf();
    let nodes = all_nodes();
    let by_id: HashMap<&str, &Node> = nodes.iter().map(|(id, n)| (id.as_str(), n)).collect();
    let closures: Vec<(&Node, HashSet<String>)> = nodes.iter().map(|(_, n)| (n, closure_up(n, &by_id))).collect();
    let Some(spine) = conf.chain.first().map(|s| s.as_str()) else { return (0, 0, 0) };
    let vrole = conf.chain.last().map(|s| s.as_str()).unwrap_or("validation");
    let downstream: Vec<String> = conf.chain.iter().skip(1).cloned().collect();
    let intents: Vec<(&str, &Node)> = nodes.iter().filter(|(_, n)| n.get("role") == Some(spine)).map(|(id, n)| (id.as_str(), n)).collect();
    let (mut covered, mut verified, mut gaps) = (0usize, 0usize, 0usize);
    let mut rows: Vec<String> = Vec::new();
    for &(iid, intent) in &intents {
        let mut cells = Vec::new();
        let mut statuses = vec![intent.get("status").map(String::from)];
        let mut whole = true;
        for role in &downstream {
            let mut any = false;
            for (n, cl) in &closures {
                if n.get("role") == Some(role.as_str()) && cl.contains(iid) {
                    any = true;
                    statuses.push(n.get("status").map(String::from));
                }
            }
            whole &= any;
            cells.push(format!("{role}{}", if any { "✓" } else { "✗" }));
        }
        let is_verified = closures.iter().any(|(n, cl)| {
            n.get("role") == Some(vrole)
                && cl.contains(iid)
                && n.get("status") == Some("ok")
                && n.all("at").iter().any(|a| {
                    a.contains(".test.")
                        || a.starts_with("tests/")
                        || a.starts_with("test/")
                        || a.starts_with("qa/tests/")
                        || a.starts_with("__tests__/")
                })
        });
        let mark = if is_verified { "⚡verified" } else if whole { "·unproven" } else { "" };
        let chip = rollup(&statuses).map(|s| format!("[{s}]")).unwrap_or_else(|| "[—]".into());
        rows.push(format!("  {} {}  {chip} {mark}", trunc(intent.get("title").unwrap_or(""), 30), cells.join(" ")));
        if whole { covered += 1; } else { gaps += 1; }
        if is_verified { verified += 1; }
    }
    if tests_stale() {
        println!("  ⚠ test/ changed AFTER the last status write — statuses may be stale, run `arte verify`");
    }
    if src_stale() {
        println!("  ⚠ src/ changed AFTER the last status write — every green predates the current code, run `arte verify`");
    }
    if let Some(h) = git_short_head() {
        let old = nodes.iter().filter(|(_, n)| n.get("status").is_some() && n.get("sha").is_some_and(|s| s != h)).count();
        if old > 0 {
            println!("  ⚠ {old} status(es) measured against an OLDER commit than HEAD — re-verify to re-attribute");
        }
    }
    let crole = if conf.chain.len() >= 2 { conf.chain[conf.chain.len() - 2].clone() } else { "control".into() };
    let binding = ["must", "never", "always", "only", "exactly", "required"];
    let words = |t: &str| t.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| w.len() > 3).map(String::from).collect::<HashSet<_>>();
    let mut loose = 0usize;
    for (id, n) in nodes.iter().filter(|(_, n)| matches!(n.get("role"), r if r == conf.chain.first().map(String::as_str) || r == conf.chain.get(1).map(String::as_str))) {
        for note in n.all("note") {
            let low = note.to_lowercase();
            if !binding.iter().any(|b| low.contains(b)) { continue; }
            let nw = words(note);
            let spoken_for = closures.iter().any(|(cn, cl)| {
                cn.get("role") == Some(crole.as_str())
                    && cl.contains(id.as_str())
                    && words(cn.get("title").unwrap_or("")).intersection(&nw).count() >= 2
            });
            if !spoken_for {
                if loose == 0 {
                    println!("  ⚠ binding language in notes with NO matching control (note→control lint):");
                }
                loose += 1;
                if loose <= 5 {
                    println!("      · {id}: {}", trunc(note, 58));
                }
            }
        }
    }
    if loose > 5 { println!("      … and {} more", loose - 5); }

    let holes: Vec<&str> = nodes
        .iter()
        .filter(|(id, n)| {
            n.get("role") == Some(crole.as_str())
                && !closures.iter().any(|(vn, cl)| vn.get("role") == Some(vrole) && cl.contains(id.as_str()))
        })
        .map(|(id, _)| id.as_str())
        .collect();
    let irole = conf.chain.get(1).cloned().unwrap_or_else(|| "impl".into());
    let bare: Vec<&str> = nodes
        .iter()
        .filter(|(id, n)| {
            n.get("role") == Some(irole.as_str())
                && !closures.iter().any(|(cn, cl)| cn.get("role") == Some(crole.as_str()) && cl.contains(id.as_str()))
        })
        .map(|(id, _)| id.as_str())
        .collect();
    if !bare.is_empty() {
        println!("  ⚠ {} impl(s) with NO control (ungoverned surface — nothing constrains them):", bare.len());
        for iid in bare.iter().take(8) {
            let t = nodes.iter().find(|(i, _)| i == iid).and_then(|(_, n)| n.get("title")).unwrap_or(iid);
            println!("      · {}", trunc(t, 62));
        }
    }
    if !holes.is_empty() {
        println!("  ⚠ {} control(s) with NO validation (reproduction holes):", holes.len());
        for cid in holes.iter().take(8) {
            let t = nodes.iter().find(|(i, _)| i == cid).and_then(|(_, n)| n.get("title")).unwrap_or(cid);
            println!("      · {}", trunc(t, 62));
        }
        if holes.len() > 8 {
            println!("      … and {} more", holes.len() - 8);
        }
    }
    println!("  ──");
    for r in &rows { println!("{r}"); }
    println!("  ──");
    println!("  {} intents · {covered} covered · {verified} VERIFIED (test-backed+green) · {gaps} gap(s)", intents.len());
    // PROVEN: of the green validations, how many have ever been observed red?
    // An unproven green may be a test that cannot fail (measured: the first
    // never-red validation fault-injected on the subject repo was fake).
    {
        let greens: Vec<&String> = nodes
            .iter()
            .filter(|(_, n)| n.get("role") == Some("validation") && n.get("status") == Some("ok"))
            .map(|(id, _)| id)
            .collect();
        if !greens.is_empty() {
            let proven = greens.iter().filter(|id| crate::proven_detector(id)).count();
            println!("  {proven}/{} green validations PROVEN (observed red at least once)", greens.len());
        }
    }
    (intents.len(), gaps, holes.len())
}

// ─── runs ────────────────────────────────────────────────────────────────────

/// `arte runs <v-id>` — print the recent run window + stable-pass verdict.
pub fn cmd_runs() {
    let Some(id) = env::args().nth(2) else {
        eprintln!("usage: arte runs <v-id>");
        std::process::exit(2);
    };
    let runs = load_runs(&id);
    if runs.is_empty() {
        println!("no runs yet for {id} — `arte verify` writes them, or write qa/runs/{id}.001.run by hand");
        return;
    }
    println!("{id} — last {} run(s), newest first:", runs.len().min(STABLE_PASS_WINDOW));
    for r in runs.iter().take(STABLE_PASS_WINDOW) {
        let mark = if r.result == "pass" { "✓" } else if r.result == "fail" { "✗" } else { "?" };
        let when = if r.timestamp.is_empty() { String::new() } else { format!("  {}", r.timestamp) };
        let sha = if r.sha.is_empty() { String::new() } else { format!("  {}", r.sha) };
        let note = if r.note.is_empty() { String::new() } else { format!("  ({})", trunc(&r.note, 40)) };
        println!("  {mark} {}{when}{sha}{note}", r.seq);
    }
    let (passed, stable) = stable_pass(&runs);
    println!("{passed} of last {} pass — {}", STABLE_PASS_WINDOW, if stable { "stable-pass ✓" } else { "not stable" });
}