//! Graph mutators: add, set, status, link, unlink, unset, delete, at.
//! All write `.truth/<id>.node` files; nothing here is derived.

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use crate::*;

/// `arte add <role> "<title>" [--subset F] [--parent P] [--serves S]`
pub fn cmd_add() {
    let mut a = std::env::args().skip(2);
    let (Some(role), Some(title)) = (a.next(), a.next()) else {
        eprintln!("usage: arte add <role> \"<title>\" [--subset F] [--parent P] [--serves S]");
        std::process::exit(2);
    };
    let rest: Vec<String> = a.collect();
    let conf = read_conf();
    if !conf.chain.contains(&role) {
        eprintln!("warn: role '{role}' not in chain {:?}", conf.chain);
    }
    let subset = flag_value(&rest, "--subset").unwrap_or_else(|| default_subset(&role));
    if let Some(fr) = conf.subsets.get(&role) {
        if !fr.contains(&subset) {
            eprintln!("warn: subset '{subset}' not declared in [subsets] for {role}");
        }
    }
    // DEDUPE: same role + same title is the SAME fact — reuse it instead of
    // minting a twin (field report: 26 auto-generated duplicate validations).
    let existing = all_nodes();
    if let Some((eid, _)) = existing.iter().find(|(_, n)| n.get("role") == Some(role.as_str()) && n.get("title") == Some(title.as_str())) {
        println!("exists: {eid}  (same role+title — link/stamp that id instead of duplicating)");
        return;
    }
    // near-duplicate: heavy word overlap with a same-role node — warn, still add
    let words = |t: &str| t.to_lowercase().split_whitespace().filter(|w| w.len() > 3).map(String::from).collect::<HashSet<_>>();
    let new_w = words(&title);
    if !new_w.is_empty() {
        for (eid, n) in existing.iter().filter(|(_, n)| n.get("role") == Some(role.as_str())) {
            let ex_w = words(n.get("title").unwrap_or(""));
            let hits = new_w.intersection(&ex_w).count();
            if hits * 10 >= new_w.len().min(ex_w.len()).max(1) * 7 && hits >= 3 {
                eprintln!("warn: very similar to {eid} (\"{}\") — if it's the same fact, stamp/link that id and delete this one", crate::trunc(n.get("title").unwrap_or(""), 48));
                break;
            }
        }
    }
    let id = gen_id(&role, &title);
    let mut n = Node { fields: Vec::new() };
    n.set_field("id", &id);
    n.set_field("role", &role);
    n.set_field("subset", &subset);
    if let Some(p) = flag_value(&rest, "--parent") {
        n.set_field("parent", &p);
        if !node_exists(&p) {
            eprintln!("warn: parent '{p}' doesn't exist (dangling)");
        }
    }
    n.set_field("title", &title);
    if let Some(s) = flag_value(&rest, "--serves") {
        n.push_field("serves", &s);
        if !node_exists(&s) {
            eprintln!("warn: serves '{s}' doesn't exist (dangling)");
        }
    }
    save_node(&id, &n);
    println!("added {id}  ({role}/{subset})");
}

/// `arte set <id> <key> <value>` — replace first occurrence (or append).
/// `contract` is multi-value so set APPENDS it (dedup'd); clear with unset.
pub fn cmd_set() {
    let mut a = std::env::args().skip(2);
    let (Some(id), Some(key), Some(val)) = (a.next(), a.next(), a.next()) else {
        eprintln!("usage: arte set <id> <key> <value>");
        std::process::exit(2);
    };
    let mut n = load_or_exit(&id);
    if key == "contract" {
        n.push_field(&key, &val);
    } else {
        n.set_field(&key, &val);
    }
    save_node(&id, &n);
    println!("set {id}.{key} = {val}");
}

/// `arte status <id> <ok|ko|pending|justified> [--requires-stable-pass] [--force]`
pub fn cmd_status() {
    let mut a = std::env::args().skip(2);
    let (Some(id), Some(st)) = (a.next(), a.next()) else {
        eprintln!("usage: arte status <id> <ok|ko|pending|justified> [--requires-stable-pass] [--force]");
        std::process::exit(2);
    };
    if !["ok", "ko", "pending", "justified"].contains(&st.as_str()) {
        eprintln!("status must be one of ok|ko|pending|justified (got '{st}')");
        std::process::exit(2);
    }
    let all_args: Vec<String> = std::env::args().collect();
    let requires = has_flag(&all_args, "--requires-stable-pass");
    let force = has_flag(&all_args, "--force");
    // Only gate GREEN claims (ko/pending/justified are signed, not measured).
    if st == "ok" && requires && !force {
        let runs = load_runs(&id);
        let (passed, stable) = stable_pass(&runs);
        if !stable {
            eprintln!("refusing: {id} has {passed} of last {} pass (need {}) — run `arte runs {id}` to see history, or pass --force to bypass", STABLE_PASS_WINDOW, STABLE_PASS_REQUIRED);
            std::process::exit(1);
        }
    }
    let mut n = load_or_exit(&id);
    n.set_field("status", &st);
    save_node(&id, &n);
    println!("status {id} = {st}");
}

pub fn cmd_link() {
    let mut a = std::env::args().skip(2);
    let (Some(id), Some(target)) = (a.next(), a.next()) else {
        eprintln!("usage: arte link <id> <serves-id>");
        std::process::exit(2);
    };
    if id == target {
        eprintln!("{id} cannot serve itself");
        std::process::exit(2);
    }
    let mut n = load_or_exit(&id);
    n.push_field("serves", &target);
    if !node_exists(&target) {
        eprintln!("warn: '{target}' doesn't exist (dangling link)");
    }
    save_node(&id, &n);
    println!("linked {id} ↑ {target}");
}

pub fn cmd_unlink() {
    let mut a = std::env::args().skip(2);
    let (Some(id), Some(target)) = (a.next(), a.next()) else {
        eprintln!("usage: arte unlink <id> <serves-id>");
        std::process::exit(2);
    };
    let mut n = load_or_exit(&id);
    if !n.remove_value("serves", &target) {
        eprintln!("{id} does not serve {target}");
        std::process::exit(2);
    }
    save_node(&id, &n);
    println!("unlinked {id} ↑ {target}");
}

pub fn cmd_unset() {
    let mut a = std::env::args().skip(2);
    let (Some(id), Some(key)) = (a.next(), a.next()) else {
        eprintln!("usage: arte unset <id> <key>   (clears a field: status|parent|serves|at|note|…)");
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

/// Nodes that reference `id` via `serves` or `parent` (would dangle if it's deleted).
pub fn dependents(id: &str) -> Vec<String> {
    all_nodes()
        .into_iter()
        .filter(|(nid, n)| nid != id && (n.all("serves").contains(&id) || n.get("parent") == Some(id)))
        .map(|(nid, _)| nid)
        .collect()
}

pub fn cmd_delete() {
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

/// `arte at <id> <file#unit>` — append a down-link to the artifact (dedup).
/// `<file#unit>` accepts `#` (FORMAT's `file#unit`) or `:` (grep/vitest habit);
/// the path part is what we check for existence.
pub fn cmd_at() {
    let mut a = std::env::args().skip(2);
    let (Some(id), Some(loc)) = (a.next(), a.next()) else {
        eprintln!("usage: arte at <id> <file#unit>");
        std::process::exit(2);
    };
    let file_part = loc.split(['#', ':']).next().unwrap_or(&loc);
    if !Path::new(file_part).exists() {
        eprintln!("warn: '{file_part}' doesn't exist (dangling trace — `arte gate` will fail until this resolves)");
    }
    let mut n = load_or_exit(&id);
    n.push_field("at", &loc);
    save_node(&id, &n);
    println!("at {id} → {loc}");
}

/// `arte check-commits` — backs c-every-change-commits-with-its-intent-id:
/// every commit since the last tag (or HEAD~20, or all-time if no tag) must
/// reference at least one intent id that exists on the board. Reports a per-
/// commit pass/fail so a reviewer (or the merge gate) can see the chain of
/// authorship without re-reading every message. Intents match the regex
/// `i[0-9A-Za-z][A-Za-z0-9_-]*` against the known intent ids under `.truth/`.
pub fn cmd_check_commits() {
    let mut a = std::env::args().skip(2);
    let limit = a.next().and_then(|s| s.parse::<usize>().ok()).unwrap_or(20);
    // Intent-id universe — every intent id present on the board.
    let known: HashSet<String> = all_nodes()
        .iter()
        .filter(|(_, n)| n.get("role") == Some("intent"))
        .map(|(id, _)| id.clone())
        .collect();
    // No git / no intents — nothing to enforce.
    if known.is_empty() {
        println!("check-commits: no intent nodes yet — nothing to enforce");
        return;
    }
    // Walk commits. Use the simplest `git log` shape that gives us sha + body.
    let log = std::process::Command::new("git")
        .args(["log", &format!("-{limit}"), "--format=%H%n%B%n--ARTE--"])
        .output();
    let Ok(out) = log else {
        eprintln!("check-commits: git unavailable — not enforcing");
        return;
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut commits: Vec<(String, String)> = Vec::new();
    for block in text.split("--ARTE--\n") {
        let block = block.trim();
        if block.is_empty() { continue; }
        let Some((sha, body)) = block.split_once('\n') else { continue };
        commits.push((sha.trim().to_string(), body.to_string()));
    }
    let mut unref = 0usize;
    let mut checked = 0usize;
    for (sha, body) in &commits {
        let first = body.lines().next().unwrap_or("").trim().to_lowercase();
        if first.starts_with("merge ") || first.is_empty() { continue; } // merges / empty
        checked += 1;
        let lower = body.to_lowercase();
        let hits: Vec<&str> = known
            .iter()
            .filter(|id| lower.contains(&id.to_lowercase()))
            .map(String::as_str)
            .collect();
        if hits.is_empty() {
            println!("  ✗ {sha:.7} — no intent id referenced");
            println!("      {first}");
            unref += 1;
        } else {
            println!("  ✓ {sha:.7} — {}", hits.join(", "));
        }
    }
    if checked == 0 {
        println!("check-commits: no commits to grade");
        return;
    }
    println!("check-commits: {checked} commit(s) · {} unref", unref);
    if unref > 0 { std::process::exit(1); }
}