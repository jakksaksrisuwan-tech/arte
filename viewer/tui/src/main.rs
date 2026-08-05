//! CLI:
//!   ratatui_render                 edit the local store (.surface.jsonl), or seed one
//!   ratatui_render <path>          edit a store at <path>
//!   ratatui_render schema          print the JSON Schema for UiMessage
//!   ratatui_render observe [addr]  agent read: terse digest of the store
//!   ratatui_render act '<cmd>'     agent drive: apply one terse command
//!   ratatui_render snapshot        full state as one JSON doc (for thin clients)
//!   ratatui_render coverage        per-intent coverage across layers + status
//! observe/act/snapshot/coverage target .surface.jsonl; set $DTRUTH_STORE to
//! point them at another project's board.

mod control;
mod mouse;
mod render;
mod theme;

use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::{self, Event};

use arte_core::{agent, store, sync};
use arte_core::protocol::{BoardColumn, BoardStyle, Item, LayoutKind, Status, UiMessage, UiNode};
use arte_core::state::AppState;

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("schema") => print_schema(),
        Some("usage") | Some("help") => {
            println!("{}", arte_core::agent::USAGE);
            Ok(())
        }
        // Agent interface (terse wire) over the local store.
        Some("observe") => run_observe(args.next()),
        Some("trace") | Some("depends") => run_trace(args.next()), // depends = back-compat alias
        Some("act") => run_act(args.next()),
        // Rich read for thin clients: full state as one JSON document.
        Some("snapshot") => run_snapshot(),
        // Design-truth coverage: per-intent, served across layers + status roll-up.
        Some("coverage") => run_coverage(),
        // PreToolUse gate: is the working focus backed by impl + control?
        Some("gate") => run_gate(),
        // Seed the 4-layer board + (optionally) wire an agent's hook/convention.
        Some("init") => run_init(args.collect()),
        // Assign stable lvl_area_NNN ids (area derived from the chain).
        Some("autoid") => run_autoid(),
        // Reconcile @trace <id> tags in code vs the board; --sync writes `at`.
        Some("verify") => run_verify(args.collect()),
        // git merge driver: 3-way semantic merge of the board (base ours theirs → ours).
        Some("merge") => run_merge(args.collect()),
        // Frontend for the arte primitive: render a `.truth/` node graph (read-only).
        Some("arte") => run_arte(args.next()),
        // Convert this arte-tui board → arte node files (one .node per item).
        Some("export-arte") => run_export_arte(args.next()),
        // A path → editable store at that path; no arg → the local hidden store.
        Some(path) => run_store(path),
        None => run_store(store::DEFAULT_PATH),
    }
}

/// Store path for the non-TUI subcommands. `$DTRUTH_STORE` overrides the default
/// hidden file, so observe/act/snapshot/coverage can target another project's
/// board without a positional path that would clash with addr/command args.
// ponytail: env var over a flag — the arg parser is positional; a flag would need
// real parsing for one knob. `DTRUTH_STORE=foo.jsonl ratatui_render observe`.
fn store_path() -> String {
    std::env::var("DTRUTH_STORE").unwrap_or_else(|_| store::DEFAULT_PATH.to_string())
}

fn print_schema() -> Result<()> {
    let schema = schemars::schema_for!(UiMessage);
    println!("{}", serde_json::to_string_pretty(&schema)?);
    Ok(())
}

/// Full-state JSON for a thin client. Reads the persisted store (the live TUI
/// saves there on every edit, so it stays current).
fn run_snapshot() -> Result<()> {
    let app = store::load_or_seed(&store_path())?;
    println!("{}", agent::snapshot(&app));
    Ok(())
}

/// Per-intent coverage digest across the layer boards.
fn run_coverage() -> Result<()> {
    let app = store::load_or_seed(&store_path())?;
    println!("{}", agent::coverage_digest(&app));
    Ok(())
}

/// Editable store: load the hidden file (or seed a fresh board), then persist
/// after every edit. This is the project-agnostic mode — runs in any directory.
fn run_store(path: &str) -> Result<()> {
    let mut app = store::load_or_seed(path)?;
    app.ui.working = store::load_focus(path); // restore the agent's declared focus → pulse is live on open
    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, &mut app, Some(path), None);
    ratatui::restore();
    result
}

/// Agent read: live digest from the running TUI if any, else computed from store.
fn run_observe(addr: Option<String>) -> Result<()> {
    let store = store_path();
    let store = store.as_str();
    // Top-level digest carries runtime cursor state, which only the live TUI has.
    if addr.is_none() && sync::is_live(store) {
        if let Some(s) = sync::read_state(store) {
            print!("{s}");
            return Ok(());
        }
    }
    let app = store::load_or_seed(store)?;
    println!("{}", agent::observe(&app, addr.as_deref()));
    Ok(())
}

/// Reverse trace: what does this item ultimately satisfy? (walks `serves` up.)
fn run_trace(addr: Option<String>) -> Result<()> {
    let Some(addr) = addr else {
        eprintln!("usage: ratatui_render trace <addr|id>   e.g. trace tsts_sco_001");
        std::process::exit(2);
    };
    let app = store::load_or_seed(&store_path())?;
    println!("{}", agent::trace(&app, &addr));
    Ok(())
}

/// Agent drive: send to the live TUI (it applies + saves), else apply to store.
fn run_act(line: Option<String>) -> Result<()> {
    let Some(line) = line else {
        eprintln!("usage: ratatui_render act '<command>'   e.g. add intent/who \"New item\"");
        std::process::exit(2);
    };
    let store = store_path();
    let store = store.as_str();
    if sync::is_live(store) {
        sync::send(store, &line)?;
        println!("→ {line}"); // applied live by the TUI
        return Ok(());
    }
    let mut app = store::load_or_seed(store)?;
    let out = agent::dispatch(&mut app, &line)?;
    if app.dirty {
        store::save(store, &app)?;
        app.dirty = false;
    }
    // `work` sets the durable focus the gate reads (headless has no live TUI to).
    if line.split_whitespace().next() == Some("work") {
        store::save_focus(store, &app.ui.working)?;
    }
    println!("{out}");
    Ok(())
}

/// git merge driver: `arte-tui merge <base> <ours> <theirs>` (git's %O %A %B).
/// Writes the 3-way semantic merge into <ours> (= %A). Exit 1 if conflicts were
/// annotated (so git flags the path), but the file is always valid.
fn run_merge(args: Vec<String>) -> Result<()> {
    let (Some(base), Some(ours), Some(theirs)) = (args.first(), args.get(1), args.get(2)) else {
        eprintln!("usage: arte-tui merge <base> <ours> <theirs>   (git: driver = arte-tui merge %O %A %B)");
        std::process::exit(2);
    };
    // base may be empty/absent (add/add) → empty board.
    let load = |p: &str| store::load(p).unwrap_or_default();
    let out = agent::merge3(&load(base), &load(ours), &load(theirs));
    store::save(ours, &out.app)?;
    if out.conflicts.is_empty() {
        println!("merge: clean ({} pages)", out.app.order.len());
        Ok(())
    } else {
        eprintln!("merge: {} conflict(s) annotated in the board (file is valid — resolve on the board):", out.conflicts.len());
        for c in &out.conflicts {
            eprintln!("  {c}");
        }
        std::process::exit(1);
    }
}

/// Assign stable ids to id-less items (area derived from the chain).
fn run_autoid() -> Result<()> {
    let store = store_path();
    if sync::is_live(&store) {
        eprintln!("a TUI is live on this board — quit it first so it doesn't save over the new ids");
        std::process::exit(2);
    }
    let mut app = store::load_or_seed(&store)?;
    let msg = agent::auto_id(&mut app)?;
    if app.dirty {
        store::save(&store, &app)?;
    }
    println!("{msg}");
    Ok(())
}

/// Reconcile `@trace <id>` tags found in code against the board.
fn run_verify(args: Vec<String>) -> Result<()> {
    let store = store_path();
    let sync = args.iter().any(|a| a == "--sync");
    let tags = scan_traces();
    let mut app = store::load_or_seed(&store)?;
    let rep = if sync {
        if sync::is_live(&store) {
            eprintln!("a TUI is live on this board — quit it first to --sync");
            std::process::exit(2);
        }
        let r = agent::sync_at(&mut app, &tags)?;
        if app.dirty {
            store::save(&store, &app)?;
        }
        r
    } else {
        agent::reconcile(&app, &tags)
    };
    println!(
        "verify: {} matched · {} dangling · {} unrealized · {} tags scanned{}",
        rep.matched.len(),
        rep.dangling.len(),
        rep.unrealized.len(),
        tags.len(),
        if sync { " (at written)" } else { "" }
    );
    for m in &rep.matched {
        println!("  ✓ {}/{}/{}  {} → {}", m.page, m.col, m.item, m.id, m.locator);
    }
    for (id, loc) in &rep.dangling {
        println!("  ⚠ dangling: @trace {id} ({loc}) — no such id on the board");
    }
    for u in &rep.unrealized {
        println!("  ✗ unrealized: {u} — no @trace in code");
    }
    Ok(())
}

/// id → locator (`file#Lnn`) from every `@trace <id>` in the project's source.
fn scan_traces() -> std::collections::HashMap<String, String> {
    let mut tags = std::collections::HashMap::new();
    for f in source_files() {
        let Ok(text) = std::fs::read_to_string(&f) else { continue };
        for (n, line) in text.lines().enumerate() {
            if let Some(pos) = line.find("@trace") {
                let id: String = line[pos + 6..]
                    .trim_start()
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if !id.is_empty() {
                    tags.insert(id, format!("{f}#L{}", n + 1));
                }
            }
        }
    }
    tags
}

/// Source files to scan: git-tracked if available, else a shallow walk of cwd.
fn source_files() -> Vec<String> {
    if let Ok(out) = std::process::Command::new("git").args(["ls-files"]).output() {
        if out.status.success() {
            return String::from_utf8_lossy(&out.stdout).lines().map(str::to_string).collect();
        }
    }
    let mut out = Vec::new();
    let mut stack = vec![std::path::PathBuf::from(".")];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || name == "target" || name == "node_modules" {
                continue;
            }
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push(p.to_string_lossy().to_string());
            }
        }
    }
    out
}

/// PreToolUse gate. Exit 0 = allow the code change; exit 2 = block (the agent
/// sees stderr). No board, or editing the board itself → allow (casual / free).
fn run_gate() -> Result<()> {
    let store = store_path();
    if !std::path::Path::new(&store).exists() {
        return Ok(()); // no board → gate is inert; casual projects unaffected
    }
    if let Some(fp) = edited_path_from_stdin() {
        let base = std::path::Path::new(&store).file_name().and_then(|s| s.to_str()).unwrap_or(".surface.jsonl");
        if fp.contains(base) {
            return Ok(()); // editing the board itself is free
        }
    }
    let app = store::load_or_seed(&store)?;
    let (ok, msg) = agent::gate(&app, &store::load_focus(&store));
    if ok {
        println!("{msg}");
        Ok(())
    } else {
        eprintln!("{msg}");
        std::process::exit(2); // Claude Code: non-zero PreToolUse blocks the call
    }
}

/// The edited file path out of a PreToolUse JSON payload on stdin (if piped).
fn edited_path_from_stdin() -> Option<String> {
    use std::io::{IsTerminal, Read};
    if std::io::stdin().is_terminal() {
        return None; // run by hand — no hook payload
    }
    let mut s = String::new();
    std::io::stdin().read_to_string(&mut s).ok()?;
    let v: serde_json::Value = serde_json::from_str(&s).ok()?;
    v.get("tool_input")?.get("file_path")?.as_str().map(str::to_string)
}

/// Seed the four-layer board and (optionally) wire an agent's hook/convention.
fn run_init(args: Vec<String>) -> Result<()> {
    let store = store_path();
    if std::path::Path::new(&store).exists() {
        println!("• board exists at {store} — left as-is");
    } else {
        store::save(&store, &store::seed_full())?;
        println!("• seeded 4-layer board → {store}");
    }
    install_merge_driver()?;
    let agent = args.iter().position(|a| a == "--agent").and_then(|i| args.get(i + 1)).map(String::as_str);
    let enforce = args.iter().any(|a| a == "--enforce");
    match agent {
        None => println!("• casual mode: CLI only, no hook (add --agent claude [--enforce] to wire one)"),
        Some("claude") => init_claude(enforce)?,
        Some(other) => {
            if other != "generic" {
                println!("• unknown agent '{other}' → generic convention");
            }
            write_agents_md()?;
            println!("• generic: convention written (AGENTS.md); drive the board via the CLI");
        }
    }
    Ok(())
}

const AGENTS_MD: &str = r#"## Design-truth board (arte-tui)

This project tracks design truth on a board (.surface.jsonl). Before changing code:

1. `arte-tui observe` — read the board (intent -> impl -> control -> validation).
2. `arte-tui act 'work "<intent>"'` — declare which intent the change serves.
3. If no intent/impl/control fits, ASK THE USER to confirm, then add them:
   add intent/features "...", add impl/components "..." "<intent>", add control/rules "..." "<component>".
4. Make the code change, then record proof: `arte-tui act 'status <id> ok'`.

Watch live: run `arte-tui` in a terminal — items you `work` on pulse.
"#;

fn write_agents_md() -> Result<()> {
    let path = "AGENTS.md";
    let existing = std::fs::read_to_string(path).unwrap_or_default();
    if existing.contains("Design-truth board (arte-tui)") {
        println!("• AGENTS.md already documents the board — left as-is");
        return Ok(());
    }
    let body = if existing.trim().is_empty() { AGENTS_MD.to_string() } else { format!("{existing}\n{AGENTS_MD}") };
    std::fs::write(path, body).with_context(|| format!("writing {path}"))?;
    Ok(())
}

/// Install the board's git merge driver: `.gitattributes` + local git config, so
/// `git merge` reconciles design truth per-item instead of clobbering lines.
fn install_merge_driver() -> Result<()> {
    let path = ".gitattributes";
    let existing = std::fs::read_to_string(path).unwrap_or_default();
    if !existing.contains("merge=arte-tui") {
        let line = "*.surface.jsonl merge=arte-tui\ndesign.jsonl merge=arte-tui\n";
        let body = if existing.trim().is_empty() { line.to_string() } else { format!("{existing}\n{line}") };
        std::fs::write(path, body).with_context(|| format!("writing {path}"))?;
        println!("• merge driver: .gitattributes → board files use merge=arte-tui");
    }
    let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_else(|_| "arte-tui".into());
    let driver = format!("{exe} merge %O %A %B");
    let _ = std::process::Command::new("git").args(["config", "merge.arte-tui.name", "arte-tui design-truth merge"]).status();
    let ok = std::process::Command::new("git").args(["config", "merge.arte-tui.driver", &driver]).status().map(|s| s.success()).unwrap_or(false);
    if ok {
        println!("• merge driver: git config set (merge.arte-tui.driver)");
    } else {
        println!("• not a git repo — set manually: git config merge.arte-tui.driver '{driver}'");
    }
    Ok(())
}

fn init_claude(enforce: bool) -> Result<()> {
    write_agents_md()?;
    if !enforce {
        println!("• Claude: convention written (AGENTS.md) — soft mode, no gate");
        return Ok(());
    }
    let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_else(|_| "arte-tui".into());
    let snippet = format!(
        r#"{{
  "hooks": {{
    "PreToolUse": [
      {{ "matcher": "Edit|Write|MultiEdit", "hooks": [ {{ "type": "command", "command": "{exe} gate" }} ] }}
    ]
  }}
}}
"#
    );
    let path = ".claude/settings.json";
    if std::path::Path::new(path).exists() {
        println!("• {path} exists — merge this PreToolUse hook yourself:\n\n{snippet}");
    } else {
        std::fs::create_dir_all(".claude").ok();
        std::fs::write(path, &snippet).with_context(|| format!("writing {path}"))?;
        println!("• Claude: enforce hook written → {path} (calls `{exe} gate`)");
    }
    Ok(())
}

fn find_board_cols(n: &UiNode) -> Option<&Vec<BoardColumn>> {
    match n {
        UiNode::Board { columns, .. } => Some(columns),
        UiNode::Panel { children, .. } => children.iter().find_map(find_board_cols),
        _ => None,
    }
}

/// Convert the current arte-tui board → arte node files under `<out>/.truth/`.
/// Each item becomes one `.node`, with a stable id assigned and title-links
/// resolved to id-links (the identity-by-title → id migration, in practice).
fn run_export_arte(out: Option<String>) -> Result<()> {
    let out = out.unwrap_or_else(|| ".".into());
    let app = store::load_or_seed(&store_path())?;
    let rolecode = |r: &str| match r {
        "intent" => "i",
        "impl" => "m",
        "control" => "c",
        "validation" => "v",
        _ => "n",
    };
    let slug = |s: &str| {
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
        o.trim_matches('-').chars().take(28).collect::<String>()
    };
    // pages → columns, keeping each column's key as the FRAME (the framing within a role).
    let pages: Vec<(String, Vec<(String, Vec<&Item>)>)> = app
        .order
        .iter()
        .filter_map(|pid| {
            let s = app.surfaces.get(pid)?;
            let cols = find_board_cols(&s.root)?;
            let colvec = cols.iter().map(|c| (c.key.clone().unwrap_or_else(|| slug(&c.header)), c.items.iter().collect::<Vec<&Item>>())).collect();
            Some((pid.clone(), colvec))
        })
        .collect();
    // pass 1: stable id per item; title→id for link resolution; record (role, frame).
    let mut title2id: std::collections::HashMap<String, String> = Default::default();
    let mut used: std::collections::HashSet<String> = Default::default();
    let mut assigned: Vec<(String, String, String, &Item)> = Vec::new();
    for (role, cols) in &pages {
        for (colframe, items) in cols {
            for it in items {
                let base = it.id.clone().filter(|x| !x.is_empty()).unwrap_or_else(|| format!("{}-{}", rolecode(role), slug(&it.text)));
                let (mut id, mut n) = (base.clone(), 2);
                while used.contains(&id) {
                    id = format!("{base}-{n}");
                    n += 1;
                }
                used.insert(id.clone());
                title2id.entry(it.text.clone()).or_insert_with(|| id.clone());
                assigned.push((id, role.clone(), colframe.clone(), it));
            }
        }
    }
    // pass 2: write nodes (frame + id-resolved serves).
    let sw = |s: Status| match s {
        Status::Ok => "ok",
        Status::Fail => "ko",
        Status::Pending => "pending",
        Status::Justified => "justified",
    };
    let dir = format!("{out}/.truth");
    std::fs::create_dir_all(&dir)?;
    for (id, role, frame, it) in &assigned {
        let mut s = format!("id: {id}\nrole: {role}\nsubset: {frame}\ntitle: {}\n", it.text);
        // category = the type tag. control expects a type → default TBD when unset.
        let cat = it.category.clone().filter(|c| !c.trim().is_empty()).or_else(|| (role == "control").then(|| "TBD".into()));
        if let Some(c) = cat {
            s += &format!("category: {c}\n");
        }
        for sv in &it.serves {
            s += &format!("serves: {}\n", title2id.get(sv).cloned().unwrap_or_else(|| sv.clone()));
        }
        for a in &it.at {
            s += &format!("at: {a}\n");
        }
        if let Some(st) = it.status {
            s += &format!("status: {}\n", sw(st));
        }
        for nt in &it.note {
            s += &format!("note: {nt}\n");
        }
        std::fs::write(format!("{dir}/{id}.node"), s)?;
    }
    // arte.toml: chain + [subsets] — subsets derived from the EFFECTIVE ones used
    // (so control lists its types + TBD, not the dummy "rules" column).
    let chain: Vec<String> = pages.iter().map(|(r, _)| format!("\"{r}\"")).collect();
    let mut toml = format!("chain = [{}]\n\n[subsets]\n", chain.join(", "));
    for (role, _) in &pages {
        let mut fr: Vec<String> = Vec::new();
        for (_, r, f, _) in &assigned {
            if r == role && !fr.contains(f) {
                fr.push(f.clone());
            }
        }
        let q: Vec<String> = fr.iter().map(|f| format!("\"{f}\"")).collect();
        toml += &format!("{role} = [{}]\n", q.join(", "));
    }
    std::fs::write(format!("{out}/arte.toml"), toml)?;
    println!("exported {} nodes → {dir}  (chain: {})", assigned.len(), pages.iter().map(|(r, _)| r.as_str()).collect::<Vec<_>>().join(" -> "));
    Ok(())
}

// ---- arte frontend: read the `.truth/` node graph directly (the format is the
// contract — no dependency on the arte crate, like a git client reading .git) ----

struct ArteNode {
    id: String,
    role: String,
    frame: Option<String>, // sub-group within the role (the framing → a column)
    title: String,
    serves: Vec<String>,
    status: Option<String>,
    category: Option<String>, // the "type" tag (a11y / authz / …)
    note: Vec<String>,
    at: Vec<String>,
    sha: Option<String>, // commit the status was measured against
    modified: Option<u64>, // file mtime (working-tree edit time)
}

/// Parse `<dir>/.truth/*.node` (line-oriented `key: value`, repeated keys = multi).
fn load_arte_nodes(dir: &str) -> Vec<ArteNode> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(format!("{dir}/.truth")) else { return out };
    for e in rd.flatten() {
        if e.path().extension().map(|x| x != "node").unwrap_or(true) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(e.path()) else { continue };
        // file mtime = working-tree "modified" (resets on checkout; git history is the durable source)
        let modified = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs());
        let mut n = ArteNode {
            id: String::new(), role: String::new(), frame: None, title: String::new(),
            serves: Vec::new(), status: None, category: None, note: Vec::new(), at: Vec::new(), sha: None, modified,
        };
        for line in text.lines() {
            let l = line.trim();
            if l.is_empty() || l.starts_with('#') {
                continue;
            }
            let Some((k, v)) = l.split_once(':') else { continue };
            let (k, v) = (k.trim(), v.trim().to_string());
            match k {
                "id" => n.id = v,
                "role" => n.role = v,
                "subset" => n.frame = Some(v), // arte's wire field is `subset`; kept as `frame` internally
                "title" => n.title = v,
                "serves" => n.serves.push(v),
                "status" => n.status = Some(v),
                "category" => n.category = Some(v),
                "note" => n.note.push(v),
                "at" => n.at.push(v),
                // Was missing: every `sha:` line fell through to `_ => {}`, so
                // the board reserved a sha column and had nothing to put in it.
                "sha" => n.sha = Some(v),
                _ => {}
            }
        }
        if !n.id.is_empty() {
            out.push(n);
        }
    }
    out
}

fn arte_chain(dir: &str) -> Vec<String> {
    std::fs::read_to_string(format!("{dir}/arte.toml"))
        .ok()
        .and_then(|s| s.lines().find(|l| l.trim_start().starts_with("chain")).map(str::to_string))
        .and_then(|l| l.split_once('=').map(|(_, v)| v.to_string()))
        .map(|v| v.trim().trim_matches(['[', ']']).split(',').map(|x| x.trim().trim_matches('"').to_string()).filter(|x| !x.is_empty()).collect())
        .unwrap_or_else(|| vec!["intent".into(), "impl".into(), "control".into(), "validation".into()])
}

/// `[subsets]` from arte.toml: per-role ordered list of subsets (sub-columns).
fn arte_frames(dir: &str) -> std::collections::HashMap<String, Vec<String>> {
    let mut m = std::collections::HashMap::new();
    let Ok(s) = std::fs::read_to_string(format!("{dir}/arte.toml")) else { return m };
    let mut in_frames = false;
    for line in s.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            in_frames = t == "[subsets]";
            continue;
        }
        if !in_frames {
            continue;
        }
        if let Some((k, v)) = t.split_once('=') {
            let frames: Vec<String> = v.trim().trim_matches(['[', ']']).split(',').map(|x| x.trim().trim_matches('"').to_string()).filter(|x| !x.is_empty()).collect();
            if !frames.is_empty() {
                m.insert(k.trim().to_string(), frames);
            }
        }
    }
    m
}

/// Bridge: arte node graph → an AppState the existing renderer already knows.
/// One page per role (chain order); within a role, one COLUMN per framing
/// (declared in `[frames]`, plus any frame a node uses). Links resolved id→title.
fn arte_appstate(nodes: &[ArteNode], chain: &[String], frames: &std::collections::HashMap<String, Vec<String>>) -> AppState {
    let id2title: std::collections::HashMap<&str, &str> = nodes.iter().map(|n| (n.id.as_str(), n.title.as_str())).collect();
    let parse_status = |s: &Option<String>| match s.as_deref() {
        Some("ok") => Some(Status::Ok),
        Some("ko") | Some("fail") => Some(Status::Fail),
        Some("pending") => Some(Status::Pending),
        Some("justified") => Some(Status::Justified),
        _ => None,
    };
    let style = |r: &str| match r {
        "impl" => BoardStyle::Cards,
        "control" => BoardStyle::Panel, // control.spec viz: component sidebar + type/details/modified table
        "validation" => BoardStyle::Sheet,
        _ => BoardStyle::List,
    };
    let mut roles: Vec<String> = chain.to_vec();
    for n in nodes {
        if !roles.contains(&n.role) {
            roles.push(n.role.clone());
        }
    }
    let mut app = AppState::default();
    for role in &roles {
        let role_nodes: Vec<&ArteNode> = nodes.iter().filter(|n| &n.role == role).collect();
        // framing order: declared first, then any frame a node uses, then "general".
        let mut fr: Vec<String> = frames.get(role).cloned().unwrap_or_default();
        for n in &role_nodes {
            let f = n.frame.clone().unwrap_or_else(|| "general".into());
            if !fr.contains(&f) {
                fr.push(f);
            }
        }
        if fr.is_empty() {
            fr.push("general".into());
        }
        let columns: Vec<BoardColumn> = fr
            .iter()
            .map(|frame| {
                let items: Vec<Item> = role_nodes
                    .iter()
                    .filter(|n| n.frame.as_deref().unwrap_or("general") == frame)
                    .map(|n| Item {
                        text: n.title.clone(),
                        serves: n.serves.iter().map(|s| id2title.get(s.as_str()).map(|t| t.to_string()).unwrap_or_else(|| s.clone())).collect(),
                        status: parse_status(&n.status),
                        id: (!n.id.is_empty()).then(|| n.id.clone()),
                        category: n.category.clone(),
                        note: n.note.clone(),
                        at: n.at.clone(),
                        sha: n.sha.clone(),
                        modified: n.modified,
                        ..Default::default()
                    })
                    .collect();
                BoardColumn { header: frame.to_uppercase(), key: Some(frame.clone()), items }
            })
            .collect();
        let board = UiNode::Board { id: format!("{role}-board"), columns, style: style(role) };
        let root = UiNode::Panel { id: format!("{role}-root"), title: None, layout: LayoutKind::Vertical, children: vec![board] };
        let _ = app.apply(UiMessage::CreateSurface { id: role.clone(), title: role.clone(), root });
    }
    app
}

/// Fingerprint of `<dir>/.truth/*.node` (sorted name+mtime+len). Changes whenever
/// a node is added, removed, or edited — the cheap poll behind live refresh.
fn truth_fingerprint(dir: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut entries: Vec<(String, u64, u64)> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(format!("{dir}/.truth")) {
        for e in rd.flatten() {
            if e.path().extension().map(|x| x != "node").unwrap_or(true) {
                continue;
            }
            let name = e.file_name().to_string_lossy().to_string();
            let (mut mtime, mut len) = (0u64, 0u64);
            if let Ok(m) = e.metadata() {
                len = m.len();
                mtime = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_nanos() as u64).unwrap_or(0);
            }
            entries.push((name, mtime, len));
        }
    }
    entries.sort();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    entries.hash(&mut h);
    // focus changes must also repaint (so the declared-focus pulse updates live)
    std::fs::read_to_string(format!("{dir}/.truth/.focus")).unwrap_or_default().hash(&mut h);
    h.finish()
}

/// Node ids the agent declared as focus — `.truth/.focus`, written by `arte working`.
fn arte_focus_ids(dir: &str) -> Vec<String> {
    std::fs::read_to_string(format!("{dir}/.truth/.focus"))
        .ok()
        .map(|s| s.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect())
        .unwrap_or_default()
}

/// Titles to PULSE: each focused node plus its serves-ancestors, so the whole
/// intent chain lights up across layers (matches AppState::working_cells).
fn arte_working(dir: &str, nodes: &[ArteNode]) -> std::collections::HashSet<String> {
    let by_id: std::collections::HashMap<&str, &ArteNode> = nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    // auto-retire: a focus node that's already green (ok/justified) stops pulsing,
    // so a stale or finished focus doesn't blink forever when nothing's in progress.
    let green = |id: &str| by_id.get(id).map(|n| matches!(n.status.as_deref(), Some("ok") | Some("justified"))).unwrap_or(false);
    let mut out = std::collections::HashSet::new();
    let mut seen = std::collections::HashSet::new();
    let mut stack: Vec<String> = arte_focus_ids(dir).into_iter().filter(|id| !green(id)).collect();
    // DERIVED focus: a node written in the last ~45s IS being worked on — pulse it
    // without any agent declaring anything. (Measured: across 10+ agent runs, not
    // one ever called `arte working`; activity itself is the honest focus signal.)
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    stack.extend(
        nodes
            .iter()
            .filter(|n| n.modified.is_some_and(|m| now.saturating_sub(m) < 45) && !green(&n.id))
            .map(|n| n.id.clone()),
    );
    while let Some(id) = stack.pop() {
        if !seen.insert(id.clone()) {
            continue;
        }
        if let Some(n) = by_id.get(id.as_str()) {
            out.insert(n.title.clone());
            stack.extend(n.serves.iter().cloned());
        }
    }
    out
}

/// Render an arte board in the TUI (read-only viewer). Launches even on an empty/
/// uninitialized dir — the board shows empty and live-refresh fills it in as the
/// agent runs `arte init` / `arte add` in the other pane.
fn run_arte(dir: Option<String>) -> Result<()> {
    let dir = dir.unwrap_or_else(|| ".".into());
    let nodes = load_arte_nodes(&dir);
    let mut app = arte_appstate(&nodes, &arte_chain(&dir), &arte_frames(&dir));
    app.ui.working = arte_working(&dir, &nodes); // declared focus → live pulse on open
    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, &mut app, None, Some(&dir));
    ratatui::restore();
    result
}

fn event_loop(terminal: &mut ratatui::DefaultTerminal, app: &mut AppState, store_path: Option<&str>, arte_dir: Option<&str>) -> Result<()> {
    // Live-sync channel (only when store-backed). See sync.rs.
    let mut chan = store_path.map(sync::Channel::open);
    if let Some(c) = &chan {
        c.publish(app);
    }
    mouse::enable();
    terminal.clear()?; // full clear so nothing bleeds from before launch

    let mut last_focus = app.ui.working.clone();
    let start = std::time::Instant::now();
    // arte live refresh: poll .truth/ and rebuild when the agent edits it (no
    // fs-watch dep). Throttled to ~2×/sec. Only active in arte mode (Some(dir)).
    let mut arte_fp = arte_dir.map(truth_fingerprint);
    let mut last_scan = std::time::Instant::now();
    let mut last_pulse = std::time::Instant::now();
    loop {
        if let Some(dir) = arte_dir {
            if last_scan.elapsed() >= Duration::from_millis(400) {
                last_scan = std::time::Instant::now();
                let now = truth_fingerprint(dir);
                if Some(now) != arte_fp {
                    arte_fp = Some(now);
                    let nodes = load_arte_nodes(dir);
                    let mut fresh = arte_appstate(&nodes, &arte_chain(dir), &arte_frames(dir));
                    fresh.ui = std::mem::take(&mut app.ui); // keep cursor/scroll/page
                    *app = fresh;
                    app.ui.working = arte_working(dir, &nodes); // re-read focus → pulse follows it live
                    last_pulse = std::time::Instant::now();
                } else if last_pulse.elapsed() >= Duration::from_secs(5) {
                    // recency pulse DECAYS: recompute every 5s even with no board
                    // change, so "modified in the last 45s" stops blinking on time.
                    last_pulse = std::time::Instant::now();
                    app.ui.working = arte_working(dir, &load_arte_nodes(dir));
                }
            }
        }
        // Pulse phase for "working on" items — flips every 500ms (poll is 100ms).
        app.ui.blink.set((start.elapsed().as_millis() / 500).is_multiple_of(2));
        terminal.draw(|frame| render::render_app(frame, app))?;
        let mut changed = false;

        if event::poll(Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(key) => {
                    if let control::Outcome::Quit = control::handle_key(app, key) {
                        break;
                    }
                    changed = true;
                }
                Event::Mouse(me)
                    if mouse::handle(app, me) => {
                        changed = true;
                    }
                _ => {}
            }
        }
        if let Some(c) = &mut chan {
            if c.drain(app) {
                changed = true;
            }
        }

        if changed {
            if app.dirty {
                if let Some(path) = store_path {
                    store::save(path, app)?;
                }
                app.dirty = false;
            }
            if let Some(c) = &chan {
                c.publish(app);
            }
            // keep the focus sidecar (read by the gate) in step with the pulse.
            if let Some(path) = store_path {
                if app.ui.working != last_focus {
                    let _ = store::save_focus(path, &app.ui.working);
                    last_focus = app.ui.working.clone();
                }
            }
        }
    }

    mouse::disable();
    if let Some(c) = &chan {
        c.close();
    }
    Ok(())
}


#[cfg(test)]
mod tests {
    use super::*;
    use arte_core::protocol::UiNode;

    #[test]
    fn parses_demo_line() {
        let line = r#"{"type":"CreateSurface","id":"s","title":"T","root":{"type":"Text","id":"t","text":"hi"}}"#;
        let msg: UiMessage = serde_json::from_str(line).unwrap();
        assert!(matches!(msg, UiMessage::CreateSurface { .. }));
    }

    #[test]
    fn rejects_unknown_component_type() {
        let line = r#"{"type":"CreateSurface","id":"s","title":"T","root":{"type":"Bogus","id":"x"}}"#;
        assert!(serde_json::from_str::<UiMessage>(line).is_err());
    }

    #[test]
    fn example_file_loads_and_validates() {
        // tests run with cwd = this crate dir; examples live at the workspace root.
        let app = store::load(concat!(env!("CARGO_MANIFEST_DIR"), "/../examples/ocr_pipeline.jsonl")).unwrap();
        let surface = app.latest().unwrap();
        assert_eq!(surface.title, "OCR Pipeline");
        assert!(matches!(surface.root, UiNode::Panel { .. }));
    }
}

#[cfg(test)]
mod pulse_tests {
    use super::*;
    #[test]
    fn recent_write_pulses_without_focus() {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        let n = ArteNode {
            id: "c-x".into(), role: "control".into(), frame: None, title: "fresh work".into(),
            serves: vec![], status: None, category: None, note: vec![], at: vec![], sha: None, modified: Some(now - 5),
        };
        let old = ArteNode { modified: Some(now - 300), id: "c-old".into(), title: "stale".into(), ..dummy() };
        let w = arte_working("/nonexistent-dir", &[n, old]);
        assert!(w.contains("fresh work"), "recently-written node must pulse");
        assert!(!w.contains("stale"), "old node must not pulse");
    }
    #[test]
    fn derived_pulse_reaches_working_cells_end_to_end() {
        // node written seconds ago -> arte_working -> appstate -> working_cells hit
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        let n = ArteNode {
            id: "c-fresh".into(), role: "control".into(), frame: Some("ux".into()),
            title: "fresh control".into(), serves: vec![], status: None, category: None,
            note: vec![], at: vec![], sha: None, modified: Some(now - 3),
        };
        let nodes = vec![n];
        let chain = vec!["intent".to_string(), "impl".into(), "control".into(), "validation".into()];
        let mut app = arte_appstate(&nodes, &chain, &Default::default());
        app.ui.working = arte_working("/nonexistent-dir", &nodes);
        // activate the control page (chain index 2)
        app.set_active(2);
        assert!(!app.working_cells().is_empty(), "freshly-written control must light a cell on its page");
    }

    fn dummy() -> ArteNode {
        ArteNode { id: String::new(), role: "control".into(), frame: None, title: String::new(),
            serves: vec![], status: None, category: None, note: vec![], at: vec![], sha: None, modified: None }
    }
}

#[cfg(test)]
mod arte_import_tests {
    use super::*;

    /// v-viewer-board-shows-the-measured-against-commit
    /// The bug this pins: `load_arte_nodes` had no "sha" arm, so every `sha:`
    /// line fell through `_ => {}`. The TUI reserved a 9-char sha column
    /// (render.rs `sha_w`) and rendered `item.sha`, which was always None — the
    /// board displayed a blank column for a field the board files carried all
    /// along. Recording evidence nobody can see is most of the way to not
    /// having it.
    #[test]
    fn importer_carries_sha_from_node_files() {
        let dir = std::env::temp_dir().join(format!("arte-tui-sha-{}", std::process::id()));
        let truth = dir.join(".truth");
        std::fs::create_dir_all(&truth).expect("scratch");
        std::fs::write(
            truth.join("v-probe.node"),
            "id: v-probe\nrole: validation\nsubset: proof\ntitle: probe\nstatus: ok\nsha: 5c2826e8b360b0366d28d8d0bb3d48c09b9f61de\nat: tests/probe.rs\n",
        )
        .expect("node");
        let nodes = load_arte_nodes(dir.to_str().unwrap());
        let probe = nodes.iter().find(|n| n.id == "v-probe").expect("node loaded");
        assert_eq!(
            probe.sha.as_deref(),
            Some("5c2826e8b360b0366d28d8d0bb3d48c09b9f61de"),
            "the viewer must carry `sha:` from the node file — the board has a sha column and \
             an importer that drops the field renders it permanently blank"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Reported from use: "some columns have 7, some have 8 chars". Cause was
    /// not the data — arte records the full 40-char sha-1 — but the surfaces:
    /// the CLI truncated to 7 and the TUI to 8, and a board mid-re-derive holds
    /// both widths, so the column came out ragged. One shared constant now.
    fn both_surfaces_render_one_sha_width() {
        let legacy = "35591c9";
        let full = "5c2826e8b360b0366d28d8d0bb3d48c09b9f61de";
        let show = |s: &str| -> String { s.chars().take(arte_core::SHA_DISPLAY_LEN).collect() };
        assert_eq!(
            show(legacy).chars().count(),
            show(full).chars().count(),
            "a legacy short sha and a full sha-1 must render at the same width — \
             otherwise the sha column is ragged depending on when each row was verified"
        );
        assert_eq!(show(full), "5c2826e", "display must be a prefix of the recorded sha");
        assert_eq!(show(legacy), legacy, "a 7-char legacy record renders whole");
    }
}
