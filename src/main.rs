//! arte — a design-truth primitive.
//!
//! Truth is a graph of NODES. Each node is ONE file under `.truth/`, named by a
//! stable opaque id (`.truth/<id>.node`). A node is line-oriented `key: value`:
//! repeated keys = multi-value. Identity is the id (filename), NEVER the title —
//! so a rename is just a label change and links never break. One-node-per-file +
//! line-oriented means **git's own merge reconciles concurrent edits per field**;
//! the tool barely has to merge at all.
//!
//! CLI:
//!   arte init [--template software|hardware|generic|arte]
//!                    scaffold `arte.toml` (default: software) + `.truth/`
//!   arte observe     read the graph back, grouped by role → subset
//!   arte add <role> "<title>" [--subset F --parent P --serves S --category C]
//!   arte set <id> <key> <value>      arte unset <id> <key>       arte status <id> <v>
//!   arte link <id> <serves-id>       arte unlink <id> <serves-id>
//!   arte delete <id>                 arte template save <name> | list
//!   arte coverage                    arte trace <id>
//!   arte gate        coverage + verify + contract as ONE exit code (the CI merge gate)
//!
//! Everything else (link, status, coverage, trace, reconcile, the git field-merge
//! driver, clients) is for the next agent — see HANDOFF.md.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::Path;

const TRUTH_DIR: &str = ".truth";

fn main() {
    match std::env::args().nth(1).as_deref() {
        None => observe(), // bare `arte` = front door: show the board, or guide init if empty
        Some("guide") | Some("help") => print!("{AGENT_GUIDE}"),
        Some("init") => init(),
        Some("observe") => observe(),
        Some("add") => cmd_add(),
        Some("set") => cmd_set(),
        Some("status") => cmd_status(),
        Some("link") => cmd_link(),
        Some("unlink") => cmd_unlink(),
        Some("unset") => cmd_unset(),
        Some("delete") | Some("rm") => cmd_delete(),
        Some("working") | Some("focus") => cmd_working(),
        Some("at") => cmd_at(),
        Some("coverage") => cmd_coverage(),
        Some("trace") => cmd_trace(),
        Some("implement") => cmd_implement(),
        Some("verify") => cmd_verify(),
        Some("contract") => cmd_contract(),
        Some("gate") => cmd_gate(),
        Some("view") => cmd_view(),
        Some("role") => cmd_role(),
        Some("template") => template_cmd(),
        _ => {
            eprintln!("usage: arte guide  |  arte init [--template NAME] | observe | add <role> \"title\" | set <id> <k> <v> | unset <id> <k> | status <id> <v> | link/unlink <id> <target> | delete <id> | template save|list");
            std::process::exit(2);
        }
    }
}

/// A node: ordered fields, multi-value via repeated keys. We keep it as a flat
/// list of (key, value) so unknown keys round-trip untouched (forward-compat).
struct Node {
    fields: Vec<(String, String)>,
}
impl Node {
    fn get(&self, k: &str) -> Option<&str> {
        self.fields.iter().find(|(key, _)| key == k).map(|(_, v)| v.as_str())
    }
    fn all(&self, k: &str) -> Vec<&str> {
        self.fields.iter().filter(|(key, _)| key == k).map(|(_, v)| v.as_str()).collect()
    }
    /// Serialize: `key: value`, one per line. Stable order = stable diffs.
    fn to_text(&self) -> String {
        let mut s = String::new();
        for (k, v) in &self.fields {
            s.push_str(k);
            s.push_str(": ");
            s.push_str(v);
            s.push('\n');
        }
        s
    }
    fn parse(text: &str) -> Node {
        let fields = text
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
            .filter_map(|l| l.split_once(':').map(|(k, v)| (k.trim().to_string(), v.trim().to_string())))
            .collect();
        Node { fields }
    }
    /// Set a scalar field (replace first occurrence, else append). Preserves all
    /// other fields — round-trip safe, so unknown keys survive an edit.
    fn set_field(&mut self, k: &str, v: &str) {
        if let Some(f) = self.fields.iter_mut().find(|(key, _)| key == k) {
            f.1 = v.to_string();
        } else {
            self.fields.push((k.to_string(), v.to_string()));
        }
    }
    /// Append a multi-value field (e.g. serves), skipping an exact duplicate.
    fn push_field(&mut self, k: &str, v: &str) {
        if !self.fields.iter().any(|(kk, vv)| kk == k && vv == v) {
            self.fields.push((k.to_string(), v.to_string()));
        }
    }
    /// Remove every occurrence of a field (clear a scalar, or all of a multi-value).
    fn unset_field(&mut self, k: &str) -> bool {
        let before = self.fields.len();
        self.fields.retain(|(key, _)| key != k);
        self.fields.len() != before
    }
    /// Remove one specific (key, value) — e.g. a single `serves` link.
    fn remove_value(&mut self, k: &str, v: &str) -> bool {
        let before = self.fields.len();
        self.fields.retain(|(key, val)| !(key == k && val == v));
        self.fields.len() != before
    }
}

fn node_path(id: &str) -> String {
    format!("{TRUTH_DIR}/{id}.node")
}

fn write_node(id: &str, role: &str, subset: &str, category: &str, parent: &str, title: &str, serves: &[&str]) {
    let mut fields = vec![
        ("id".into(), id.to_string()),
        ("role".into(), role.to_string()),
        ("subset".into(), subset.to_string()), // top grouping (root level of the layer's tree)
    ];
    if !category.is_empty() {
        fields.push(("category".into(), category.to_string())); // type axis (orthogonal)
    }
    if !parent.is_empty() {
        fields.push(("parent".into(), parent.to_string())); // decomposition axis: part-of (the tree)
    }
    fields.push(("title".into(), title.to_string()));
    for s in serves {
        fields.push(("serves".into(), s.to_string())); // realization axis: realizes (across layers)
    }
    let _ = fs::write(node_path(id), Node { fields }.to_text());
}

// ---- mutators: turn arte from a viewer into a workbench ----
fn node_exists(id: &str) -> bool {
    Path::new(&node_path(id)).exists()
}
fn load_node(id: &str) -> Option<Node> {
    fs::read_to_string(node_path(id)).ok().map(|t| Node::parse(&t))
}
fn save_node(id: &str, n: &Node) {
    if let Err(e) = fs::write(node_path(id), n.to_text()) {
        eprintln!("error writing {id}: {e}");
        std::process::exit(1);
    }
}
/// Load a node or exit(2) with the uniform "no node" error — the mutator prelude.
fn load_or_exit(id: &str) -> Node {
    load_node(id).unwrap_or_else(|| {
        eprintln!("no node '{id}'");
        std::process::exit(2);
    })
}
/// The one graph loader every reader shares: each `.truth/*.node` as
/// (filename-id, parsed node), sorted by id. Filename is the identity — warn
/// once if a stored `id:` disagrees. This is the only place we scan `.truth`.
fn all_nodes() -> Vec<(String, Node)> {
    let mut out = Vec::new();
    let Ok(dir) = fs::read_dir(TRUTH_DIR) else { return out };
    for e in dir.flatten() {
        if e.path().extension().map(|x| x != "node").unwrap_or(true) {
            continue;
        }
        let id = e.path().file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let Ok(text) = fs::read_to_string(e.path()) else { continue };
        let n = Node::parse(&text);
        if let Some(cid) = n.get("id") {
            if cid != id {
                eprintln!("warn: {id}.node has id: {cid} — filename wins");
            }
        }
        out.push((id, n));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}
fn slugify(s: &str) -> String {
    let mut o = String::new();
    let mut dash = false;
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() {
            o.push(ch.to_ascii_lowercase());
            dash = false;
        } else if !dash && !o.is_empty() {
            o.push('-');
            dash = true;
        }
    }
    o.trim_matches('-').chars().take(62).collect::<String>().trim_matches('-').to_string()
}
/// Stable, readable id: `<role-initial>-<slug>`, deduped against existing files.
fn gen_id(role: &str, title: &str) -> String {
    let pre = match role {
        "intent" => "i",
        "impl" => "m",
        "control" => "c",
        "validation" => "v",
        _ => role.get(..1).unwrap_or("n"),
    };
    let base = format!("{pre}-{}", slugify(title));
    let (mut id, mut n) = (base.clone(), 2);
    while node_exists(&id) {
        id = format!("{base}-{n}");
        n += 1;
    }
    id
}
fn default_subset(role: &str) -> String {
    read_conf().subsets.get(role).and_then(|v| v.first()).cloned().unwrap_or_else(|| "TBD".into())
}
fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn cmd_add() {
    let mut a = std::env::args().skip(2);
    let (Some(role), Some(title)) = (a.next(), a.next()) else {
        eprintln!("usage: arte add <role> \"<title>\" [--subset F] [--parent P] [--serves S] [--category C]");
        std::process::exit(2);
    };
    let rest: Vec<String> = a.collect();
    let conf = read_conf();
    if !conf.chain.contains(&role) {
        eprintln!("warn: role '{role}' not in chain {:?}", conf.chain);
    }
    let subset = flag(&rest, "--subset").unwrap_or_else(|| default_subset(&role));
    if let Some(fr) = conf.subsets.get(&role) {
        if !fr.contains(&subset) {
            eprintln!("warn: subset '{subset}' not declared in [subsets] for {role}");
        }
    }
    let id = gen_id(&role, &title);
    let mut n = Node { fields: Vec::new() };
    n.set_field("id", &id);
    n.set_field("role", &role);
    n.set_field("subset", &subset);
    if let Some(c) = flag(&rest, "--category") {
        n.set_field("category", &c);
    }
    if let Some(p) = flag(&rest, "--parent") {
        n.set_field("parent", &p);
        if !node_exists(&p) {
            eprintln!("warn: parent '{p}' doesn't exist (dangling)");
        }
    }
    n.set_field("title", &title);
    if let Some(s) = flag(&rest, "--serves") {
        n.push_field("serves", &s);
        if !node_exists(&s) {
            eprintln!("warn: serves '{s}' doesn't exist (dangling)");
        }
    }
    save_node(&id, &n);
    println!("added {id}  ({role}/{subset})");
}

fn cmd_set() {
    let mut a = std::env::args().skip(2);
    let (Some(id), Some(key), Some(val)) = (a.next(), a.next(), a.next()) else {
        eprintln!("usage: arte set <id> <key> <value>");
        std::process::exit(2);
    };
    let mut n = load_or_exit(&id);
    // `contract` is multi-value with no dedicated append verb (serves has link, at has
    // arte at) — so set APPENDS it (dedup'd); clear with `unset <id> contract`.
    if key == "contract" {
        n.push_field(&key, &val);
    } else {
        n.set_field(&key, &val);
    }
    save_node(&id, &n);
    println!("set {id}.{key} = {val}");
}

fn cmd_status() {
    let mut a = std::env::args().skip(2);
    let (Some(id), Some(st)) = (a.next(), a.next()) else {
        eprintln!("usage: arte status <id> <ok|ko|pending|justified>");
        std::process::exit(2);
    };
    let mut n = load_or_exit(&id);
    n.set_field("status", &st);
    save_node(&id, &n);
    println!("status {id} = {st}");
}

fn cmd_link() {
    let mut a = std::env::args().skip(2);
    let (Some(id), Some(target)) = (a.next(), a.next()) else {
        eprintln!("usage: arte link <id> <serves-id>");
        std::process::exit(2);
    };
    let mut n = load_or_exit(&id);
    n.push_field("serves", &target);
    if !node_exists(&target) {
        eprintln!("warn: '{target}' doesn't exist (dangling link)");
    }
    save_node(&id, &n);
    println!("linked {id} ↑ {target}");
}

/// Nodes that reference `id` via `serves` or `parent` (would dangle if it's deleted).
fn dependents(id: &str) -> Vec<String> {
    all_nodes()
        .into_iter()
        .filter(|(nid, n)| nid != id && (n.all("serves").contains(&id) || n.get("parent") == Some(id)))
        .map(|(nid, _)| nid)
        .collect()
}

fn cmd_delete() {
    let mut a = std::env::args().skip(2);
    let Some(id) = a.next() else {
        eprintln!("usage: arte delete <id>");
        std::process::exit(2);
    };
    if !node_exists(&id) {
        eprintln!("no node '{id}'");
        std::process::exit(2);
    }
    let deps = dependents(&id); // warn, but don't cascade-delete (that's destructive)
    if let Err(e) = fs::remove_file(node_path(&id)) {
        eprintln!("error deleting {id}: {e}");
        std::process::exit(1);
    }
    println!("deleted {id}");
    if !deps.is_empty() {
        eprintln!("warn: {} node(s) now dangle → {} (relink or delete them)", deps.len(), deps.join(", "));
    }
}

fn cmd_unset() {
    let mut a = std::env::args().skip(2);
    let (Some(id), Some(key)) = (a.next(), a.next()) else {
        eprintln!("usage: arte unset <id> <key>   (clears a field: status|parent|category|serves|at|note|…)");
        std::process::exit(2);
    };
    let mut n = load_or_exit(&id);
    if n.unset_field(&key) {
        save_node(&id, &n);
        println!("unset {id}.{key}");
    } else {
        eprintln!("{id} has no '{key}'");
    }
}

fn cmd_unlink() {
    let mut a = std::env::args().skip(2);
    let (Some(id), Some(target)) = (a.next(), a.next()) else {
        eprintln!("usage: arte unlink <id> <serves-id>");
        std::process::exit(2);
    };
    let mut n = load_or_exit(&id);
    if n.remove_value("serves", &target) {
        save_node(&id, &n);
        println!("unlinked {id} ↑ {target}");
    } else {
        eprintln!("{id} does not serve {target}");
    }
}

// ---- focus: the "declare what you're building" signal (drives the visualiser
// pulse + the edit gate). Stored in `.truth/.focus`, one node id per line. ----
fn focus_path() -> String {
    format!("{TRUTH_DIR}/.focus")
}
fn read_focus() -> Vec<String> {
    fs::read_to_string(focus_path())
        .ok()
        .map(|s| s.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect())
        .unwrap_or_default()
}

/// `arte working <id>...` declare focus · `arte working` show · `arte working clear`.
fn cmd_working() {
    let mut a = std::env::args().skip(2);
    match a.next().as_deref() {
        None => {
            let f = read_focus();
            if f.is_empty() {
                println!("no focus set. declare before editing code: arte working <id>");
            } else {
                println!("▶ working on: {}", f.join(", "));
            }
        }
        Some("clear") | Some("off") | Some("none") => {
            let _ = fs::remove_file(focus_path());
            println!("focus cleared");
        }
        Some(first) => {
            let ids: Vec<String> = std::iter::once(first.to_string()).chain(a).collect();
            for i in &ids {
                if !node_exists(i) {
                    eprintln!("no node '{i}' — map it on the board first (arte add ...)");
                    std::process::exit(2);
                }
            }
            let _ = fs::write(focus_path(), ids.join("\n") + "\n");
            println!("▶ working on: {}  (pulsing in the visualiser)", ids.join(", "));
        }
    }
}

/// `arte at <id> <file#unit>` — append a down-link to the artifact (dedup). The
/// edit gate calls this to auto-stamp the file being edited onto the focus node.
fn cmd_at() {
    let mut a = std::env::args().skip(2);
    let (Some(id), Some(loc)) = (a.next(), a.next()) else {
        eprintln!("usage: arte at <id> <file#unit>");
        std::process::exit(2);
    };
    let mut n = load_or_exit(&id);
    n.push_field("at", &loc);
    save_node(&id, &n);
    println!("at {id} → {loc}");
}

// ---- derived queries: compute answers from the graph (not stored) ----
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
    if rank(best) > 0 {
        best.clone()
    } else {
        None
    }
}
fn trunc(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        format!("{s:<n$}")
    } else {
        s.chars().take(n - 1).collect::<String>() + "…"
    }
}

/// Ancestors of a node, walking `serves` UP transitively (the ids it realizes).
fn closure_up(start: &Node, by_id: &HashMap<&str, &Node>) -> HashSet<String> {
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

/// coverage: per intent, is it realized in every downstream layer? + status roll-up.
fn cmd_coverage() {
    coverage_pass();
}

/// The coverage report, returning (intents, gaps, control-holes) so `arte gate`
/// can judge what this prints.
fn coverage_pass() -> (usize, usize, usize) {
    let conf = read_conf();
    let nodes = all_nodes(); // already sorted by id → stable output
    let by_id: HashMap<&str, &Node> = nodes.iter().map(|(id, n)| (id.as_str(), n)).collect();
    let closures: Vec<(&Node, HashSet<String>)> = nodes.iter().map(|(_, n)| (n, closure_up(n, &by_id))).collect();
    let Some(spine) = conf.chain.first().map(|s| s.as_str()) else { return (0, 0, 0) };
    let vrole = conf.chain.last().map(|s| s.as_str()).unwrap_or("validation");
    let downstream: Vec<String> = conf.chain.iter().skip(1).cloned().collect();
    let intents: Vec<(&str, &Node)> = nodes.iter().filter(|(_, n)| n.get("role") == Some(spine)).map(|(id, n)| (id.as_str(), n)).collect();
    let (mut covered, mut verified, mut gaps) = (0usize, 0usize, 0usize);
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
        // verified = a covering validation that is test-backed (`at:` → *.test.*) AND green
        let is_verified = closures.iter().any(|(n, cl)| {
            n.get("role") == Some(vrole)
                && cl.contains(iid)
                && n.get("status") == Some("ok")
                && n.all("at").iter().any(|a| a.contains(".test."))
        });
        let mark = if is_verified { "⚡verified" } else if whole { "·unproven" } else { "" };
        let chip = rollup(&statuses).map(|s| format!("[{s}]")).unwrap_or_else(|| "[—]".into());
        println!("  {} {}  {chip} {mark}", trunc(intent.get("title").unwrap_or(""), 30), cells.join(" "));
        if whole {
            covered += 1;
        } else {
            gaps += 1;
        }
        if is_verified {
            verified += 1;
        }
    }
    println!("  ──");
    println!("  {} intents · {covered} covered · {verified} VERIFIED (test-backed+green) · {gaps} gap(s)", intents.len());

    // controls with NO validation proving them — the guide's "reproduction hole",
    // now reported (a shallow board can't hide behind a covered intent).
    let crole = if conf.chain.len() >= 2 { conf.chain[conf.chain.len() - 2].clone() } else { "control".into() };
    let holes: Vec<&str> = nodes
        .iter()
        .filter(|(id, n)| {
            n.get("role") == Some(crole.as_str())
                && !closures.iter().any(|(vn, cl)| vn.get("role") == Some(vrole) && cl.contains(id.as_str()))
        })
        .map(|(id, _)| id.as_str())
        .collect();
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
    (intents.len(), gaps, holes.len())
}

/// trace: walk `serves` UP from a node to the requirement(s) it satisfies.
fn cmd_trace() {
    let Some(id) = std::env::args().nth(2) else {
        eprintln!("usage: arte trace <id>");
        std::process::exit(2);
    };
    let nodes = all_nodes();
    let by_id: HashMap<&str, &Node> = nodes.iter().map(|(id, n)| (id.as_str(), n)).collect();
    if !by_id.contains_key(id.as_str()) {
        eprintln!("no node '{id}'");
        std::process::exit(2);
    }
    trace_up(&id, &by_id, 0);
}
fn trace_up(id: &str, by_id: &HashMap<&str, &Node>, depth: usize) {
    let pad = "  ".repeat(depth);
    let arrow = if depth == 0 { "" } else { "↑ " };
    match by_id.get(id) {
        Some(n) => {
            let st = n.get("status").map(|s| format!("  [{s}]")).unwrap_or_default();
            println!("{pad}{arrow}{id}  ({}){st}  {}", n.get("role").unwrap_or("?"), n.get("title").unwrap_or(""));
            for s in n.all("serves") {
                trace_up(s, by_id, depth + 1);
            }
        }
        None => println!("{pad}{arrow}{id}  ⚠(missing)"),
    }
}

// ---- implement: the productized specifier↔implementer loop. `arte implement
// [--watch]` fires a headless implementer agent to drive the test suite green
// whenever the board or the specs change. The specifier writes tests; this keeps
// the implementation chasing them. ----
const IMPLEMENTER_BRIEF: &str = "You are the IMPLEMENTER in a spec-driven workflow. Drive this project's test suite to FULLY GREEN by writing code under src/ that satisfies the failing tests, obeying the rules in .truth/c-*.node (the controls). If the test command already passes, change nothing and report 'already green'. HARD RULES: never edit .truth/ (the board) or test/ (the specs); add no dependencies; no backend/network; minimal diffs. Read the failing tests and the relevant controls, implement, run the tests, iterate until green, then report exactly what you changed.";

/// `[implement]` from arte.toml — test command + implementer agent (with defaults).
fn impl_conf() -> (String, String) {
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

/// Fingerprint of the SPEC sources (`.truth/` + `test/`) — not `src/`, so the
/// implementer's own edits never re-trigger it (no feedback loop).
fn impl_fingerprint() -> u64 {
    use std::hash::{Hash, Hasher};
    fn walk(dir: &str, out: &mut Vec<(String, u64, u64)>) {
        let Ok(rd) = fs::read_dir(dir) else { return };
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
    walk(TRUTH_DIR, &mut entries);
    walk("test", &mut entries);
    entries.sort();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    entries.hash(&mut h);
    h.finish()
}

fn implement_pass(test: &str, agent: &str) {
    use std::process::Command;
    println!("• implementer working  ({agent})");
    let parts: Vec<&str> = agent.split_whitespace().collect();
    if let Some((bin, args)) = parts.split_first() {
        let mut argv: Vec<&str> = args.to_vec();
        argv.push(IMPLEMENTER_BRIEF);
        // the implementer runs ISOLATED — it physically cannot write .truth/ or test/
        let _ = run_isolated(role_deny_dirs("implementer"), bin, &argv);
    }
    println!("• verifying  ({test})");
    let green = Command::new("sh").arg("-c").arg(test).status().map(|s| s.success()).unwrap_or(false);
    println!("{}", if green { "✓ all tests green" } else { "✗ not green — a spec still needs implementing" });
}

/// `arte verify` — run each validation's OWN test and DERIVE that validation's
/// status from IT, not from whole-suite green. A validation whose `at:` test file
/// is missing is flagged a HOLE, never blessed. Per-test, not per-suite.
fn cmd_verify() {
    let (_, ko, holes) = verify_pass();
    if ko > 0 || holes > 0 {
        std::process::exit(1);
    }
}

/// The verify run, returning (ok, ko, missing-test holes) so `arte gate` can
/// judge what this prints. Writes each validation's measured status back.
fn verify_pass() -> (usize, usize, usize) {
    use std::collections::HashMap;
    use std::process::Command;
    let (test, _) = impl_conf();
    let nodes = all_nodes();
    // each validation → the test file it points at (`at:` ending in a *.test.* / test/ path)
    let vals: Vec<(String, String)> = nodes
        .iter()
        .filter(|(_, n)| n.get("role") == Some("validation"))
        .filter_map(|(id, n)| {
            n.all("at")
                .into_iter()
                .find(|a| a.contains(".test.") || a.starts_with("test/"))
                .map(|a| (id.clone(), a.split('#').next().unwrap_or(a).to_string()))
        })
        .collect();
    if vals.is_empty() {
        println!("no test-backed validations yet — write a test, then `arte at <validation> <test-file>`");
        return (0, 0, 0);
    }
    // run each UNIQUE test file once; a validation's status comes from ITS file
    let mut file_pass: HashMap<String, Option<bool>> = HashMap::new(); // None = file missing
    for (_, f) in &vals {
        if file_pass.contains_key(f) {
            continue;
        }
        let res = if Path::new(f).exists() {
            println!("• {test} {f}");
            Some(Command::new("sh").arg("-c").arg(format!("{test} {f}")).status().map(|s| s.success()).unwrap_or(false))
        } else {
            None
        };
        file_pass.insert(f.clone(), res);
    }
    let (mut ok, mut ko, mut holes) = (0usize, 0usize, 0usize);
    for (id, f) in &vals {
        match file_pass.get(f).copied().flatten() {
            Some(true) => {
                let mut n = load_or_exit(id);
                n.set_field("status", "ok");
                save_node(id, &n);
                ok += 1;
            }
            Some(false) => {
                let mut n = load_or_exit(id);
                n.set_field("status", "ko");
                save_node(id, &n);
                ko += 1;
            }
            None => {
                println!("  ⚠ {id}: at: {f} — no such test file (a validation with no real test)");
                holes += 1;
            }
        }
    }
    println!("verify: {ok} ok · {ko} ko · {holes} missing-test hole(s)  (per test, not whole-suite)");
    (ok, ko, holes)
}

/// `arte contract` — the NARROW interface pin (review #3). An impl may declare
/// `contract:` lines (public exports/signatures). This checks each declared symbol
/// actually appears in the impl's source `at:` — so a reproduction that renames the
/// public API is CAUGHT (the AlphaFlow `diagram`→`state` divergence, made loud).
fn cmd_contract() {
    if contract_pass() > 0 {
        std::process::exit(1);
    }
}

/// The contract check, returning the violation count so `arte gate` can judge it.
fn contract_pass() -> usize {
    let (mut checked, mut violations) = (0usize, 0usize);
    for (id, n) in all_nodes() {
        if n.get("role") != Some("impl") {
            continue;
        }
        let contracts = n.all("contract");
        if contracts.is_empty() {
            continue;
        }
        let srcs: Vec<String> = n.all("at").into_iter().filter(|a| !a.contains(".test.") && !a.starts_with("test/")).map(String::from).collect();
        if srcs.is_empty() {
            println!("  ⚠ {id}: declares a contract but has no source `at:`");
            violations += 1;
            continue;
        }
        let body: String = srcs.iter().filter_map(|s| fs::read_to_string(s).ok()).collect::<Vec<_>>().join("\n");
        for c in &contracts {
            // the symbol = leading identifier before ( : space < — the public name to pin
            let sym = c.split(|ch: char| ch == '(' || ch == ':' || ch == ' ' || ch == '<').next().unwrap_or("").trim();
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

/// Trace hygiene (gate-only): (1) DANGLING `at:` — a board pointer at a file that
/// no longer exists (stale trace, otherwise silent); (2) ORPHAN src files — code
/// no node claims via `at:` (unspecced work; the inverse discipline hole: gate
/// proves everything on the board, this proves everything in the code is ON it).
/// Returns (dangling, orphans).
fn trace_pass() -> (usize, usize) {
    let nodes = all_nodes();
    let claimed: HashSet<String> =
        nodes.iter().flat_map(|(_, n)| n.all("at").into_iter().map(|a| a.split('#').next().unwrap_or(a).to_string())).collect();
    let mut dangling = 0usize;
    for (id, n) in &nodes {
        for a in n.all("at") {
            let p = a.split('#').next().unwrap_or(a);
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
            if p.is_dir() {
                walk(&p.to_string_lossy(), out);
            } else {
                out.push(p.to_string_lossy().to_string());
            }
        }
    }
    let mut srcs = Vec::new();
    walk("src", &mut srcs);
    srcs.sort();
    // an `at:` may claim a DIRECTORY (`at: src/assets` claims the subtree) — the
    // board decides claiming granularity, the gate doesn't force file-level bureaucracy
    let orphans: Vec<&String> =
        srcs.iter().filter(|f| !claimed.contains(*f) && !claimed.iter().any(|c| f.starts_with(&format!("{c}/")))).collect();
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

/// `arte gate` — the merge gate: coverage + verify + contract + trace in one
/// command. Exit 0 ONLY when the board is whole (every intent realized in every
/// layer, every control validated), every validation's own test is green, the code
/// exposes its declared contracts, and the trace is bidirectionally sound (no
/// dangling at:, no unclaimed src). Install as a CI check / pre-merge hook:
/// enforcement moves from write time (agent compliance) to accept time.
fn cmd_gate() {
    println!("── arte gate · coverage ──");
    let (intents, gaps, choles) = coverage_pass();
    println!("── arte gate · verify ──");
    let (ok, ko, vholes) = verify_pass();
    println!("── arte gate · contract ──");
    let violations = contract_pass();
    println!("── arte gate · trace ──");
    let (dangling, orphans) = trace_pass();

    let mut fails: Vec<String> = Vec::new();
    if intents == 0 {
        fails.push("no intents on the board — nothing to gate against".into());
    }
    if gaps > 0 {
        fails.push(format!("{gaps} intent(s) not realized in every layer"));
    }
    if choles > 0 {
        fails.push(format!("{choles} control(s) with NO validation (reproduction holes)"));
    }
    if ok == 0 && intents > 0 {
        fails.push("no green test-backed validation — nothing is measured".into());
    }
    if ko > 0 {
        fails.push(format!("{ko} validation(s) red"));
    }
    if vholes > 0 {
        fails.push(format!("{vholes} validation(s) whose test file is missing"));
    }
    if violations > 0 {
        fails.push(format!("{violations} contract violation(s)"));
    }
    if dangling > 0 {
        fails.push(format!("{dangling} dangling at: pointer(s) — the board points at missing files"));
    }
    if orphans > 0 {
        fails.push(format!("{orphans} orphan src file(s) — code no node claims (unspecced work)"));
    }
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

/// `arte view [dir]` — launch the optional visualiser (arte-tui) on this board.
/// arte stays viewer-agnostic: this just delegates to the `arte-tui` binary if
/// installed, so the two ship together but neither requires the other.
fn cmd_view() {
    use std::process::Command;
    let dir = std::env::args().nth(2).unwrap_or_else(|| ".".into());
    match Command::new("arte-tui").arg("arte").arg(&dir).status() {
        Ok(s) => std::process::exit(s.code().unwrap_or(0)),
        Err(_) => {
            eprintln!("viewer not installed — `arte view` delegates to the arte-tui binary.");
            eprintln!("get it: cargo install --path viewer/tui  (from the arte repo, or the release bundle)");
            std::process::exit(2);
        }
    }
}

/// Per-role write isolation (review #2): run a command that PHYSICALLY cannot
/// write a role's forbidden dirs — separation of powers as an OS property, not a
/// prose brief. macOS `sandbox-exec`; warns + runs unisolated if unavailable.
fn role_deny_dirs(role: &str) -> &'static [&'static str] {
    match role {
        "implementer" | "impl" => &[".truth", "test"],
        "test-author" | "tester" | "adversary" => &["src"],
        "specifier" | "spec" => &["src", "test"],
        "qa" => &["src", "test", ".truth"], // read-only reviewer
        _ => &[],
    }
}
fn sandbox_profile(deny_dirs: &[&str]) -> Option<String> {
    let cwd = std::env::current_dir().ok()?;
    let subs: String = deny_dirs.iter().map(|d| format!("(subpath \"{}\")", cwd.join(d).display())).collect::<Vec<_>>().join(" ");
    Some(format!("(version 1)(allow default)(deny file-write* {subs})"))
}
fn run_isolated(deny_dirs: &[&str], program: &str, args: &[&str]) -> std::io::Result<std::process::ExitStatus> {
    run_isolated_as(None, deny_dirs, program, args)
}

/// Like run_isolated, but also announces the role to the child via ARTE_ROLE —
/// so an agent launched inside `arte role X -- <agent>` KNOWS its lane instead of
/// discovering it by hitting `Operation not permitted`.
fn run_isolated_as(role: Option<&str>, deny_dirs: &[&str], program: &str, args: &[&str]) -> std::io::Result<std::process::ExitStatus> {
    use std::process::Command;
    if let Some(profile) = sandbox_profile(deny_dirs) {
        let mut c = Command::new("sandbox-exec");
        c.arg("-p").arg(&profile).arg(program).args(args);
        if let Some(r) = role {
            c.env("ARTE_ROLE", r);
        }
        if let Ok(s) = c.status() {
            return Ok(s);
        }
    }
    eprintln!("warn: sandbox-exec unavailable — running WITHOUT capability isolation");
    let mut c = Command::new(program);
    c.args(args);
    if let Some(r) = role {
        c.env("ARTE_ROLE", r);
    }
    c.status()
}

/// `arte role <role> -- <command...>` — run a command under that role's write
/// isolation (e.g. `arte role implementer -- <agent>` cannot touch test/ or .truth/).
fn cmd_role() {
    let args: Vec<String> = std::env::args().skip(2).collect();
    let (Some(role), Some(sep)) = (args.first().cloned(), args.iter().position(|a| a == "--")) else {
        eprintln!("usage: arte role <implementer|test-author|specifier|qa> -- <command...>");
        std::process::exit(2);
    };
    let deny = role_deny_dirs(&role);
    if deny.is_empty() {
        eprintln!("unknown role '{role}' (implementer | test-author | specifier | qa)");
        std::process::exit(2);
    }
    let cmd = args[sep + 1..].join(" ");
    let code = run_isolated_as(Some(&role), deny, "sh", &["-c", &cmd]).ok().and_then(|s| s.code()).unwrap_or(1);
    std::process::exit(code);
}

fn cmd_implement() {
    let watch = std::env::args().any(|a| a == "--watch");
    let (test, agent) = impl_conf();
    if !watch {
        implement_pass(&test, &agent);
        return;
    }
    println!("arte implement --watch — watching {TRUTH_DIR}/ + test/ (ctrl-c to stop)");
    implement_pass(&test, &agent);
    let mut last = impl_fingerprint();
    loop {
        std::thread::sleep(std::time::Duration::from_secs(2));
        let now = impl_fingerprint();
        if now != last {
            last = now;
            println!("\n▶ spec changed — implementer running");
            implement_pass(&test, &agent);
        }
    }
}

// Templates: presets for chain + [subsets] + [subset_axis]. The structure (axes,
// their per-layer names + values) is the only thing that varies by domain — so a
// project picks a template and replaces it with its own framings as it grows.
fn template_toml(name: &str) -> Option<&'static str> {
    Some(match name {
        "software" => SOFTWARE_TOML,
        "hardware" => HARDWARE_TOML,
        "generic" => GENERIC_TOML,
        "arte" => ARTE_TOML, // arte's own dogfood structure
        _ => return None,
    })
}

/// Where saved (user) templates live. A template is just an `arte.toml`.
fn templates_dir() -> std::path::PathBuf {
    std::path::Path::new(&std::env::var("HOME").unwrap_or_else(|_| ".".into())).join(".arte/templates")
}

/// Resolve a template by name: built-in first, then a saved `~/.arte/templates/<name>.toml`.
fn resolve_template(name: &str) -> Option<String> {
    if let Some(t) = template_toml(name) {
        return Some(t.to_string());
    }
    fs::read_to_string(templates_dir().join(format!("{name}.toml"))).ok()
}

/// `arte template save <name> | list` — save the current arte.toml as a reusable
/// template, or list what's available (built-in + saved).
fn template_cmd() {
    let mut args = std::env::args().skip(2);
    match args.next().as_deref() {
        Some("save") => {
            let Some(name) = args.next() else {
                eprintln!("usage: arte template save <name>");
                std::process::exit(2);
            };
            let Ok(src) = fs::read_to_string("arte.toml") else {
                eprintln!("no arte.toml here — run `arte init` first");
                std::process::exit(2);
            };
            let dir = templates_dir();
            fs::create_dir_all(&dir).expect("create templates dir");
            let path = dir.join(format!("{name}.toml"));
            fs::write(&path, src).expect("write template");
            println!("saved template '{name}' → {}", path.display());
        }
        Some("list") => {
            println!("built-in: software, hardware, generic, arte");
            if let Ok(rd) = fs::read_dir(templates_dir()) {
                let mut saved: Vec<String> = rd.flatten().filter_map(|e| e.path().file_stem().map(|s| s.to_string_lossy().to_string())).collect();
                saved.sort();
                if !saved.is_empty() {
                    println!("saved:    {}", saved.join(", "));
                }
            }
        }
        _ => eprintln!("usage: arte template save <name> | list"),
    }
}

const SOFTWARE_TOML: &str = "# arte — software project\n\
chain = [\"intent\", \"impl\", \"control\", \"validation\"]\n\n\
[subsets]\n\
intent = [\"who\", \"jobs\", \"why\", \"features\", \"constraints\", \"success\"]\n\
impl = [\"frontend\", \"backend\", \"database\", \"infra\"]\n\
control = [\"security\", \"limit\", \"ux\", \"interaction\", \"validation\", \"TBD\"]\n\
validation = [\"unit\", \"integration\", \"e2e\", \"qa\"]\n\n\
[subset_axis]\n\
intent = \"framing\"\n\
impl = \"component\"\n\
control = \"subset\"\n\
validation = \"proof\"\n";

/// The whole how-to-use-arte, for an agent's first contact. `arte guide`.
const AGENT_GUIDE: &str = r#"arte — a design-truth board. You specify + trace software as a graph, so any
agent (or human) can see what exists, why, and whether it still holds.

THE MODEL — four layers, linked bottom-up by `serves` (impl serves intent, etc.):
  intent      what & why            (who · jobs · features · constraints · success)
  impl        the components that realize intents  (put the HOW in `note`)
  control     the RULE an impl must obey — a CHECKABLE criterion, not prose.
              "muted" is not a control; "colour saturation <= 0.25" is.
  validation  a RUNNABLE TEST that proves a control.  `at:` points to its test file.
  Identity = the node id (filename). Links are by id. `title` is a renamable label.

THE LOOP (do this, in order):
  1. MAP FIRST — put intent -> impl -> control -> validation on the board BEFORE code.
  2. CONTROLS ARE CRITERIA — each control states one testable condition.
  3. VALIDATIONS ARE TESTS — write the test, `arte at <validation> <test-file>`.
  4. STATUS IS MEASURED — `arte verify` runs the tests and sets validation status.
     Never hand-assert "it works".
  5. `arte coverage` shows VERIFIED (test-backed + green), not just covered.

BE DETAILED — this is where boards fail and reproductions diverge:
  - decompose components to their BEHAVIOUR. for software, one control per
    INTERACTION:  "<event> on <target> -> <observable state change>".
    cover pointer, drag, wheel AND keyboard (keydown on the document, GUARDED when a
    text field is focused).  e.g. pointerdown empty + move -> pan;  keydown Delete -> remove selection.
  - declare the public CONTRACT as `contract:` lines on the impl (one export/signature
    each); `arte contract` checks the code exposes them, so a rename is CAUGHT.
  - a control with no checkable criterion, or no validation, is a reproduction hole.

LANE — arte grades BEHAVIOUR (runnable controls). Aesthetics / UX / PDF that no test can
  pin belong to a QA MILESTONE (a `qa`-subset validation the QA role signs off), not a
  pretend auto-test. `arte coverage` shows test-verified vs merely covered — keep that
  honesty; a green you can't defend is worse than an honest gap.

ROLES — separation of authority (no agent grades its own work):
  specifier    writes the spec: fills the board (intents, impls, controls). Decides WHAT.
  test-author  writes a FALSIFYING test per control -> validations. Independent grader;
               may NOT edit src/ or the controls (can't move the goalposts it scores).
  implementer  builds src/ to pass the tests. Decides only HOW;
               may NOT edit the tests or the board.
  QA           at each milestone, before delivery: runs the whole app (behavioural /
               E2E), checks what unit tests miss, accepts or rejects.
  The point: whoever sets the goalposts is never who scores against them. THAT — not a
  gate you can perform — is what makes the workflow hard to game.
  Enforce it, don't just trust it:  arte role <role> -- <command>  runs the command
  under OS write-isolation (implementer can't touch test/ or .truth/; test-author can't
  touch src/). `arte implement` already sandboxes its agent this way.
  SOLO AGENT? The roles become PHASES, same order, same boundaries: FIRST act as
  specifier (map the existing code into intents/impls/controls — read-only), THEN as
  test-author (pin current behaviour as validations BEFORE changing anything), only
  THEN as implementer (change src/ against those pins). Never skip a phase because
  you're alone — sequencing in time is what separation across agents degrades to.
  HAVE SUBAGENTS? (Claude Code Agent tool, or any harness that spawns workers):
  run the roles as SEPARATE subagents; you stay ORCHESTRATOR + specifier.
    · spawn a TEST-AUTHOR with authority over test/ + validation nodes ONLY: it turns
      each unproven control into a FAILING test and stamps `arte at`. A not-yet-built
      feature SHOULD be red — that red is the implementer's work order (the torch).
    · then spawn an IMPLEMENTER with authority over src/ ONLY: its whole spec is the
      board + the red tests; it stamps `arte at` on impls it realizes, nothing more.
    · AUDIT between hand-offs, never trust reports: check each lane's footprint
      (tester touched no src/; implementer touched no test/ or statuses), run
      `arte verify` YOURSELF to derive status, `arte gate` to judge the round.
    · disputes cross lanes THROUGH YOU: an implementer who believes a test is wrong
      argues it to the orchestrator; the test-author verifies independently and fixes
      only what it confirms. Nobody ever edits the artifact that grades them.

COMMANDS:
  read     arte            (front door: show the board / guide init)
           arte observe · arte coverage · arte trace <id>
           arte view [dir]       live board visualiser (optional; needs arte-tui)
  build    arte add <role> "title" [--subset S --serves ID --parent P]
           arte set <id> <k> <v> · arte link/unlink <id> <t> · arte delete <id>
           arte status <id> ok|ko|pending|justified
  focus    arte working <id>     declare what you're on (pulses in the visualiser)
  verify   arte verify           tests -> validation status (per test)
           arte contract         check impls expose their declared `contract:` symbols
           arte implement [--watch]   fire a headless agent to drive tests green
  gate     arte gate             coverage + verify + contract; exit 1 on ANY hole.
                                 install as the CI / pre-merge check — what's not
                                 on the board (and green) doesn't merge
  isolate  arte role <role> -- <cmd>  run <cmd> under a role's write-isolation

START: run `arte observe` to read the board, then MAP your work before building it.
  Your role: if the ARTE_ROLE env var is set (you were launched via `arte role X --`),
  that is your lane — observe announces it. If it is NOT set, you enter as the
  SPECIFIER (phase one): map first, and hand testing/implementation to their roles
  (subagents if you have them, later phases if you don't).
"#;

/// Software-specific filling discipline — printed on `init --template software`.
/// This is the SOFTWARE domain's answer to \"how detailed must the board be\";
/// it stays OUT of the agnostic core (hardware/generic decompose differently).
const SOFTWARE_GUIDE: &str = "\nfilling guide (software) — decompose to BEHAVIOR or reproductions diverge:\n\
  • break each frontend component (impl) into INTERACTIONS via `parent` — one\n\
    control each, subset `interaction`:  \"<event> on <target> → <observable state change>\".\n\
    cover pointer, drag, wheel AND keyboard (keydown on the document, GUARDED when a\n\
    field is focused).  e.g. pointerdown empty + move → pan;  keydown Delete → remove selection.\n\
  • give every interaction control a BEHAVIORAL validation (fire the event, assert\n\
    the change) — needs a DOM/browser harness (@vue/test-utils or Playwright).\n\
  • state the public CONTRACT (key exports + signatures) in each impl node's `note`.\n\
  • turn styling into MEASURABLE controls (colour saturation, px sizes, contrast).\n\
  a control with no checkable criterion, or no validation, is a reproduction hole.\n";

const HARDWARE_TOML: &str = "# arte — hardware project\n\
chain = [\"requirement\", \"design\", \"control\", \"verification\"]\n\n\
[subsets]\n\
requirement = [\"user\", \"functional\", \"safety\", \"interface\"]\n\
design = [\"power\", \"mcu\", \"io\", \"rf\", \"mechanical\"]\n\
control = [\"electrical\", \"thermal\", \"emc\", \"TBD\"]\n\
verification = [\"bench\", \"environmental\", \"compliance\"]\n\n\
[subset_axis]\n\
requirement = \"framing\"\n\
design = \"block\"\n\
control = \"constraint\"\n\
verification = \"proof\"\n";

const GENERIC_TOML: &str = "# arte — generic project (rename subsets for your domain)\n\
chain = [\"intent\", \"impl\", \"control\", \"validation\"]\n\n\
[subsets]\n\
intent = [\"TBD\"]\n\
impl = [\"TBD\"]\n\
control = [\"TBD\"]\n\
validation = [\"TBD\"]\n\n\
[subset_axis]\n\
intent = \"framing\"\n\
impl = \"component\"\n\
control = \"subset\"\n\
validation = \"proof\"\n";

const ARTE_TOML: &str = "# arte — its own design truth (dogfood)\n\
chain = [\"intent\", \"impl\", \"control\", \"validation\"]\n\n\
[subsets]\n\
intent = [\"who\", \"jobs\", \"why\", \"features\", \"constraints\", \"success\"]\n\
impl = [\"core\", \"cli\", \"storage\"]\n\
control = [\"identity\", \"storage\", \"format\", \"agnostic\", \"TBD\"]\n\
validation = [\"proof\"]\n\n\
[subset_axis]\n\
intent = \"framing\"\n\
impl = \"component\"\n\
control = \"subset\"\n\
validation = \"proof\"\n";

/// arte's own nodes — only seeded by `--template arte` (dogfood); real projects start empty.
fn seed_arte() {
    write_node("i1", "intent", "who", "", "", "AI agents that build, and the humans who review them", &[]);
    write_node("i2", "intent", "why", "", "", "AI-built code is untraceable without a shared design contract", &[]);
    write_node("i3", "intent", "jobs", "", "", "Record why code exists and prove it still matches", &[]);
    write_node("i3a", "intent", "jobs", "", "i3", "Capture the intent behind each change", &[]);
    write_node("i3b", "intent", "jobs", "", "i3", "Prove the code still matches its intent", &[]);
    write_node("i4", "intent", "constraints", "", "", "Domain-agnostic: roles and subsets are declared, not hardcoded", &[]);
    write_node("m1", "impl", "storage", "", "", "Node reader/writer (.truth/<id>.node)", &["i3b"]);
    write_node("c1", "control", "identity", "", "", "A node is keyed by a stable id, never by its title", &["i3"]);
    write_node("c2", "control", "storage", "", "", "One node per file; git merges per node natively", &["i3"]);
    write_node("c3", "control", "format", "", "", "Line-oriented key:value, so git's line merge reconciles per field", &["i3"]);
    write_node("c4", "control", "agnostic", "", "", "Layers and subsets come from arte.toml, so the tool stays agnostic", &["i4"]);
}

fn init() {
    // pick the template (default: software — most projects)
    let mut args = std::env::args().skip(2);
    let mut tmpl = "software".to_string();
    while let Some(a) = args.next() {
        if a == "--template" {
            if let Some(v) = args.next() {
                tmpl = v;
            }
        }
    }
    let Some(toml) = resolve_template(&tmpl) else {
        eprintln!("unknown template '{tmpl}' — built-in: software|hardware|generic|arte, or save your own (arte template save <name>)");
        std::process::exit(2);
    };
    if !Path::new("arte.toml").exists() {
        let _ = fs::write("arte.toml", &toml);
        println!("• wrote arte.toml (template: {tmpl})");
    }
    fs::create_dir_all(TRUTH_DIR).expect("create .truth");
    let seeded = fs::read_dir(TRUTH_DIR).map(|d| d.flatten().any(|e| e.path().extension().map(|x| x == "node").unwrap_or(false))).unwrap_or(false);
    if tmpl == "arte" && !seeded {
        seed_arte();
        println!("• seeded arte's own design truth (11 nodes)");
    }
    if tmpl == "software" {
        print!("{SOFTWARE_GUIDE}");
    }
    // AGENTS.md — the cross-agent instruction file (Claude Code, Codex, … read it
    // on first contact). Generated from the SAME guide the binary prints, so it
    // can't drift; regenerated on every init. Skipped if the project already has
    // one that isn't ours (never clobber a human's file).
    let agents_hdr = "<!-- generated by `arte init` — do not edit; re-run init to refresh -->\n";
    let ours = fs::read_to_string("AGENTS.md").map(|s| s.starts_with(agents_hdr)).unwrap_or(true);
    if ours {
        let mut body = format!(
            "{agents_hdr}\n# This project runs on an arte design-truth board\n\n\
Before ANY work: run `arte observe` to read the board. The protocol below is\n\
binding — the CI gate (`arte gate`) fails work that skips it.\n\n\
SETUP: if `arte` is not on PATH, build it from the arte repo:\n\
  git clone <arte repo> && cd arte && cargo build --release --workspace\n\
  cp target/release/arte target/release/arte-tui ~/.local/bin/\n\
VIEWER (optional, for the human's split pane): `arte view` opens the live board\n\
TUI (delegates to the `arte-tui` binary; refreshes as you edit; `arte working <id>`\n\
makes the node you're on PULSE there — declare your focus).\n\n\
{AGENT_GUIDE}"
        );
        if tmpl == "software" {
            body.push_str(SOFTWARE_GUIDE);
        }
        let _ = fs::write("AGENTS.md", body);
        println!("• wrote AGENTS.md (first-contact protocol for any agent)");
    }
    println!("done. try: arte observe");
}

/// Empty-repo guide, emitted on the AGENT's read channel (`arte` / `arte observe`)
/// so an AI agent working the terminal is told exactly how to bootstrap — not left
/// staring at empty output. In the split-pane setup (visualiser in one pane, agent
/// prompting in the other) this is what actually reaches the agent, so the next
/// command it runs moves the work forward.
fn suggest_bootstrap() {
    println!("new to arte? run:  arte guide\n");
    if Path::new("arte.toml").exists() {
        // initialized but no nodes yet → start mapping the design
        println!("arte board initialized, no nodes yet.\n");
        println!("map the design — each layer serves the one above:");
        println!("  arte add intent \"<what & why>\"");
        println!("  arte add impl \"<how>\"          --serves <intent-id>");
        println!("  arte add control \"<rule/limit>\" --serves <impl-id>");
        println!("  arte add validation \"<proof>\"   --serves <control-id>");
        println!("\nthen: arte (view) · arte coverage (gaps) · arte trace <id>");
    } else {
        // no board at all → initialize one
        println!("no arte board here (.truth/ not found).\n");
        println!("initialize one:");
        println!("  arte init                        software (default)");
        println!("  arte init --template hardware|generic|arte");
        println!("\nthen run `arte` again — it'll guide the next step.");
    }
}

/// A node as displayed: identity comes from the FILENAME (the invariant), not content.
struct Disp {
    id: String,
    title: String,
    serves: Vec<String>,
    at: Vec<String>,
    parent: String,
    status: Option<String>,
}

/// Read the graph back: `.truth/*.node`, grouped by role (chain order) → subset,
/// rendering the decomposition tree (`parent`), `serves`/`at`, `status`, and
/// flagging dangling links (a `serves`/`parent` id that exists nowhere).
fn observe() {
    let conf = read_conf();
    // role banner: an agent inside `arte role X -- …` is TOLD its lane up front;
    // with NO role set, first contact defaults to SPECIFIER (phase one — map first).
    match std::env::var("ARTE_ROLE") {
        Ok(role) => {
            let deny = role_deny_dirs(&role);
            println!("▶ your role: {role}  (write-isolated from: {})\n", if deny.is_empty() { "—".into() } else { deny.join(", ") });
        }
        Err(_) => println!("▶ your role: specifier (default — no ARTE_ROLE set). Map the board first;\n  hand test-writing and src changes to their roles (subagents or later phases).\n"),
    }
    let nodes = all_nodes(); // shared loader: also emits the filename≠id warning
    if nodes.is_empty() {
        suggest_bootstrap(); // agent's read channel is empty → tell it how to move forward
        return;
    }
    let focus = read_focus();
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
        let cat = n.get("category").map(|c| format!(" ·{c}")).unwrap_or_default();
        g.entry(role).or_default().entry(subset).or_default().push(Disp {
            id: id.clone(),
            title: format!("{}{cat}", n.get("title").unwrap_or("")),
            serves: n.all("serves").iter().map(|s| s.to_string()).collect(),
            at: n.all("at").iter().map(|s| s.to_string()).collect(),
            parent: n.get("parent").unwrap_or("").to_string(),
            status: n.get("status").map(String::from),
        });
    }
    let mut roles: Vec<String> = conf.chain.clone();
    for r in g.keys() {
        if !roles.contains(r) {
            roles.push(r.clone());
        }
    }
    for role in roles {
        let Some(subsets) = g.get(&role) else { continue };
        let axis = conf.subset_axis.get(&role).map(|a| format!("  ({a})")).unwrap_or_default();
        println!("[{role}]{axis}");
        let mut order: Vec<String> = conf.subsets.get(&role).cloned().unwrap_or_default();
        for f in subsets.keys() {
            if !order.contains(f) {
                order.push(f.clone());
            }
        }
        for subset in order {
            let Some(items) = subsets.get(&subset) else { continue };
            println!("  {subset}/");
            let ids: std::collections::HashSet<&str> = items.iter().map(|d| d.id.as_str()).collect();
            // roots = no parent, or a parent outside this subset — sorted for stable output.
            let mut roots: Vec<&Disp> = items.iter().filter(|d| d.parent.is_empty() || !ids.contains(d.parent.as_str())).collect();
            roots.sort_by(|a, b| a.id.cmp(&b.id));
            for d in roots {
                print_node(d, items, &all_ids, 2);
            }
        }
    }
}

/// Print a node + recurse into its (sorted) children. Shows status, and flags any
/// dangling `serves`/`parent` (an id that exists nowhere) with ⚠.
fn print_node(d: &Disp, all: &[Disp], all_ids: &std::collections::HashSet<String>, depth: usize) {
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

#[derive(Default)]
struct Conf {
    chain: Vec<String>,
    subsets: BTreeMap<String, Vec<String>>,    // role -> framing values
    subset_axis: BTreeMap<String, String>,     // role -> what `subset` is called
}

/// Read arte.toml's fixed grammar correctly, zero-dep: `chain` + `[subsets]` +
/// `[subset_axis]`. Strips comments, detects sections by parsing the brackets
/// (not exact-match), tolerates whitespace. Single-line arrays only — which is
/// all arte writes; a multi-line hand-edit would need the real grammar.
fn read_conf() -> Conf {
    let mut c = Conf {
        chain: vec!["intent".into(), "impl".into(), "control".into(), "validation".into()],
        ..Default::default()
    };
    let Ok(text) = fs::read_to_string("arte.toml") else { return c };
    let mut section = String::new();
    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or("").trim(); // drop comments + trim
        if line.is_empty() {
            continue;
        }
        if let Some(inner) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            section = inner.trim().to_string();
            continue;
        }
        let Some((k, v)) = line.split_once('=') else { continue };
        let (k, v) = (k.trim(), v.trim());
        match section.as_str() {
            "" if k == "chain" => c.chain = parse_arr(v),
            "subsets" => {
                c.subsets.insert(k.to_string(), parse_arr(v));
            }
            "subset_axis" => {
                c.subset_axis.insert(k.to_string(), v.trim_matches('"').to_string());
            }
            _ => {}
        }
    }
    c
}

/// Parse a single-line TOML string array: `["a", "b"]` → ["a","b"].
fn parse_arr(v: &str) -> Vec<String> {
    v.trim().trim_start_matches('[').trim_end_matches(']').split(',').map(|x| x.trim().trim_matches('"').to_string()).filter(|x| !x.is_empty()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_preserves_unknown_and_multivalue() {
        let n = Node::parse("id: v1\nrole: validation\nserves: c1\nserves: c2\nweird: keep-me");
        assert_eq!(n.get("role"), Some("validation"));
        assert_eq!(n.all("serves"), vec!["c1", "c2"]); // repeated key → multi-value
        assert_eq!(n.get("weird"), Some("keep-me")); // unknown key round-trips
        // to_text is normalized `key: value`, re-parses identically (idempotent)
        assert_eq!(Node::parse(&n.to_text()).to_text(), n.to_text());
    }

    #[test]
    fn field_mutators() {
        let mut n = Node::parse("role: impl\nserves: a\nserves: b");
        n.set_field("role", "control"); // replace scalar in place
        assert_eq!(n.get("role"), Some("control"));
        n.push_field("serves", "a"); // dup skipped
        assert_eq!(n.all("serves").len(), 2);
        assert!(n.remove_value("serves", "a")); // remove one edge
        assert_eq!(n.all("serves"), vec!["b"]);
        assert!(n.unset_field("serves")); // clear the rest
        assert!(n.all("serves").is_empty());
        assert!(!n.unset_field("serves")); // nothing left → false
    }

    #[test]
    fn rollup_is_worst_wins() {
        let s = |v: &[Option<&str>]| rollup(&v.iter().map(|o| o.map(String::from)).collect::<Vec<_>>());
        assert_eq!(s(&[Some("ok"), Some("ko")]).as_deref(), Some("ko"));
        assert_eq!(s(&[Some("ok"), Some("pending")]).as_deref(), Some("pending"));
        assert_eq!(s(&[None, Some("ok")]).as_deref(), Some("ok")); // missing ignored
        assert_eq!(s(&[None, None]), None);
    }

    #[test]
    fn closure_walks_serves_transitively() {
        let nodes: Vec<(String, Node)> = [
            ("i", "role: intent"),
            ("m", "role: impl\nserves: i"),
            ("c", "role: control\nserves: m"),
            ("v", "role: validation\nserves: c"),
        ]
        .iter()
        .map(|(id, t)| (id.to_string(), Node::parse(t)))
        .collect();
        let by_id: HashMap<&str, &Node> = nodes.iter().map(|(id, n)| (id.as_str(), n)).collect();
        let cl = closure_up(by_id.get("v").copied().unwrap(), &by_id);
        assert!(cl.contains("c") && cl.contains("m") && cl.contains("i")); // whole chain
    }

    #[test]
    fn parse_arr_handles_quotes_and_spacing() {
        assert_eq!(parse_arr(r#"[ "a", "b" ,"c"]"#), vec!["a", "b", "c"]);
        assert!(parse_arr("[]").is_empty());
    }
}
