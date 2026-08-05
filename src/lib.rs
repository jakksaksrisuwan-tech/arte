//! arte — design-truth primitive, library.
//!
//! This is the canonical model + helpers. CLI dispatch and command
//! implementations live in `src/cli/` and `src/main.rs`. Anything that has to
//! stay in lockstep between the CLI and any future client (MCP, IDE, viewer)
//! lives here.
//!
//! The whole crate is zero-dep by design — see `Cargo.toml`. Add deps only when
//! a feature genuinely cannot be done in-process.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;

/// CLI command surface (each submodule owns one concern). `main.rs` matches on
/// argv and calls into these.
pub mod cli;

// ─── Board paths (env-overridable) ──────────────────────────────────────────

/// Defaults for the env-overridable board paths. Honoured by `read_env_dirs`.
pub const DEFAULT_TRUTH_DIR: &str = ".truth";
pub const DEFAULT_RUNS_DIR: &str = "qa/runs";
pub const DEFAULT_DISPATCH_PATH: &str = ".loop-dispatch";

/// `(truth, runs, dispatch)` resolved from the env with CWD fallback. c-arte-respects-ARTE_TRUTH_DIR-env-var
/// is the contract: every board read/write and dispatch emit must go through this
/// so the binary is hermetic under test isolation.
///
/// Semantics:
/// - `ARTE_TRUTH_DIR` is the CONCRETE truth directory. The binary reads and
///   writes node files directly under it. When unset, it uses `./.truth/`.
/// - `ARTE_RUNS_DIR` is the CONCRETE runs directory. When unset, it uses
///   `./qa/runs/`.
/// - `ARTE_DISPATCH_PATH` is the CONCRETE dispatch file path. When unset, it
///   uses `./.loop-dispatch`.
pub fn read_env_dirs() -> (String, String, String) {
    let truth = std::env::var("ARTE_TRUTH_DIR")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_TRUTH_DIR.to_string());
    let runs = std::env::var("ARTE_RUNS_DIR")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_RUNS_DIR.to_string());
    let dispatch = std::env::var("ARTE_DISPATCH_PATH")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_DISPATCH_PATH.to_string());
    (truth, runs, dispatch)
}

/// Path accessors — read every time (env may change between calls in test contexts).
pub fn truth_dir() -> String { read_env_dirs().0 }
pub fn runs_dir() -> String { read_env_dirs().1 }
pub fn dispatch_path() -> String { read_env_dirs().2 }

/// Stable-pass threshold: how many of the most recent N runs must pass before
/// `arte status <id> ok` is allowed without --force, and before `arte cycle`
/// auto-promotes. Matches the tetrahedron doc (2/3 of last 5).
pub const STABLE_PASS_REQUIRED: usize = 2;
pub const STABLE_PASS_WINDOW: usize = 5;

// ─── Node: the canonical graph element ──────────────────────────────────────

/// A node: ordered fields, multi-value via repeated keys. We keep it as a flat
/// list of (key, value) so unknown keys round-trip untouched (forward-compat).
#[derive(Debug, Clone)]
pub struct Node {
    pub fields: Vec<(String, String)>,
}

impl Node {
    pub fn get(&self, k: &str) -> Option<&str> {
        self.fields.iter().find(|(key, _)| key == k).map(|(_, v)| v.as_str())
    }
    pub fn all(&self, k: &str) -> Vec<&str> {
        self.fields.iter().filter(|(key, _)| key == k).map(|(_, v)| v.as_str()).collect()
    }
    /// Serialize: `key: value`, one per line. Stable order = stable diffs.
    pub fn to_text(&self) -> String {
        let mut s = String::new();
        for (k, v) in &self.fields {
            s.push_str(k);
            s.push_str(": ");
            s.push_str(v);
            s.push('\n');
        }
        s
    }
    pub fn parse(text: &str) -> Node {
        let fields = text
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
            .filter_map(|l| l.split_once(':').map(|(k, v)| (k.trim().to_string(), v.trim().to_string())))
            .collect();
        Node { fields }
    }
    /// Set a scalar field (replace first occurrence, else append). Preserves all
    /// other fields — round-trip safe, so unknown keys survive an edit.
    pub fn set_field(&mut self, k: &str, v: &str) {
        if let Some(f) = self.fields.iter_mut().find(|(key, _)| key == k) {
            f.1 = v.to_string();
        } else {
            self.fields.push((k.to_string(), v.to_string()));
        }
    }
    /// Append a multi-value field (e.g. serves), skipping an exact duplicate.
    pub fn push_field(&mut self, k: &str, v: &str) {
        if !self.fields.iter().any(|(kk, vv)| kk == k && vv == v) {
            self.fields.push((k.to_string(), v.to_string()));
        }
    }
    /// Remove every occurrence of a field (clear a scalar, or all of a multi-value).
    pub fn unset_field(&mut self, k: &str) -> bool {
        let before = self.fields.len();
        self.fields.retain(|(key, _)| key != k);
        self.fields.len() != before
    }
    /// Remove one specific (key, value) — e.g. a single `serves` link.
    pub fn remove_value(&mut self, k: &str, v: &str) -> bool {
        let before = self.fields.len();
        self.fields.retain(|(key, val)| !(key == k && val == v));
        self.fields.len() != before
    }
}

// ─── Board I/O ──────────────────────────────────────────────────────────────

pub fn node_path(id: &str) -> String {
    format!("{}/{}.node", truth_dir(), id)
}

pub fn write_node(id: &str, role: &str, subset: &str, parent: &str, title: &str, serves: &[&str]) {
    let mut fields = vec![
        ("id".into(), id.to_string()),
        ("role".into(), role.to_string()),
        ("subset".into(), subset.to_string()),
    ];
    if !parent.is_empty() {
        fields.push(("parent".into(), parent.to_string()));
    }
    fields.push(("title".into(), title.to_string()));
    for s in serves {
        fields.push(("serves".into(), s.to_string()));
    }
    let path = node_path(id);
    if let Some(p) = Path::new(&path).parent() {
        let _ = fs::create_dir_all(p);
    }
    if let Err(e) = fs::write(&path, Node { fields }.to_text()) {
        eprintln!("error writing {id}: {e}");
        std::process::exit(1);
    }
}

pub fn node_exists(id: &str) -> bool {
    Path::new(&node_path(id)).exists()
}
pub fn load_node(id: &str) -> Option<Node> {
    fs::read_to_string(node_path(id)).ok().map(|t| Node::parse(&t))
}
pub fn save_node(id: &str, n: &Node) {
    let path = node_path(id);
    if let Some(parent) = Path::new(&path).parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Err(e) = fs::write(&path, n.to_text()) {
        eprintln!("error writing {id}: {e}");
        std::process::exit(1);
    }
}
/// Load a node or exit(2) with the uniform "no node" error — the mutator prelude.
pub fn load_or_exit(id: &str) -> Node {
    load_node(id).unwrap_or_else(|| {
        eprintln!("no node '{id}'");
        std::process::exit(2);
    })
}
/// The one graph loader every reader shares: each `.truth/*.node` as
/// (filename-id, parsed node), sorted by id. Filename is the identity — warn
/// once if a stored `id:` disagrees. This is the only place we scan `.truth`.
pub fn all_nodes() -> Vec<(String, Node)> {
    let mut out = Vec::new();
    let Ok(dir) = fs::read_dir(truth_dir()) else { return out };
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

// ─── ID generation ──────────────────────────────────────────────────────────

pub fn slugify(s: &str) -> String {
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
pub fn gen_id(role: &str, title: &str) -> String {
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

// ─── Flag parsing ───────────────────────────────────────────────────────────

/// Return the value following `--name value` (or None if absent / last arg).
/// Designed for `--key value` style flags — does NOT work for boolean flags.
pub fn flag_value(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}
/// True if `--name` appears anywhere in args. Use for boolean flags.
pub fn has_flag(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}

// ─── Config: role chain, subsets, axis labels ───────────────────────────────

#[derive(Debug, Default)]
pub struct Conf {
    pub chain: Vec<String>,
    pub subsets: BTreeMap<String, Vec<String>>,
    pub subset_axis: BTreeMap<String, String>,
}

pub fn read_conf() -> Conf {
    let mut c = Conf::default();
    let Ok(txt) = fs::read_to_string("arte.toml") else { return c };
    let mut sect = String::new();
    for raw in txt.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if let Some(inner) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            sect = inner.trim().to_string();
        } else if let Some((k, v)) = line.split_once('=') {
            let (k, v) = (k.trim(), v.trim());
            if k == "chain" && sect.is_empty() {
                c.chain = parse_arr(v);
            } else if sect == "subsets" {
                c.subsets.insert(k.to_string(), parse_arr(v));
            } else if sect == "subset_axis" {
                c.subset_axis.insert(k.to_string(), v.to_string());
            }
        }
    }
    c
}
fn parse_arr(v: &str) -> Vec<String> {
    v.trim_matches(|c| c == '[' || c == ']')
        .split(',')
        .map(|x| x.trim().trim_matches('"').to_string())
        .filter(|x| !x.is_empty())
        .collect()
}

pub fn default_subset(role: &str) -> String {
    read_conf().subsets.get(role).and_then(|v| v.first()).cloned().unwrap_or_else(|| "TBD".into())
}

// ─── Run history ────────────────────────────────────────────────────────────

/// Run history — `qa/runs/<v-id>.<seq>.run`, one file per validation execution.
/// Line-oriented `key: value` (same as .truth/), so a project can write them by
/// hand and so we parse with zero new deps. Seq in the filename, not in the body;
/// lexicographic filename order = chronological order.
///
/// Per-run fields:
///   validation: <id>      // mandatory — also encoded in the filename
///   result: pass|fail     // mandatory
///   sha: <short>          // commit the test ran against
///   timestamp: <RFC3339>  // when it ran
///   note: <free>          // command, env, anything the runner wants to record
///
/// The dispatch manifest the cycle writes is `.loop-dispatch` in repo root:
///   next: spec|test|implement|none
///   target: <v-id>|—       // — when next=none
///   reason: <free>          // why this dispatch
#[derive(Debug, Clone)]
pub struct RunRec {
    pub seq: String,
    pub result: String,
    pub sha: String,
    pub timestamp: String,
    pub note: String,
}

pub fn load_runs(v_id: &str) -> Vec<RunRec> {
    let binding = runs_dir();
    let dir = Path::new(&binding);
    let Ok(rd) = fs::read_dir(dir) else { return Vec::new() };
    let prefix = format!("{v_id}.");
    let suffix = ".run";
    let mut out: Vec<RunRec> = rd
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            if !name.starts_with(&prefix) || !name.ends_with(suffix) { return None; }
            let seq = name.strip_prefix(&prefix)?.strip_suffix(suffix)?.to_string();
            let Ok(text) = fs::read_to_string(e.path()) else { return None };
            let n = Node::parse(&text);
            Some(RunRec {
                seq,
                result: n.get("result").unwrap_or("").to_string(),
                sha: n.get("sha").unwrap_or("").to_string(),
                timestamp: n.get("timestamp").unwrap_or("").to_string(),
                note: n.get("note").unwrap_or("").to_string(),
            })
        })
        .collect();
    out.sort_by(|a, b| b.seq.cmp(&a.seq)); // newest first
    out
}

/// Stable? Looks at the last STABLE_PASS_WINDOW runs; pass count must hit
/// STABLE_PASS_REQUIRED. Returns (passed_window, is_stable).
/// Has this validation ever been OBSERVED RED? A green that has never failed
/// is indistinguishable from a test that cannot fail — the red-first discipline
/// exists so every validation demonstrates it detects something. Derived from
/// run history, so it costs nothing to keep honest.
///
/// A validation with no history is unproven, never proven-by-default.
pub fn proven_detector(v_id: &str) -> bool {
    // The node stamp is the durable record (run history is pruned to the
    // stable-pass window, so a red ages out after a few greens); the history
    // scan still covers boards whose reds predate the stamp.
    if let Some(n) = load_node(v_id) {
        if n.get("proven").is_some() { return true; }
    }
    load_runs(v_id).iter().any(|r| r.result == "fail" || r.result == "ko")
}

pub fn stable_pass(runs: &[RunRec]) -> (usize, bool) {
    let window = &runs[..runs.len().min(STABLE_PASS_WINDOW)];
    let passed = window.iter().filter(|r| r.result == "pass").count();
    (passed, passed >= STABLE_PASS_REQUIRED)
}

pub fn write_run(v_id: &str, rec: &RunRec) -> std::io::Result<()> {
    let dir = runs_dir();
    fs::create_dir_all(&dir)?;
    let next = next_run_seq(v_id);
    let path = Path::new(&dir).join(format!("{v_id}.{next}.run"));
    let mut n = Node { fields: Vec::new() };
    n.push_field("validation", v_id);
    n.push_field("result", &rec.result);
    if !rec.sha.is_empty() { n.push_field("sha", &rec.sha); }
    if !rec.timestamp.is_empty() { n.push_field("timestamp", &rec.timestamp); }
    if !rec.note.is_empty() { n.push_field("note", &rec.note); }
    fs::write(&path, n.to_text())?;
    prune_runs(Path::new(&dir), STABLE_PASS_WINDOW);
    Ok(())
}

/// Bound each validation's run history to `keep` files, newest-first by
/// filename sequence. Called from `write_run` and the tail of `verify_pass`.
/// c-run-history-files-do-not-accumulate-unbounded.
pub fn prune_runs(runs_dir: &Path, keep: usize) {
    let Ok(entries) = runs_dir.read_dir() else { return };
    let mut by_validation: std::collections::HashMap<String, Vec<(u64, std::path::PathBuf)>> = std::collections::HashMap::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some(stem) = name.strip_suffix(".run") else { continue };
        let Some((validation, seq)) = stem.match_indices('.').find_map(|(dot, _)| {
            let seq = &stem[dot + 1..];
            (seq.len() >= 3 && seq.bytes().all(|b| b.is_ascii_digit()))
                .then(|| seq.parse::<u64>().ok().map(|n| (&stem[..dot], n)))
                .flatten()
        }) else { continue };
        by_validation.entry(validation.to_string()).or_default().push((seq, entry.path()));
    }
    for runs in by_validation.values_mut() {
        runs.sort_by(|a, b| b.0.cmp(&a.0));
        for (_, path) in runs.iter().skip(keep) {
            let _ = fs::remove_file(path);
        }
    }
}

pub fn next_run_seq(v_id: &str) -> String {
    let Ok(rd) = fs::read_dir(runs_dir()) else { return "001".into() };
    let prefix = format!("{v_id}.");
    let suffix = ".run";
    let mut max_n = 0u32;
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if !name.starts_with(&prefix) || !name.ends_with(suffix) { continue; }
        if let Some(mid) = name.strip_prefix(&prefix).and_then(|s| s.strip_suffix(suffix)) {
            if let Ok(n) = mid.parse::<u32>() { max_n = max_n.max(n); }
        }
    }
    format!("{:03}", max_n + 1)
}

/// Display-time truncator (used by mutators' near-duplicate warn and by query
/// coverage/trunc output). Lives in lib.rs so both modules share it. `n = 0`
/// yields the empty string (avoids the `n - 1` underflow trap on long input).
pub fn trunc(s: &str, n: usize) -> String {
    if n == 0 { return String::new(); }
    if s.chars().count() <= n {
        format!("{s:<n$}")
    } else {
        s.chars().take(n - 1).collect::<String>() + "…"
    }
}

// ─── Staleness envelope ─────────────────────────────────────────────────────

/// Current HEAD as a short sha, or None when not in a git repo.
pub fn git_short_head() -> Option<String> {
    std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Staleness envelope for one node. Returns None when: no `sha:` is recorded,
/// no git repo, the recorded sha equals HEAD, or HEAD is an ancestor of the
/// recorded sha (a rewind — we don't surface "you're behind yourself").
pub fn staleness(n: &Node, ahead: &mut HashMap<String, i64>) -> Option<String> {
    let recorded = n.get("sha")?;
    let head = git_short_head()?;
    if recorded == head { return None; }
    let range = format!("{recorded}..HEAD");
    let ahead_n = *ahead.entry(recorded.to_string()).or_insert_with(|| {
        std::process::Command::new("git")
            .args(["rev-list", "--count", &range])
            .output()
            .ok()
            .and_then(|o| o.status.success().then_some(o.stdout))
            .and_then(|b| String::from_utf8(b).ok())
            .and_then(|s| s.trim().parse::<i64>().ok())
            .unwrap_or(0)
    });
    if ahead_n <= 0 { return None; }
    Some(format!("⏱  stale: verified at {recorded}, you're {ahead_n} commit(s) ahead\n    (re-run `arte verify` to refresh)"))
}

// ─── Time ───────────────────────────────────────────────────────────────────

/// Cheap RFC3339-ish timestamp: `YYYY-MM-DDTHH:MM:SSZ` from the local clock.
/// Avoiding a date crate — one second of skew is fine for run provenance.
pub fn now_rfc3339() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let s = (secs % 60) as u32;
    let m = ((secs / 60) % 60) as u32;
    let h = ((secs / 3600) % 24) as u32;
    let days = secs / 86400;
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm, public domain).
    let z = days as i64 + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = (yoe as i64) + (era as i64) * 400 + 1970;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let mo = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let y = if mo <= 2 { y + 1 } else { y };
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

// ─── Role lanes + sandbox helpers ───────────────────────────────────────────

/// One authoritative statement of each role's lane — used by `arte brief`, and
/// composed into `arte implement`'s headless brief.
pub fn role_lane(role: &str) -> &'static str {
    match role {
        "specifier" | "spec" => "You own the BOARD (.truth/): intents, impls, controls, notes, contracts. You may NOT write tests or code. Fill the board completely; note every design decision on its node; run `arte coverage` — every intent covered before you hand off. YOU ARE ALSO THE ARBITER: when the implementer disputes a test, verify the claim independently (read both sides, re-derive the math/API yourself), then RULE — (a) test wrong: instruct the test-author precisely what to fix; (b) implementation wrong: reject the dispute with the reason; (c) the CONTROL was ambiguous: amend the control/note first, then cascade the fix. Record every ruling as a note on the contested control (the board is the court record). The disputants never settle between themselves. TRIAGE SPEC-GAP reports from any role: a real gap with no home -> mint the control (or amend one) and note the provenance; noise or already-covered -> decline WITH the existing home's id. Gaps found downstream are the spec improving — expected, not failure. NEVER COMMIT from this lane: the orchestrator audits your footprint, then commits once per round with the intent id (solo mode: you are your own orchestrator — audit, then commit with the intent id).",
        "test-author" | "tester" | "adversary" => "You own test/ + validation nodes. STAMP, DON'T MINT: if a control already has a validation stub, `arte at` it to your test — never add a twin (arte refuses exact ones; heed the similar-node warnings). Add a validation node ONLY for a control that truly has none. For EVERY control without a validation: read it with `arte show <id>` (NOT the raw file — show includes inherited ancestor notes, which are BINDING design decisions), then write a test that FAILS if the behavior is missing or wrong (a not-yet-built feature SHOULD be red — that red is the implementer's work order), add the validation node, stamp it with `arte at`. Never touch src/ (read it freely). Never set status — `arte verify` derives it. After writing each test RUN it and confirm it fails for the RIGHT reason (no vacuous passes). RECORD THE RED: stamp (`arte at`) and `arte verify <id>` while the test is still failing, THEN hand to impl — that puts the red in run history, which is the only durable proof the test can fail at all (`arte coverage` reports PROVEN = observed red at least once; `arte gate` names the unproven). A green that has never been red may be a test that CANNOT fail — measured on a real board, the first such validation fault-injected was fake (it read through a service-role endpoint, so the rule it named was never under test). When a validation cannot be red-first (pre-existing behaviour), prove it by ADVERSARY ROUND: inject the exact fault it claims to catch, confirm it goes red with the expected message, restore the fault precisely, confirm green returns. THE INITIAL CONDITION IS PART OF THE CONTRACT: a test that passes only against accumulated fixture residue proves nothing — enforce the starting state (point the verify `setup` hook at a fixture reset), seed every row you assert about, and write asserts as a DELTA from that declared baseline, never against whatever happened to be there. SELF-CLEAN: every fixture your spec mints (disposable accounts, seeded rows) is deleted by the spec's own last step — leaked residue displaces sibling specs' windows and orderings. KNOW THE VACUOUS-GREEN SHAPES and refuse them: an assert whose seeded NAME contains the asserted word; counting calls on a patched shared global instead of checking content; optional `if (el) fill(el)` guards that silently skip; a spec file that truncates (bad YAML) so only its early steps run; a zero-match test-name filter passing as \"0 failed\"; re-implementing the rule under test inside the test (expose the production statement instead and call THAT). Single-use credentials redeem exactly once — a spec uses the link OR the token, never both. If a control is untestable or contradicts another, REPORT the conflict — never silently work around it. When a test of yours is DISPUTED, do not defend or amend it on the implementer's word alone: re-verify with your own runs and math, and change it only per the SPECIFIER's ruling (recorded on the control). Direct questions to/from other roles are fine — talk is free; writes are walled. You see corners the spec missed: report each as \"SPEC-GAP: <missing control/edge, and why it matters>\" — the SPECIFIER triages and decides what enters the board; you never mint controls yourself. NEVER COMMIT from this lane: the orchestrator audits your footprint, then commits once per round with the intent id (solo mode: you are your own orchestrator — audit, then commit with the intent id).",
        "implementer" | "impl" => "You own src/. Your entire spec = the board + the failing tests. Read every control you implement against with `arte show <id>` — inherited ancestor notes are BINDING design decisions. Decide HOW, never WHAT. Never touch test/ or .truth/ except `arte at <impl-id> <src-file>` stamps on impls you realize. Honor `contract:` names exactly. If you believe a TEST is wrong, do not work around or edit it — record the dispute ON THE BOARD (`arte set <control-id> note \"DISPUTE: <test> — <your mathematical/API reason>\"`) and report DISPUTE; the SPECIFIER arbitrates. You may ASK the specifier or test-author questions directly (talk is free — the walls bound writes, not speech), but only a specifier ruling changes a goalpost. While implementing you will meet cases the spec never mentions (unhandled input, missing behavior, ambiguity): do NOT silently decide them — report each as \"SPEC-GAP: <what is unspecified, and the decision it forces>\"; the specifier rules and the board records it. Before declaring done, run `arte gate` and fix everything fixable from src/. NEVER COMMIT from this lane: the orchestrator audits your footprint, then commits once per round with the intent id (solo mode: you are your own orchestrator — audit, then commit with the intent id).",
        "meta-planner" | "planner" => "You are the META-PLANNER. You don't author code, tests, or board nodes — you COORDINATE. Read the board (`arte observe`), read run history (`arte runs <id>`), drive the loop (`arte cycle --once`), read `.loop-dispatch` after each cycle to know the next move. DISPATCH: read `recipes/<harness>/<role>.md` for the canonical lane prompt + tools allow/deny list, copy the template, fill in <TASK>, spawn via the Agent tool (Claude Code) / delegate_task (Hermes) / equivalent. The recipe is the single source of truth — never hand-compose prompts. AUDIT BETWEEN HAND-OFFS: every lane's `OWN:` line is a literal deny-list. Run `git diff --stat HEAD~1` after each spawn and reject any file outside the lane's OWN dirs; a lane violation is the actual lie the gate exists to prevent. Run `arte verify` and `arte gate` yourself to derive verdicts — never trust agent reports. Spec disputes escalate to specifier; you don't adjudicate. Stable-pass rule: green claims auto-promote only after the meta-planner observes ≥2 of last 5 passes — the meta-planner IS the promotion authority. COMMITS ARE YOURS ALONE: workers never commit. After the lane audit passes, commit ONCE per round with the intent id in the message (`arte check-commits` verifies). Audit finds an out-of-lane file → revert it and re-dispatch; never absorb a violation into the commit. DERIVATIONS SERIALIZE: one verify/gate at a time across the whole team — a concurrent run's fixture reset wipes another's seeds mid-flight and manufactures fake reds. Run the FINAL gate yourself, foregrounded and PID-tracked, as the only process — a gate orphaned in a lost background keeps re-deriving the board underneath everyone. Frictions met while working the subject repo become INTENTS on the tool's own board — that loop is how the workflow improves itself.",
        _ => "You sign off: run the real artifact adversarially (edge input, hostile input, rapid interaction), compare against each intent's plain meaning — not against the tests, which may share the build's blind spots. File concrete defects and missing-spec items; you write no code, no tests, no board nodes. A finding ATTACHES to the existing control governing that surface (strengthen it) — propose a NEW control only for genuinely ungoverned surface (`arte coverage` shows what has no home).",
    }
}

/// Directories each role is NOT allowed to write (the deny-list for sandbox).
pub fn role_deny_dirs(role: &str) -> &'static [&'static str] {
    match role {
        "implementer" | "impl" => &["test", "tests", "qa", "tests/", "qa/tests", ".truth"],
        "test-author" | "tester" | "adversary" => &["src", ".truth/impls", ".truth/controls"],
        "specifier" | "spec" => &["src", "test", "tests", "qa"],
        // meta-planner: read-only across the repo. Coordinates but doesn't author.
        // Can write `.loop-dispatch` via the cycle command (which runs as the user,
        // not inside the sandboxed agent).
        "meta-planner" | "planner" => &["src", "test", "tests", "qa/tests", ".truth"],
        _ => &[],
    }
}

// ─── Embedded docs / templates ──────────────────────────────────────────────

/// Prompt the implementer (headless agent) runs with — the test/lane contract
/// + the implementer's lane text. Single source of truth.
pub const IMPLEMENTER_HEADLESS: &str = "You are the IMPLEMENTER in a headless spec-driven loop. If the test command already passes and the gate is green, change nothing and report 'already green'. Add no dependencies; no backend/network; minimal diffs.";

pub const AGENT_GUIDE: &str = r#"arte — a design-truth board. You specify + trace software as a graph, so any
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
     Never hand-assert "it works". The INITIAL CONDITION is measured too:
     point `[verify] setup` at a fixture reset so every run starts from the
     declared baseline — preconditions in prose over accumulated residue are
     how a suite goes flaky (deterministic = enforced start → action → delta).
  5. `arte coverage` shows VERIFIED (test-backed + green), not just covered.

VERIFICATION DISCIPLINE (each of these was learned by breaking a real board):
  - ONE VERIFY AT A TIME. Derivations SERIALIZE: a destructive setup hook
    (fixture reset) makes concurrent runs wipe each other's seeds mid-flight —
    the reds it manufactures look exactly like real bugs. Never leave a verify
    or gate running in a background you can lose: an orphaned gate re-derives
    the board underneath everyone for as long as it lives.
  - Re-derive ONE validation with `arte verify <id...>` — a full-board regrind
    re-rolls shared-fixture interference on every run.
  - VERIFY-GREEN IS NOT GATE-GREEN. `arte gate` is the judgment: it also
    demands completeness — every intent realized in every layer, every control
    validation-backed, ZERO orphan src files, contracts held. Run the full
    gate, foregrounded, as the ONLY process, before declaring done.
  - Honesty raises the bar and that is the point: admitting an unspecced
    feature (new intent/impl) creates coverage gaps to close, not a regression.
  - A GREEN THAT HAS NEVER BEEN RED IS UNPROVEN. `arte coverage` reports
    PROVEN (observed red at least once, from run history); `arte gate` names
    the unproven ones. Earn it red-first (stamp + verify while failing, THEN
    implement), or by an adversary round: inject the fault the validation
    claims to catch, prove red with the expected message, restore, prove green.

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
    · COMMITS: workers never commit — lane violations discovered at commit time are
      tokens already burned. YOU commit, once per round, after the footprint audit
      passes, with the intent id in the message (`arte check-commits` verifies).

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

pub const SOFTWARE_TOML: &str = r#"# arte — software project
chain = ["intent", "impl", "control", "validation"]

[subsets]
intent     = ["who", "jobs", "why", "features", "constraints", "success"]
impl       = ["frontend", "backend", "database", "infra"]
control    = ["limit", "ux", "security", "TBD"]
validation = ["proof"]

[subset_axis]
intent     = "framing"
impl       = "component"
control    = "subset"
validation = "proof"
"#;

pub const SOFTWARE_GUIDE: &str = r#"
filling guide (software) — decompose to BEHAVIOR or reproductions diverge:
- one control per interaction: "<event> on <target> → <observable state change>"
- cover pointer, drag, wheel AND keyboard (guarded when a text field is focused)
- enumerate every pair (operator × operator, mode × edge, state × event) and
  give the pair's trickiest corner its own control
- styling is measurable (px, saturation, contrast) — never "looks right"
- every UI surface needs a REACHABILITY control (a user can navigate to it from
  the entry screen). Unit-green orphan screens are a measured failure mode.
"#;

pub const ARTE_TOML: &str = r#"# arte — its own design truth (dogfood)
chain = ["intent", "impl", "control", "validation"]

[subsets]
intent     = ["who", "jobs", "why", "features", "constraints", "success"]
impl       = ["core", "cli", "storage"]
control    = ["format", "identity", "storage", "agnostic", "subset"]
validation = ["proof"]

[subset_axis]
intent     = "framing"
impl       = "component"
control    = "subset"
validation = "proof"
"#;

pub fn template_toml(name: &str) -> Option<&'static str> {
    match name {
        "software" => Some(SOFTWARE_TOML),
        "arte" | "self" | "dogfood" => Some(ARTE_TOML),
        _ => None,
    }
}

pub fn resolve_template(name: &str) -> Option<String> {
    template_toml(name).map(|s| s.to_string())
}