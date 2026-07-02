//! Local persistence. The tool keeps its own hidden store in whatever project
//! it's launched from — drop it in any repo and it reads/writes `.surface.jsonl`
//! there. The store is just the wire format: one `CreateSurface` line per page,
//! so saving is "serialize current state" and loading is the normal `load()`.

use std::io::{BufRead, BufReader};

use anyhow::{bail, Context, Result};

use crate::protocol::{BoardColumn, BoardStyle, LayoutKind, UiMessage, UiNode};
use crate::state::AppState;

/// Default hidden store, relative to the current project dir.
pub const DEFAULT_PATH: &str = ".surface.jsonl";

/// Parse a JSONL store into state — apply every message to fresh `AppState`.
/// A core capability so any client (not just the TUI) can load a store.
pub fn load(path: &str) -> Result<AppState> {
    let file = std::fs::File::open(path).with_context(|| format!("opening {path}"))?;
    let mut app = AppState::default();
    for (i, line) in BufReader::new(file).lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let msg: UiMessage =
            serde_json::from_str(&line).with_context(|| format!("line {}: invalid message", i + 1))?;
        app.apply(msg).with_context(|| format!("line {}: rejected", i + 1))?;
    }
    if app.latest().is_none() {
        bail!("no surfaces created — need at least one CreateSurface");
    }
    Ok(app)
}

/// Load the store, or seed and save a fresh board if it doesn't exist yet.
pub fn load_or_seed(path: &str) -> Result<AppState> {
    if std::path::Path::new(path).exists() {
        load(path)
    } else {
        let app = seed();
        save(path, &app)?;
        Ok(app)
    }
}

// Live-sync sidecars: a running TUI tails the inbox (agent → TUI commands) and
// writes the state digest (TUI → agent). The lock marks that a TUI is live.
pub fn inbox_path(store: &str) -> String {
    format!("{store}.in")
}
pub fn state_path(store: &str) -> String {
    format!("{store}.state")
}
pub fn lock_path(store: &str) -> String {
    format!("{store}.lock")
}

// Focus sidecar: the durable "what the agent is working on" set, so a separate
// process (the PreToolUse gate) can read it. Mirrors `ui.working`.
pub fn focus_path(store: &str) -> String {
    format!("{store}.work")
}
/// Persist the working-focus titles (empty = remove the sidecar).
pub fn save_focus(store: &str, focus: &std::collections::HashSet<String>) -> Result<()> {
    let p = focus_path(store);
    if focus.is_empty() {
        let _ = std::fs::remove_file(&p);
        return Ok(());
    }
    let mut v: Vec<&str> = focus.iter().map(|s| s.as_str()).collect();
    v.sort();
    std::fs::write(&p, v.join("\n")).with_context(|| format!("writing {p}"))?;
    Ok(())
}
/// Read the working-focus titles (missing file = empty set).
pub fn load_focus(store: &str) -> std::collections::HashSet<String> {
    std::fs::read_to_string(focus_path(store))
        .ok()
        .map(|s| s.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect())
        .unwrap_or_default()
}

/// Serialize current surfaces back to the store, atomically (temp + rename).
pub fn save(path: &str, app: &AppState) -> Result<()> {
    let mut out = String::new();
    for id in &app.order {
        if let Some(s) = app.surfaces.get(id) {
            let msg = UiMessage::CreateSurface {
                id: id.clone(),
                title: s.title.clone(),
                root: s.root.clone(),
            };
            out.push_str(&serde_json::to_string(&msg)?);
            out.push('\n');
        }
    }
    let tmp = format!("{path}.tmp");
    std::fs::write(&tmp, out).with_context(|| format!("writing {tmp}"))?;
    std::fs::rename(&tmp, path).with_context(|| format!("renaming into {path}"))?;
    Ok(())
}

/// A fresh planning board — six empty pillars with guiding prompts. This is what
/// any new project starts with; the user fills it via the "+" affordances.
pub fn seed() -> AppState {
    // (stable key, header prompt) — the key is the absolute address handle.
    let pillars = [
        ("who", "WHO IT'S FOR — who uses it, who decides?"),
        ("jobs", "CORE JOBS — what must it get done?"),
        ("why", "WHY THIS — why build it, why now?"),
        ("features", "KEY FEATURES — what we'll actually ship"),
        ("constraints", "CONSTRAINTS — limits, rules, non-goals"),
        ("success", "SUCCESS — how we'll know it worked"),
    ];
    let columns = pillars
        .iter()
        .map(|(k, h)| BoardColumn {
            header: (*h).into(),
            key: Some((*k).into()),
            items: vec![],
        })
        .collect();
    let root = UiNode::Panel {
        id: "root".into(),
        title: None, // controls hint lives in the renderer, not the data
        layout: LayoutKind::Vertical,
        children: vec![UiNode::Board { id: "board".into(), columns, style: Default::default() }],
    };
    let mut app = AppState::default();
    app.apply(UiMessage::CreateSurface {
        id: "intents".into(),
        title: "intents".into(),
        root,
    })
    .expect("seed board is valid");
    app
}

/// One board page (surface) with the given style + (key, header) columns.
fn page(id: &str, title: &str, style: BoardStyle, cols: &[(&str, &str)]) -> UiMessage {
    let columns = cols
        .iter()
        .map(|(k, h)| BoardColumn { header: (*h).into(), key: Some((*k).into()), items: vec![] })
        .collect();
    UiMessage::CreateSurface {
        id: id.into(),
        title: title.into(),
        root: UiNode::Panel {
            id: format!("{id}-root"),
            title: None,
            layout: LayoutKind::Vertical,
            children: vec![UiNode::Board { id: format!("{id}-board"), columns, style }],
        },
    }
}

/// The canonical four-layer board: intent.map → impl.arch → control.spec →
/// validation.rep. What `init` lays down so the agent (and the gate) have the
/// full chain to fill, not just the intent pillars.
pub fn seed_full() -> AppState {
    let pillars = [
        ("who", "WHO IT'S FOR — who uses it, who decides?"),
        ("jobs", "CORE JOBS — what must it get done?"),
        ("why", "WHY THIS — why build it, why now?"),
        ("features", "KEY FEATURES — what we'll actually ship"),
        ("constraints", "CONSTRAINTS — limits, rules, non-goals"),
        ("success", "SUCCESS — how we'll know it worked"),
    ];
    let mut app = AppState::default();
    for m in [
        page("intent", "intent.map", BoardStyle::List, &pillars),
        page("impl", "impl.arch", BoardStyle::Cards, &[("components", "COMPONENTS — what realizes each intent")]),
        page("control", "control.spec", BoardStyle::Panel, &[("rules", "RULES — guardrails each component obeys")]),
        page("validation", "validation.rep", BoardStyle::Sheet, &[("proof", "PROOF — tests that verify each rule")]),
    ] {
        app.apply(m).expect("seed page is valid");
    }
    app
}
