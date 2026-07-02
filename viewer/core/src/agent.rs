//! Agent interface: observe (read) + act (drive), transport-agnostic.
//!
//! Two layers, on purpose:
//!   • EXECUTE layer — `observe()` and `act_*()` take structured args and run
//!     against `AppState`. This is the real logic; it knows nothing about wires.
//!   • PARSE layer — `dispatch()` turns a terse line ("add intent/0 \"X\"") into
//!     an execute call. This is the token-minimal transport.
//!
//! MCP SEAM: an MCP server would expose `observe` as a *resource* and the
//! `act_*` fns as *tools*, calling the EXECUTE layer directly with JSON args —
//! it never touches `dispatch()` (the string parser). So MCP is a sibling
//! transport next to the terse wire, not a rewrite. Keep new agent capability
//! in the execute layer and both transports get it for free.

use anyhow::{bail, Result};

use crate::protocol::{BoardColumn, BoardStyle, UiMessage, UiNode};
use crate::state::AppState;

/// The terse interface, self-describing — an agent reads this once to drive the
/// tool. Emitted by `usage`/`help` (CLI and verb).
pub const USAGE: &str = "\
arte-tui — a design-truth board. Be CONSCIOUS of what you change and why.
The board is your externalized reasoning: every element links up a chain to the
intent it serves. Don't edit blind — map first, then act.

PROTOCOL (always)
  1. observe + coverage — read the map before doing anything.
  2. Comprehend first. Existing code: reconstruct the WHOLE map — the INTENTS
     (the why) AND impl→control→validation realizing them, all linked — before
     editing. Empty project: define the intents with the user first, then plan
     and implement.
  3. Link every element up the chain (serves) and leave a note with your
     reasoning. No orphan element (no intent), no intent without realization.
  4. coverage is your completeness meter; status asserts truth (the tool judges
     structure, never substance — it won't guess correctness for you).
Identity = the visible title string (deduped). Links are by title.

READ
  observe [addr]   state digest; addr narrows depth (page → cols → items → one item)
  coverage         audit: per-intent gap matrix + counts + orphans (intent←impl←ctrl←val)
  trace  <addr|id>  reverse trace: what this item ultimately satisfies (walks up)
  snapshot         full state as one JSON document (incl. the audit block)
  gate             PreToolUse check: is the `work` focus backed by impl+control?
                   (exit 0 = allow code change; exit 2 = block with guidance)
  init [--agent X [--enforce]]   seed the 4-layer board; wire an agent's hook/convention
  autoid           assign stable lvl_area_NNN ids to id-less items (area from the chain)
  verify [--sync]  reconcile @trace <id> tags in code vs board; --sync writes `at`
  merge <O> <A> <B>  git merge driver: 3-way semantic merge of the board → A

ADDRESS  page/col/item   — OR a bare id / \"title\" (most efficient: no observe round-trip)
  page = surface id (e.g. impl)   col = index or key (e.g. fe)
  item = index | + (add slot) | h (header)
  bare = a stable id (tsts_sco_001) or a single-word/\"quoted\" title resolves the
         item globally — edit by what you named it, robust to reordering. e.g.
         status tsts_sco_001 ko   ·   comment tsts_sco_001 \"flaky\"

DRIVE
  add  <col> \"text\" [\"link\"…] [status ok|ko|pending|na]
                   add an element; links + status inline
  set  <addr> \"text\" [\"link\"…] [status …]
                   edit text (+links/status); set <col>/h \"name\" renames the header
  link   <addr> \"title\"…   set `serves` = title(s) one layer up; bare clears
  status <addr> ok|ko|pending|justified|na   (justified = failed but accepted)
  note   <addr> \"line\"…    the how/what description (a list); bare clears
  attach <addr> \"ref\"…     attachment paths/URLs (pointers, not bytes); bare clears
  at     <addr> \"loc\"…     realization locator(s) file#unit (where it's built); bare clears
  cat    <addr> \"category\"  free category tag / control method; bare clears
  id     <addr> \"tsts_sco_001\"  stable ref, shape lvl_area_NNN: innt|impl|ctrl|tsts
                   _ functional-area trigram _ 3 digits (trigram = high-level
                   functionality, reuse existing ones); bare clears
  comment <addr> \"text\"    remark on the RESULT (pairs with status); bare clears
  derive <addr> [off]      mark a parentless item as a derived requirement (not an
                   orphan) — the justified-twin on the trace axis; rationale in note
  sha    <addr> \"abc1234\"   the commit the item was tested/verified against; bare clears

CONTROL.SPEC governs the build — each row is a rule a component must satisfy
(and a testable contract). Rows read \"<component> [method] <detail>\":
  link → the component it constrains · cat → the method · text → the detail
  (comma detail renders as chips). Methods are free strings; software suggests:
  take emits limit connect requires authz guards on-error retains idempotent
  e.g.  add control/rules \"pdf, png\"; link control/rules/0 \"Document ingestion\"; cat control/rules/0 take
  A control is a TESTABLE CONTRACT: derive validation.rep cases from it — a proof
  item that `serves` the control ([take] pdf → \"accepts pdf\" / \"rejects bmp\").
  A validation case = id + name(text) + status + comment(on the result); note =
  what the test does, attach = a link to the test code. status = ok|ko|pending|
  justified; a JUSTIFIED fail is accepted — say why in `comment`. coverage flags
  any control with no test, and a justified test never reads as a failure.
  work   \"title\"…          highlight an intent across layers; bare clears
                   (auto-clears a title once its chain is fully done — covered + green)
  mv <addr> <±n>          reorder      del <addr>      delcol = del <col>/h
  addcol <page> \"header\" [key]        setcol = set <col>/h \"header\"
  addpage <id> \"title\" [list|cards|panel|sheet]   build a layer    delpage <id>
  setstyle <page> list|cards|panel|sheet          renamepage <id> \"title\"
  sel <addr>    page <id>    undo    redo

Status is producer-set; the tool judges structure (coverage), never substance.";

/// Full state as one JSON document — the rich read for non-agent clients (web,
/// native), which render their own way. `observe` stays the terse, token-minimal
/// read for agents. Both are derived from the same `AppState`; neither is stored.
pub fn snapshot(app: &AppState) -> String {
    let pages: Vec<serde_json::Value> = app
        .order
        .iter()
        .filter_map(|id| {
            app.surfaces
                .get(id)
                .map(|s| serde_json::json!({ "id": id, "title": s.title, "root": s.root }))
        })
        .collect();
    let mut working: Vec<&String> = app.ui.working.iter().collect();
    working.sort();
    serde_json::to_string_pretty(&serde_json::json!({
        "pages": pages,
        "coverage": coverage(app),
        "audit": audit(app), // tracy check-shape summary + gaps/orphans for a thin client
        "working": working, // highlight set (intent titles)
        "cursor": { "col": app.ui.cursor.col, "row": app.ui.cursor.row },
    }))
    .unwrap_or_else(|_| "{}".into())
}

// --- coverage: structural design-truth, computed never stored --------------
// Spine = the first board page (convention; a `role` flag can override later).
// Its items are the intents. Coverage = for each intent, which other board
// "layers" have an item that `serves` it, plus a status roll-up. The tool only
// reports STRUCTURE (served or not) — whether a realization is *correct* is the
// producer's `status`, never computed here. Domain-agnostic by construction.
#[derive(serde::Serialize)]
pub struct LayerHit {
    pub layer: String,
    pub served: bool,
}
#[derive(serde::Serialize)]
pub struct IntentCoverage {
    pub title: String,
    pub layers: Vec<LayerHit>,
    pub whole: bool, // served in every layer
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<crate::protocol::Status>,
    pub green: bool, // whole AND every serving item is ok/justified (no unverified gap)
}

fn board_pages(app: &AppState) -> Vec<(&str, &[BoardColumn])> {
    app.order
        .iter()
        .filter_map(|id| match app.surfaces.get(id).map(|s| find_primary(&s.root)) {
            Some(Some(UiNode::Board { columns, .. })) => Some((id.as_str(), columns.as_slice())),
            _ => None,
        })
        .collect()
}

pub fn coverage(app: &AppState) -> Vec<IntentCoverage> {
    let boards = board_pages(app);
    let Some((_, spine)) = boards.first() else { return Vec::new() };
    let layers = &boards[1..];
    let mut out = Vec::new();
    for col in spine.iter() {
        for intent in col.items.iter() {
            let title = &intent.text;
            let mut statuses = vec![intent.status];
            let mut layers_all_ok = true; // every serving layer item asserted ok/justified
            let mut hits = Vec::new();
            // Chain model: each layer links to the layer ABOVE it. Propagate the
            // reachable titles down — impl serves the intent, ctrl serves those
            // impl items, val serves those ctrl items. A broken link stops the chain.
            let mut reachable: std::collections::HashSet<&str> = std::iter::once(title.as_str()).collect();
            for (lid, lcols) in layers {
                let mut next: std::collections::HashSet<&str> = std::collections::HashSet::new();
                for lc in lcols.iter() {
                    for li in lc.items.iter() {
                        if li.serves.iter().any(|s| reachable.contains(s.as_str())) {
                            statuses.push(li.status);
                            if !matches!(li.status, Some(crate::protocol::Status::Ok) | Some(crate::protocol::Status::Justified)) {
                                layers_all_ok = false;
                            }
                            next.insert(li.text.as_str());
                        }
                    }
                }
                hits.push(LayerHit { layer: lid.to_string(), served: !next.is_empty() });
                reachable = next;
            }
            let whole = !hits.is_empty() && hits.iter().all(|h| h.served);
            let green = whole && layers_all_ok;
            out.push(IntentCoverage { title: title.clone(), layers: hits, whole, status: rollup(&statuses), green });
        }
    }
    out
}

/// Worst status wins: ko > pending > justified > ok > n/a. Justified (accepted
/// failure) surfaces above ok but never reads as a failure.
fn rollup(statuses: &[Option<crate::protocol::Status>]) -> Option<crate::protocol::Status> {
    use crate::protocol::Status::*;
    let rank = |s: &Option<crate::protocol::Status>| match s {
        Some(Fail) => 4,
        Some(Pending) => 3,
        Some(Justified) => 2,
        Some(Ok) => 1,
        None => 0,
    };
    statuses.iter().max_by_key(|s| rank(s)).copied().flatten()
}

/// Terse per-intent coverage for the agent: "title — impl✓ control✗  [pending]".
/// Cross-layer audit summary (tracy `check` shape): per-requirement coverage
/// rolled into counts, plus the actual gaps and orphans. Derived, never stored.
#[derive(Default, serde::Serialize)]
pub struct Audit {
    pub total: usize,       // requirements (spine items)
    pub implemented: usize, // intent served by the first downstream layer
    pub verified: usize,    // intent served by the last layer
    pub covered: usize,     // served in every layer (whole chain)
    pub gaps: Vec<String>,  // "title — impl✓ ctrl✗ val✗"
    // orphaned: a non-spine item with no parent (unlinked), a dangling parent title,
    // or a parent changed/renamed after the child (the link went stale).
    pub orphans: Vec<String>,
    pub deviations: Vec<String>, // justified (accepted) failures with no rationale comment
    pub justified: usize,   // validation items accepted as justified failures
    pub derived: usize,     // items accepted as parentless-by-design (not orphans)
    pub validated: usize,     // intents whose status rolls up green (ok), not just covered
    pub layers: Vec<(String, usize)>, // per downstream layer: how many intents it serves
}

pub fn audit(app: &AppState) -> Audit {
    let cov = coverage(app);
    let mut a = Audit { total: cov.len(), ..Default::default() };
    // per-layer served counts (impl / control / validation …), in chain order
    if let Some(first) = cov.first() {
        a.layers = first.layers.iter().map(|l| (l.layer.clone(), 0usize)).collect();
    }
    for c in &cov {
        for (i, l) in c.layers.iter().enumerate() {
            if l.served {
                if let Some(slot) = a.layers.get_mut(i) {
                    slot.1 += 1;
                }
            }
        }
        if c.layers.first().map(|l| l.served).unwrap_or(false) {
            a.implemented += 1;
        }
        if c.layers.last().map(|l| l.served).unwrap_or(false) {
            a.verified += 1;
        }
        if c.whole {
            a.covered += 1;
        } else {
            let cells: Vec<String> =
                c.layers.iter().map(|l| format!("{}{}", l.layer, if l.served { "✓" } else { "✗" })).collect();
            a.gaps.push(format!("{} — {}", trunc(&c.title, 22), cells.join(" ")));
        }
    }
    // orphans: a non-spine item links to nothing, or to a title that exists nowhere
    let boards = board_pages(app);
    let titles: std::collections::HashSet<&str> =
        boards.iter().flat_map(|(_, cols)| cols.iter().flat_map(|c| c.items.iter().map(|i| i.text.as_str()))).collect();
    for (pid, cols) in boards.iter().skip(1) {
        for it in cols.iter().flat_map(|c| &c.items) {
            if it.derived {
                a.derived += 1; // parentless on purpose — not an orphan
            } else if it.serves.is_empty() {
                a.orphans.push(format!("{pid}: {} — links to nothing", trunc(&it.text, 22)));
            } else if let Some(bad) = it.serves.iter().find(|s| !titles.contains(s.as_str())) {
                a.orphans.push(format!("{pid}: {} — dangling → {}", trunc(&it.text, 16), trunc(bad, 14)));
            }
        }
    }
    // justified (accepted) failures; each must carry a rationale comment (else a deviation)
    for (pid, cols) in &boards {
        for it in cols.iter().flat_map(|c| &c.items) {
            if it.status == Some(crate::protocol::Status::Justified) {
                a.justified += 1;
                if it.comment.as_deref().unwrap_or("").trim().is_empty() {
                    a.deviations.push(format!("{pid}: {} — justified with no rationale", trunc(&it.text, 22)));
                }
            }
        }
    }
    // validated: intents whose rolled-up status is green (covered AND ok), not just traced
    a.validated = cov.iter().filter(|c| c.status == Some(crate::protocol::Status::Ok)).count();
    // also orphaned: a child whose linked parent was modified/renamed AFTER it — the
    // link went stale, so the child is cut off from the current parent.
    let mut mtime: std::collections::HashMap<&str, u64> = std::collections::HashMap::new();
    for (_, cols) in &boards {
        for it in cols.iter().flat_map(|c| &c.items) {
            if let Some(m) = it.modified {
                let e = mtime.entry(it.text.as_str()).or_insert(0);
                *e = (*e).max(m);
            }
        }
    }
    for (pid, cols) in boards.iter().skip(1) {
        for it in cols.iter().flat_map(|c| &c.items) {
            if it.derived {
                continue;
            }
            // id-linked items (arte boards) resolve serves by ID — a parent rename
            // can't strand them, and routine parent writes (at: stamps, measured
            // status) bump mtime constantly. Staleness only exists for title links.
            if it.id.is_some() {
                continue;
            }
            let Some(cm) = it.modified else { continue };
            if it.serves.iter().any(|s| mtime.get(s.as_str()).is_some_and(|&pm| pm > cm)) {
                a.orphans.push(format!("{pid}: {} — parent changed since", trunc(&it.text, 22)));
            }
        }
    }
    a
}

/// Reverse trace: from an item, walk `serves` UPWARD to the requirement(s) it
/// ultimately satisfies — the auditor's "what does this verify?" (bidirectional
/// traceability). `addr` is a page/col/item address, or a bare id / title.
pub fn trace(app: &AppState, addr: &str) -> String {
    use crate::protocol::Item;
    let boards = board_pages(app);
    // title -> (page, serves) for the upward walk
    let mut map: std::collections::HashMap<&str, (&str, &[String])> = std::collections::HashMap::new();
    for (pid, cols) in &boards {
        for it in cols.iter().flat_map(|c| &c.items) {
            map.entry(it.text.as_str()).or_insert((pid, it.serves.as_slice()));
        }
    }
    // resolve the starting item: by address (page/col/item) or by id / title
    let resolved: (String, &Item) = if addr.contains('/') {
        let ad = parse_addr(addr);
        let Ok((p, c, i)) = resolve3(app, &ad) else { return format!("? bad address '{addr}'") };
        match board(app, &p).and_then(|cs| cs.get(c)).and_then(|col| col.items.get(i)) {
            Some(it) => (p, it),
            None => return format!("? no item {addr}"),
        }
    } else {
        match boards
            .iter()
            .flat_map(|(pid, cols)| cols.iter().flat_map(move |c| c.items.iter().map(move |it| (*pid, it))))
            .find(|(_, it)| it.id.as_deref() == Some(addr) || it.text == addr)
        {
            Some((pid, it)) => (pid.to_string(), it),
            None => return format!("? no item '{addr}'"),
        }
    };
    let (spage, sitem) = resolved;
    let head = sitem.id.as_deref().map(|i| format!("#{i} ")).unwrap_or_default();
    let mut out = vec![format!("{head}{}  ({spage})", sitem.text)];
    if sitem.serves.is_empty() {
        out.push("  (links to nothing — orphan)".into());
        return out.join("\n");
    }
    // DFS upward, bounded depth + a guard against cycles
    let mut stack: Vec<(String, usize)> = sitem.serves.iter().rev().map(|s| (s.clone(), 1)).collect();
    let mut guard = 0;
    while let Some((title, depth)) = stack.pop() {
        guard += 1;
        if guard > 300 {
            break;
        }
        match map.get(title.as_str()) {
            Some((pg, serves)) => {
                out.push(format!("{}⇒ {} ({})", "  ".repeat(depth), title, pg));
                if depth < 6 {
                    for s in serves.iter().rev() {
                        stack.push((s.clone(), depth + 1));
                    }
                }
            }
            None => out.push(format!("{}⇒ {} (dangling)", "  ".repeat(depth), title)),
        }
    }
    out.join("\n")
}

/// The PreToolUse gate: a code change is allowed only when every focused intent
/// (the `work` set) is backed by impl + control on the board — so the agent has
/// answered "what intent, which component, under which rule?" before touching code.
/// Returns (allowed, message). Validation isn't required (tests come after code).
///
/// ponytail: this proves a *backed focus exists*, not that the edit truly matches
/// it — forcing function, not airtight. Upgrade path: map edited file→impl item.
pub fn gate(app: &AppState, focus: &std::collections::HashSet<String>) -> (bool, String) {
    if focus.is_empty() {
        return (
            false,
            "Blocked: no active intent. Before changing code, declare what it serves:\n  \
             arte-tui act 'work \"<intent>\"'\n\
             If no intent fits, confirm with the user, then add intent → impl → control."
                .into(),
        );
    }
    let cov = coverage(app);
    let mut missing = Vec::new();
    for title in focus {
        match cov.iter().find(|c| &c.title == title) {
            None => missing.push(format!("'{title}' is not an intent on the board")),
            Some(c) => {
                let layer_ok = |i: usize| c.layers.get(i).map(|l| l.served).unwrap_or(false);
                let mut need = Vec::new();
                if !layer_ok(0) {
                    need.push("impl");
                }
                if !layer_ok(1) {
                    need.push("control");
                }
                if !need.is_empty() {
                    missing.push(format!("'{title}' needs {} on the board", need.join(" + ")));
                }
            }
        }
    }
    if missing.is_empty() {
        let mut f: Vec<&str> = focus.iter().map(|s| s.as_str()).collect();
        f.sort();
        (true, format!("ok — change answers to: {}", f.join(", ")))
    } else {
        (
            false,
            format!(
                "Blocked: code change not backed by the board:\n  {}\n\
                 Confirm with the user, add the missing layer(s), or `work` a backed intent.",
                missing.join("\n  ")
            ),
        )
    }
}

// ---- auto-id: stable lvl_area_NNN, area DERIVED from the chain (not hand-typed) ----
fn level_code(page: &str) -> Option<&'static str> {
    match page {
        "intent" => Some("innt"),
        "impl" => Some("impl"),
        "control" => Some("ctrl"),
        "validation" => Some("tsts"),
        _ => None,
    }
}
fn trigram(title: &str) -> String {
    let a: String = title.chars().filter(|c| c.is_ascii_alphabetic()).take(3).flat_map(char::to_lowercase).collect();
    if a.len() == 3 {
        a
    } else {
        "gen".into()
    }
}
/// Walk `serves` up to the first intent-page title; its trigram is this item's area.
fn area_of_item<'a>(
    title: &'a str,
    intents: &std::collections::HashSet<&str>,
    serves: &std::collections::HashMap<&'a str, &'a [String]>,
) -> String {
    let mut seen = std::collections::HashSet::new();
    let mut stack = vec![title];
    while let Some(t) = stack.pop() {
        if intents.contains(t) {
            return trigram(t);
        }
        if !seen.insert(t) {
            continue;
        }
        if let Some(s) = serves.get(t) {
            for x in s.iter() {
                stack.push(x.as_str());
            }
        }
    }
    "gen".into()
}
fn id_prefix_num(id: &str) -> Option<(String, u32)> {
    let (pre, num) = id.rsplit_once('_')?;
    num.parse().ok().map(|n| (pre.to_string(), n))
}

/// Assign `lvl_area_NNN` ids to every id-less item; area is derived by walking the
/// chain up to its intent (so grouping comes from links, not a hand-typed code).
/// Idempotent — items that already have an id are untouched.
pub fn auto_id(app: &mut AppState) -> Result<String> {
    use std::collections::{HashMap, HashSet};
    let mut plan: Vec<(String, usize, usize, String)> = Vec::new();
    {
        let boards = board_pages(app);
        let intents: HashSet<&str> = boards
            .first()
            .map(|(_, c)| c.iter().flat_map(|c| c.items.iter().map(|i| i.text.as_str())).collect())
            .unwrap_or_default();
        let serves: HashMap<&str, &[String]> = boards
            .iter()
            .flat_map(|(_, c)| c.iter().flat_map(|c| c.items.iter().map(|i| (i.text.as_str(), i.serves.as_slice()))))
            .collect();
        let mut maxn: HashMap<String, u32> = HashMap::new();
        let mut used: HashSet<String> = HashSet::new();
        for (_, c) in &boards {
            for it in c.iter().flat_map(|c| &c.items) {
                if let Some(id) = &it.id {
                    used.insert(id.clone());
                    if let Some((pre, n)) = id_prefix_num(id) {
                        let e = maxn.entry(pre).or_insert(0);
                        *e = (*e).max(n);
                    }
                }
            }
        }
        for (pid, cols) in &boards {
            let Some(lvl) = level_code(pid) else { continue };
            for (ci, c) in cols.iter().enumerate() {
                for (ii, it) in c.items.iter().enumerate() {
                    if it.id.is_some() {
                        continue;
                    }
                    let pre = format!("{lvl}_{}", area_of_item(&it.text, &intents, &serves));
                    let mut n = *maxn.get(&pre).unwrap_or(&0) + 1;
                    let mut id = format!("{pre}_{n:03}");
                    while used.contains(&id) {
                        n += 1;
                        id = format!("{pre}_{n:03}");
                    }
                    maxn.insert(pre.clone(), n);
                    used.insert(id.clone());
                    plan.push((pid.to_string(), ci, ii, id));
                }
            }
        }
    }
    if plan.is_empty() {
        return Ok("auto-id: every item already has an id".into());
    }
    let n = plan.len();
    let first = board_pages(app).first().map(|(p, _)| p.to_string()).unwrap_or_else(|| "intent".into());
    app.snapshot(&first); // one undo step for the whole pass
    for (page, col, item, id) in plan {
        if let Some(it) = board_mut(app, &page)?.get_mut(col).and_then(|c| c.items.get_mut(item)) {
            it.id = Some(id);
        }
    }
    app.dirty = true;
    Ok(format!("auto-id: assigned {n} id(s)"))
}

// ---- reconcile: @trace <id> tags in code  vs  the board ----
pub struct Matched {
    pub page: String,
    pub col: usize,
    pub item: usize,
    pub id: String,
    pub locator: String,
}
#[derive(Default)]
pub struct ReconcileReport {
    pub matched: Vec<Matched>,             // tag id found on the board → set its `at`
    pub dangling: Vec<(String, String)>,   // (id, locator) — tag points to no board id
    pub unrealized: Vec<String>,           // control items with an id but no code tag
}

/// Reconcile a map of `id → locator` (scanned from `@trace` tags) against the board.
pub fn reconcile(app: &AppState, tags: &std::collections::HashMap<String, String>) -> ReconcileReport {
    let mut rep = ReconcileReport::default();
    let mut byid: std::collections::HashMap<&str, (String, usize, usize)> = std::collections::HashMap::new();
    for (pid, cols) in board_pages(app) {
        for (ci, c) in cols.iter().enumerate() {
            for (ii, it) in c.items.iter().enumerate() {
                if let Some(id) = &it.id {
                    byid.insert(id.as_str(), (pid.to_string(), ci, ii));
                }
            }
        }
    }
    for (id, locator) in tags {
        match byid.get(id.as_str()) {
            Some((p, c, i)) => rep.matched.push(Matched { page: p.clone(), col: *c, item: *i, id: id.clone(), locator: locator.clone() }),
            None => rep.dangling.push((id.clone(), locator.clone())),
        }
    }
    // unrealized = control-granularity items with an id that no tag references
    for (pid, cols) in board_pages(app) {
        if pid != "control" {
            continue;
        }
        for it in cols.iter().flat_map(|c| &c.items) {
            if let Some(id) = &it.id {
                if !tags.contains_key(id) {
                    rep.unrealized.push(format!("{}  ({id})", trunc(&it.text, 30)));
                }
            }
        }
    }
    rep
}

/// Reconcile + write each matched item's `at` from its code locator.
pub fn sync_at(app: &mut AppState, tags: &std::collections::HashMap<String, String>) -> Result<ReconcileReport> {
    let rep = reconcile(app, tags);
    for m in &rep.matched {
        act_at(app, &m.page, m.col, m.item, vec![m.locator.clone()])?;
    }
    Ok(rep)
}

// ---- 3-way merge: design truth reconciles per ITEM (by title), never per line ----
pub struct MergeOutcome {
    pub app: AppState,
    pub conflicts: Vec<String>,
}

fn union(a: &[String], b: &[String]) -> Vec<String> {
    let mut v = a.to_vec();
    for x in b {
        if !v.contains(x) {
            v.push(x.clone());
        }
    }
    v
}
/// 3-way scalar: take the side that changed from base; both changed → keep ours + conflict.
fn pick3<T: PartialEq + Clone>(base: &Option<T>, ours: &Option<T>, theirs: &Option<T>) -> (Option<T>, bool) {
    if ours == theirs {
        (ours.clone(), false)
    } else if ours == base {
        (theirs.clone(), false)
    } else if theirs == base {
        (ours.clone(), false)
    } else {
        (ours.clone(), true)
    }
}
fn st_word(s: Option<crate::protocol::Status>) -> &'static str {
    s.map(status_word).unwrap_or("n/a")
}
fn board_and_style(app: &AppState, page: &str) -> Option<(Vec<BoardColumn>, BoardStyle)> {
    fn find(n: &UiNode) -> Option<(&Vec<BoardColumn>, BoardStyle)> {
        match n {
            UiNode::Board { columns, style, .. } => Some((columns, *style)),
            UiNode::Panel { children, .. } => children.iter().find_map(find),
            _ => None,
        }
    }
    find(&app.surfaces.get(page)?.root).map(|(c, s)| (c.clone(), s))
}
fn build_surface(page: &str, title: &str, style: BoardStyle, cols: Vec<BoardColumn>) -> crate::protocol::UiMessage {
    crate::protocol::UiMessage::CreateSurface {
        id: page.into(),
        title: title.into(),
        root: UiNode::Panel {
            id: format!("{page}-root"),
            title: None,
            layout: crate::protocol::LayoutKind::Vertical,
            children: vec![UiNode::Board { id: format!("{page}-board"), columns: cols, style }],
        },
    }
}
fn merge_item(
    base: Option<&crate::protocol::Item>,
    ours: &crate::protocol::Item,
    theirs: &crate::protocol::Item,
    addr: &str,
    conflicts: &mut Vec<String>,
) -> crate::protocol::Item {
    let mut it = ours.clone();
    it.serves = union(&ours.serves, &theirs.serves); // additive fields union
    it.note = union(&ours.note, &theirs.note);
    it.attachments = union(&ours.attachments, &theirs.attachments);
    it.at = union(&ours.at, &theirs.at);
    it.derived = ours.derived || theirs.derived;
    it.modified = ours.modified.max(theirs.modified);
    let mut scalar = |label: &str, b: Option<String>, o: &Option<String>, t: &Option<String>| -> Option<String> {
        let (v, c) = pick3(&b, o, t);
        if c {
            conflicts.push(format!("{addr}: {label} (kept ours)"));
        }
        v
    };
    it.category = scalar("category", base.and_then(|b| b.category.clone()), &ours.category, &theirs.category);
    it.id = scalar("id", base.and_then(|b| b.id.clone()), &ours.id, &theirs.id);
    it.comment = scalar("comment", base.and_then(|b| b.comment.clone()), &ours.comment, &theirs.comment);
    it.sha = scalar("sha", base.and_then(|b| b.sha.clone()), &ours.sha, &theirs.sha);
    let (st, c) = pick3(&base.and_then(|b| b.status), &ours.status, &theirs.status);
    if c {
        let worst = rollup(&[ours.status, theirs.status]); // deterministic: worst-wins
        it.status = worst;
        let note = format!("merge: status conflict (ours={}, theirs={}) → {}", st_word(ours.status), st_word(theirs.status), st_word(worst));
        it.comment = Some(it.comment.map(|c| format!("{c}; {note}")).unwrap_or(note));
        conflicts.push(format!("{addr}: status"));
    } else {
        it.status = st;
    }
    it
}
fn merge_items(base: &[crate::protocol::Item], ours: &[crate::protocol::Item], theirs: &[crate::protocol::Item], addr: &str, conflicts: &mut Vec<String>) -> Vec<crate::protocol::Item> {
    let find = |v: &[crate::protocol::Item], t: &str| v.iter().find(|i| i.text == t).cloned();
    let mut titles: Vec<String> = ours.iter().map(|i| i.text.clone()).collect();
    for i in theirs {
        if !titles.contains(&i.text) {
            titles.push(i.text.clone());
        }
    }
    let mut out = Vec::new();
    for t in titles {
        let bo = find(base, &t);
        match (find(ours, &t), find(theirs, &t)) {
            (Some(o), Some(th)) => out.push(merge_item(bo.as_ref(), &o, &th, &format!("{addr}/{t}"), conflicts)),
            (Some(o), None) => {
                if bo.is_none() {
                    out.push(o); // added by ours
                } else if bo.as_ref() != Some(&o) {
                    conflicts.push(format!("{addr}/{t}: modified by ours, deleted by theirs (kept)"));
                    out.push(o);
                } // else deleted by theirs, unchanged ours → drop
            }
            (None, Some(th)) => {
                if bo.is_none() {
                    out.push(th);
                } else if bo.as_ref() != Some(&th) {
                    conflicts.push(format!("{addr}/{t}: modified by theirs, deleted by ours (kept)"));
                    out.push(th);
                }
            }
            (None, None) => {} // base-only → deleted on both
        }
    }
    out
}
fn merge_columns(base: &[BoardColumn], ours: &[BoardColumn], theirs: &[BoardColumn], page: &str, conflicts: &mut Vec<String>) -> Vec<BoardColumn> {
    let key = |c: &BoardColumn| c.key.clone().unwrap_or_else(|| c.header.clone());
    let find = |v: &[BoardColumn], k: &str| v.iter().find(|c| key(c) == k).cloned();
    let mut keys: Vec<String> = ours.iter().map(&key).collect();
    for c in theirs {
        let k = key(c);
        if !keys.contains(&k) {
            keys.push(k);
        }
    }
    let mut out = Vec::new();
    for k in keys {
        let (oc, tc, bc) = (find(ours, &k), find(theirs, &k), find(base, &k));
        let anchor = oc.as_ref().or(tc.as_ref());
        let Some(a) = anchor else { continue };
        let items = merge_items(
            &bc.map(|c| c.items).unwrap_or_default(),
            &oc.as_ref().map(|c| c.items.clone()).unwrap_or_default(),
            &tc.as_ref().map(|c| c.items.clone()).unwrap_or_default(),
            &format!("{page}/{k}"),
            conflicts,
        );
        out.push(BoardColumn { header: a.header.clone(), key: a.key.clone(), items });
    }
    out
}

/// Three-way merge two divergent boards against their common ancestor. Always
/// yields a VALID board; unresolved conflicts are annotated (in `comment`) and
/// listed — never written as text conflict markers (which would corrupt JSONL).
pub fn merge3(base: &AppState, ours: &AppState, theirs: &AppState) -> MergeOutcome {
    let mut conflicts = Vec::new();
    let mut pages = ours.order.clone();
    for p in &theirs.order {
        if !pages.contains(p) {
            pages.push(p.clone());
        }
    }
    let mut app = AppState::default();
    for pid in &pages {
        let ob = board_and_style(ours, pid);
        let tb = board_and_style(theirs, pid);
        if ob.is_none() && tb.is_none() {
            continue; // deleted on both
        }
        let style = ob.as_ref().map(|x| x.1).or_else(|| tb.as_ref().map(|x| x.1)).unwrap_or_default();
        let title = ours.surfaces.get(pid).or_else(|| theirs.surfaces.get(pid)).map(|s| s.title.clone()).unwrap_or_else(|| pid.clone());
        let cols = merge_columns(
            &board_and_style(base, pid).map(|x| x.0).unwrap_or_default(),
            &ob.map(|x| x.0).unwrap_or_default(),
            &tb.map(|x| x.0).unwrap_or_default(),
            pid,
            &mut conflicts,
        );
        let _ = app.apply(build_surface(pid, &title, style, cols));
    }
    MergeOutcome { app, conflicts }
}

pub fn coverage_digest(app: &AppState) -> String {
    let cov = coverage(app);
    if cov.is_empty() {
        return "no spine board".into();
    }
    let a = audit(app);
    // same vocabulary as the UI coverage dashboard (intents/impl/control/tests/…)
    let ctrl = a.layers.get(1).map(|(_, n)| *n).unwrap_or(0);
    let mut out = vec![format!(
        "coverage: {} intents · {} impl · {} control · {} tests · {} covered · {} validated · {} uncovered · {} orphaned · {} derived · {} justified · {} deviations",
        a.total, a.implemented, ctrl, a.verified, a.covered, a.validated, a.gaps.len(), a.orphans.len(), a.derived, a.justified, a.deviations.len()
    )];
    for c in &cov {
        let layers: Vec<String> =
            c.layers.iter().map(|l| format!("{}{}", l.layer, if l.served { "✓" } else { "✗" })).collect();
        let st = c.status.map(|s| format!("  [{}]", status_word(s))).unwrap_or_default();
        out.push(format!("  {} — {}{}", trunc(&c.title, 28), layers.join(" "), st));
    }
    if !a.orphans.is_empty() {
        out.push("orphaned:".into());
        for o in &a.orphans {
            out.push(format!("  ⚠ {o}"));
        }
    }
    if !a.deviations.is_empty() {
        out.push("deviations (justified, no rationale):".into());
        for d in &a.deviations {
            out.push(format!("  ⚠ {d}"));
        }
    }
    out.join("\n")
}

// --- addressing: page[/col[/item]] -------------------------------------------
// `col` is an index (2) or a stable key (why). `item` is an index (0) or `+`
// (the add slot). Keys/`+` give absolute jumps that don't depend on counting.
struct Addr<'a> {
    page: &'a str,
    col: Option<&'a str>,
    item: Option<&'a str>,
}
fn parse_addr(s: &str) -> Addr<'_> {
    let mut p = s.split('/');
    Addr {
        page: p.next().unwrap_or(""),
        col: p.next().filter(|s| !s.is_empty()),
        item: p.next().filter(|s| !s.is_empty()),
    }
}

/// Resolve a column token (index or key) to an index.
fn resolve_col(cols: &[BoardColumn], tok: &str) -> Result<usize> {
    if let Ok(i) = tok.parse::<usize>() {
        if i < cols.len() {
            return Ok(i);
        }
    }
    cols.iter()
        .position(|c| c.key.as_deref() == Some(tok))
        .ok_or_else(|| anyhow::anyhow!("no column '{tok}'"))
}

/// Resolve an item token: index, or `+` = the trailing add slot.
fn resolve_item(col: &BoardColumn, tok: &str) -> Result<usize> {
    match tok {
        "+" => Ok(col.items.len()),
        t => t.parse::<usize>().map_err(|_| anyhow::anyhow!("bad item '{t}'")),
    }
}

/// Resolve a full `page/col/item` address to (page, col index, item index).
fn resolve3(app: &AppState, ad: &Addr) -> Result<(String, usize, usize)> {
    // A bare token (no `/col/item`) is a global id or title — the efficient address:
    // edit by the stable id you assigned, no observe round-trip, and it survives
    // reordering (identity = id / title, not position).
    let Some(col_tok) = ad.col else {
        return resolve_any(app, ad.page);
    };
    let page = ad.page.to_string();
    let cols = board(app, &page).ok_or_else(|| anyhow::anyhow!("no board on '{page}'"))?;
    let col = resolve_col(cols, col_tok)?;
    let item = resolve_item(&cols[col], ad.item.ok_or_else(|| anyhow::anyhow!("need an item"))?)?;
    Ok((page, col, item))
}

/// Resolve a bare token to (page, col, item) by `id` first, then by title — so any
/// item-targeting verb can address by the stable id or the visible title.
fn resolve_any(app: &AppState, token: &str) -> Result<(String, usize, usize)> {
    let t = token.trim_matches('"');
    for by_id in [true, false] {
        for (pid, cols) in board_pages(app) {
            for (ci, c) in cols.iter().enumerate() {
                for (ii, it) in c.items.iter().enumerate() {
                    let hit = if by_id { it.id.as_deref() == Some(t) } else { it.text == t };
                    if hit {
                        return Ok((pid.to_string(), ci, ii));
                    }
                }
            }
        }
    }
    Err(anyhow::anyhow!("no item with id/title '{token}' (use page/col/item, an id, or a \"title\")"))
}

/// Parse a status word; "" / na / clear → None (n/a).
fn parse_status(s: &str) -> Result<Option<crate::protocol::Status>> {
    use crate::protocol::Status;
    Ok(match s {
        "ok" => Some(Status::Ok),
        "ko" | "fail" => Some(Status::Fail),
        "pending" => Some(Status::Pending),
        "justified" => Some(Status::Justified),
        "na" | "clear" => None, // explicit clear only — empty is a mistake, not a wipe
        "" => bail!("status needs a value: ok|ko|pending|justified|na"),
        o => bail!("bad status '{o}' (ok|ko|pending|justified|na)"),
    })
}

fn find_primary(node: &UiNode) -> Option<&UiNode> {
    match node {
        UiNode::Board { .. } | UiNode::Table { .. } => Some(node),
        UiNode::Panel { children, .. } => children.iter().find_map(find_primary),
        _ => None,
    }
}
/// Truncate to `n` chars with an ellipsis (shared with the renderer).
pub fn trunc(s: &str, n: usize) -> String {
    let c: Vec<char> = s.chars().collect();
    if c.len() <= n {
        s.to_string()
    } else if n == 0 {
        String::new()
    } else {
        c[..n - 1].iter().collect::<String>() + "…"
    }
}

fn style_word(s: BoardStyle) -> &'static str {
    match s {
        BoardStyle::Cards => "cards",
        BoardStyle::Panel => "panel",
        BoardStyle::Sheet => "sheet",
        BoardStyle::List => "list",
    }
}

/// Set a page's board render style (list/cards).
fn set_page_style(app: &mut AppState, page: &str, style: BoardStyle) -> Result<()> {
    fn walk(n: &mut UiNode, style: BoardStyle) -> bool {
        match n {
            UiNode::Board { style: st, .. } => {
                *st = style;
                true
            }
            UiNode::Panel { children, .. } => children.iter_mut().any(|c| walk(c, style)),
            _ => false,
        }
    }
    let s = app.surfaces.get_mut(page).ok_or_else(|| anyhow::anyhow!("no page '{page}'"))?;
    if walk(&mut s.root, style) {
        app.dirty = true;
        Ok(())
    } else {
        bail!("no board on '{page}'")
    }
}

// --- OBSERVE (read) ----------------------------------------------------------
/// Digest the store at a depth chosen by `addr`: none = all pages, `page` =
/// columns, `page/col` = items, `page/col/item` = one item.
pub fn observe(app: &AppState, addr: Option<&str>) -> String {
    match addr {
        None => digest_all(app),
        Some(a) => {
            let ad = parse_addr(a);
            match (ad.col, ad.item) {
                (None, _) => digest_page(app, ad.page),
                (Some(c), None) => digest_col(app, ad.page, c),
                (Some(c), Some(i)) => digest_item(app, ad.page, c, i),
            }
        }
    }
}

fn digest_all(app: &AppState) -> String {
    // First contact for many agents — point them at the protocol/manual.
    let mut out = vec!["# arte-tui — run `usage` for the protocol; map before you edit".to_string()];
    for (i, id) in app.order.iter().enumerate() {
        let Some(s) = app.surfaces.get(id) else { continue };
        let mark = if i == app.ui.active { "*" } else { " " };
        let body = match find_primary(&s.root) {
            Some(UiNode::Board { columns, style, .. }) => {
                let items: usize = columns.iter().map(|c| c.items.len()).sum();
                let mut b = format!("board[{}] {}c {}i", style_word(*style), columns.len(), items);
                if i == app.ui.active {
                    // cursor: row 0 = header, else item row-1.
                    let cur = app.ui.cursor;
                    let pos = if cur.row == 0 { "h".to_string() } else { (cur.row - 1).to_string() };
                    b += &format!(" sel={}/{}", cur.col, pos);
                }
                b
            }
            Some(UiNode::Table { columns, rows, .. }) => {
                format!("table {}x{}", rows.len(), columns.len())
            }
            _ => "other".into(),
        };
        out.push(format!("{mark}{id} {body}"));
    }
    // The working/highlight set is part of the live observable state.
    if !app.ui.working.is_empty() {
        let mut w: Vec<&str> = app.ui.working.iter().map(String::as_str).collect();
        w.sort();
        out.push(format!("~work {}", w.join(", ")));
    }
    out.join("\n")
}

fn digest_page(app: &AppState, page: &str) -> String {
    let Some(s) = app.surfaces.get(page) else { return format!("? no page {page}") };
    match find_primary(&s.root) {
        Some(UiNode::Board { columns, style, .. }) => {
            let mut out = vec![format!("{page} board[{}]", style_word(*style))];
            for (c, col) in columns.iter().enumerate() {
                // show the stable handle (key) so the agent can address absolutely
                let handle = col.key.clone().unwrap_or_else(|| c.to_string());
                out.push(format!("  {handle} {} ({})", trunc(&col.header, 28), col.items.len()));
            }
            out.join("\n")
        }
        _ => digest_all_one(app, page),
    }
}

fn digest_all_one(app: &AppState, page: &str) -> String {
    // fall back to the one-line summary for non-board pages
    let i = app.order.iter().position(|x| x == page).unwrap_or(0);
    digest_all(app).lines().nth(i).unwrap_or("?").to_string()
}

fn digest_col(app: &AppState, page: &str, ctok: &str) -> String {
    let Some(cols) = board(app, page) else { return format!("? no page {page}") };
    let Ok(c) = resolve_col(cols, ctok) else { return format!("? no {page}/{ctok}") };
    let col = &cols[c];
    let handle = col.key.clone().unwrap_or_else(|| c.to_string());
    let mut out = vec![format!("{page}/{handle}")];
    out.push(format!("  h  {}", col.header)); // header is row 'h'
    for (i, item) in col.items.iter().enumerate() {
        out.push(format!("  {i}  {}{}", item.text, item_suffix(item)));
    }
    out.join("\n")
}

/// Trailing link/status info for observe: "⇒ a, b [pending]".
fn item_suffix(item: &crate::protocol::Item) -> String {
    let mut s = String::new();
    if let Some(i) = &item.id {
        s += &format!("  #{i}");
    }
    if let Some(c) = &item.category {
        s += &format!("  ·{c}");
    }
    if !item.serves.is_empty() {
        s += &format!("  ⇒ {}", item.serves.join(", "));
    }
    if let Some(st) = item.status {
        s += &format!("  [{}]", status_word(st));
    }
    if !item.note.is_empty() {
        s += " *"; // has note(s)
    }
    if !item.attachments.is_empty() {
        s += &format!(" @{}", item.attachments.len()); // attachment count
    }
    if let Some(c) = &item.comment {
        s += &format!("  ~{c}"); // result comment
    }
    if item.derived {
        s += "  (derived)";
    }
    if let Some(sha) = &item.sha {
        s += &format!("  @{}", sha.chars().take(8).collect::<String>());
    }
    s
}

use crate::state::now_secs;

/// Human age for a unix-secs timestamp: relative within a week, then the date.
pub fn rel_age(t: u64) -> String {
    let d = now_secs().saturating_sub(t);
    if d < 60 {
        "just now".into()
    } else if d < 3600 {
        format!("{}m ago", d / 60)
    } else if d < 86400 {
        format!("{}h ago", d / 3600)
    } else if d < 7 * 86400 {
        let days = d / 86400;
        format!("{days} day{} ago", if days == 1 { "" } else { "s" })
    } else {
        ymd(t) // past a week → show the date
    }
}

/// Date in the machine's locale + local timezone (strftime "%x"), unix only.
#[cfg(unix)]
fn ymd(secs: u64) -> String {
    // setlocale is process-global — pick up LC_TIME from the env ONCE (thread-safe,
    // not per-frame), not on every render. `localtime_r` is reentrant.
    static LOCALE: std::sync::Once = std::sync::Once::new();
    LOCALE.call_once(|| unsafe {
        libc::setlocale(libc::LC_TIME, b"\0".as_ptr().cast());
    });
    // SAFETY: fixed buffer, null-terminated formats; localtime_r is reentrant.
    unsafe {
        let t = secs as libc::time_t;
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&t, &mut tm).is_null() {
            return ymd_iso(secs);
        }
        let mut buf = [0u8; 64];
        let n = libc::strftime(buf.as_mut_ptr().cast(), buf.len(), b"%x\0".as_ptr().cast(), &tm);
        if n == 0 {
            return ymd_iso(secs);
        }
        String::from_utf8_lossy(&buf[..n]).into_owned()
    }
}
#[cfg(not(unix))]
fn ymd(secs: u64) -> String {
    ymd_iso(secs)
}

/// Civil date `YYYY-MM-DD` (UTC) from unix seconds — Hinnant's algorithm, no deps.
fn ymd_iso(secs: u64) -> String {
    let z = (secs / 86400) as i64 + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + if month <= 2 { 1 } else { 0 };
    format!("{year:04}-{month:02}-{day:02}")
}

fn status_word(s: crate::protocol::Status) -> &'static str {
    match s {
        crate::protocol::Status::Ok => "ok",
        crate::protocol::Status::Fail => "ko",
        crate::protocol::Status::Pending => "pending",
        crate::protocol::Status::Justified => "justified",
    }
}

fn digest_item(app: &AppState, page: &str, ctok: &str, itok: &str) -> String {
    let Some(cols) = board(app, page) else { return format!("? no page {page}") };
    let Ok(c) = resolve_col(cols, ctok) else { return format!("? no {page}/{ctok}") };
    if itok == "h" {
        return format!("{page}/{ctok}/h {}", cols[c].header); // the header cell
    }
    match resolve_item(&cols[c], itok).ok().and_then(|i| cols[c].items.get(i)) {
        Some(item) => {
            let mut s = format!("{page}/{ctok}/{itok} {}{}", item.text, item_suffix(item));
            if let Some(t) = item.modified {
                s += &format!("\n  modified: {}", rel_age(t));
            }
            for n in &item.note {
                s += &format!("\n  note: {n}");
            }
            for a in &item.attachments {
                s += &format!("\n  @ {a}");
            }
            for loc in &item.at {
                s += &format!("\n  at {loc}");
            }
            s
        }
        None => format!("? no {page}/{ctok}/{itok}"),
    }
}

fn board<'a>(app: &'a AppState, page: &str) -> Option<&'a [BoardColumn]> {
    app.surfaces.get(page).and_then(|s| match find_primary(&s.root) {
        Some(UiNode::Board { columns, .. }) => Some(columns.as_slice()),
        _ => None,
    })
}

// --- ACT (drive) -------------------------------------------------------------
fn board_mut<'a>(app: &'a mut AppState, page: &str) -> Result<&'a mut Vec<BoardColumn>> {
    app.surfaces
        .get_mut(page)
        .and_then(|s| crate::state::find_board_mut(&mut s.root))
        .ok_or_else(|| anyhow::anyhow!("no board on page '{page}'"))
}

/// Switch the active page.
pub fn act_page(app: &mut AppState, page: &str) -> Result<String> {
    let i = app.order.iter().position(|x| x == page).ok_or_else(|| anyhow::anyhow!("no page '{page}'"))?;
    app.set_active(i);
    Ok(observe(app, None))
}

/// Move selection to a board item (switches page if needed).
pub fn act_sel(app: &mut AppState, page: &str, col: usize, item: usize) -> Result<String> {
    act_page(app, page)?;
    app.select_board_item(col, item);
    Ok(observe(app, Some(page)))
}

// Every act_* below snapshots for undo itself, so undo works for ANY caller
// (the terse wire OR a direct MCP call) — the PARSE layer never snapshots.

/// Append an item to a column.
pub fn act_add(app: &mut AppState, page: &str, col: usize, text: &str) -> Result<String> {
    act_add_full(app, page, col, text.to_string(), Vec::new(), None)
}

/// Append an item with serves links + status in one shot (the compound add).
pub fn act_add_full(
    app: &mut AppState,
    page: &str,
    col: usize,
    text: String,
    mut serves: Vec<String>,
    status: Option<crate::protocol::Status>,
) -> Result<String> {
    serves.sort();
    serves.dedup();
    app.snapshot(page);
    if let Some(c) = board_mut(app, page)?.get_mut(col) {
        let mut it = crate::protocol::Item { text, serves, status, ..Default::default() };
        it.touch();
        c.items.push(it);
        crate::state::dedupe_items(c);
    }
    app.dirty = true;
    let idx = board(app, page).map(|cs| cs[col].items.len().saturating_sub(1)).unwrap_or(0);
    Ok(digest_item(app, page, &col.to_string(), &idx.to_string()))
}

/// Replace an item's text (rename cascades serves links).
pub fn act_set(app: &mut AppState, page: &str, col: usize, item: usize, text: &str) -> Result<String> {
    act_set_full(app, page, col, item, text.to_string(), None, None)
}

/// Edit text and optionally serves/status in one shot (`None` = leave as is).
pub fn act_set_full(
    app: &mut AppState,
    page: &str,
    col: usize,
    item: usize,
    text: String,
    serves: Option<Vec<String>>,
    status: Option<Option<crate::protocol::Status>>,
) -> Result<String> {
    app.snapshot(page);
    let old = board(app, page).and_then(|cs| cs.get(col)).and_then(|c| c.items.get(item)).map(|i| i.text.clone());
    if let Some(it) = board_mut(app, page)?.get_mut(col).and_then(|c| c.items.get_mut(item)) {
        it.text = text.clone();
        if let Some(s) = serves {
            it.serves = s;
        }
        if let Some(st) = status {
            it.status = st;
        }
        it.touch();
    }
    if let Some(old) = old {
        app.rename_intent(&old, &text); // cascade serves links that named the old text
    }
    app.dirty = true;
    Ok(digest_item(app, page, &col.to_string(), &item.to_string()))
}

/// Snapshot, mutate the addressed item with `f`, stamp + dirty — the shared body
/// for the single-field item edits (link/status/note/attach/cat).
fn edit_item(app: &mut AppState, page: &str, col: usize, item: usize, f: impl FnOnce(&mut crate::protocol::Item)) -> Result<String> {
    let before = app.done_work_titles(); // retire only what THIS edit finishes
    app.snapshot(page);
    if let Some(it) = board_mut(app, page)?.get_mut(col).and_then(|c| c.items.get_mut(item)) {
        f(it);
        it.touch();
    }
    app.dirty = true;
    app.retire_newly_done(&before); // a task stops pulsing only when this edit completes it
    Ok(digest_item(app, page, &col.to_string(), &item.to_string()))
}

/// Set an item's serves links (replaces; empty clears).
pub fn act_link(app: &mut AppState, page: &str, col: usize, item: usize, mut titles: Vec<String>) -> Result<String> {
    titles.sort();
    titles.dedup();
    edit_item(app, page, col, item, move |it| it.serves = titles)
}
/// Set an item's producer status (`None` = n/a).
pub fn act_status(app: &mut AppState, page: &str, col: usize, item: usize, status: Option<crate::protocol::Status>) -> Result<String> {
    edit_item(app, page, col, item, move |it| it.status = status)
}
/// Set an item's note lines (replaces; empty clears).
pub fn act_note(app: &mut AppState, page: &str, col: usize, item: usize, lines: Vec<String>) -> Result<String> {
    edit_item(app, page, col, item, move |it| it.note = lines)
}
/// Set an item's attachment refs (replaces; empty clears).
pub fn act_attach(app: &mut AppState, page: &str, col: usize, item: usize, refs: Vec<String>) -> Result<String> {
    edit_item(app, page, col, item, move |it| it.attachments = refs)
}
/// Set an item's realization locator(s) — `file#unit` (replaces; empty clears).
pub fn act_at(app: &mut AppState, page: &str, col: usize, item: usize, locs: Vec<String>) -> Result<String> {
    edit_item(app, page, col, item, move |it| it.at = locs)
}
/// Set an item's category tag (`None` clears).
pub fn act_cat(app: &mut AppState, page: &str, col: usize, item: usize, category: Option<String>) -> Result<String> {
    edit_item(app, page, col, item, move |it| it.category = category)
}
/// `level_area_NNN` shape: 4 lowercase + 3 lowercase (functional trigram) + 3
/// digits, e.g. `tsts_sco_001`. The tool judges the STRUCTURE; the producer
/// asserts the substance (which level / functional area the codes mean).
fn valid_id_shape(s: &str) -> bool {
    let p: Vec<&str> = s.split('_').collect();
    p.len() == 3
        && p[0].len() == 4
        && p[0].chars().all(|c| c.is_ascii_lowercase())
        && p[1].len() == 3
        && p[1].chars().all(|c| c.is_ascii_lowercase())
        && p[2].len() == 3
        && p[2].chars().all(|c| c.is_ascii_digit())
}

/// The functional-area trigram of an id (the middle `_abc_` segment), if shaped.
pub fn area_of(id: &str) -> Option<&str> {
    let p: Vec<&str> = id.split('_').collect();
    (p.len() == 3 && p[1].len() == 3).then(|| p[1])
}

/// Functional-area trigrams in use → how many items use each. Derived from ids;
/// the pop-up is just the area-filter picker (no separate glossary page).
pub fn glossary(app: &AppState) -> Vec<(String, usize)> {
    let mut counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for (_, cols) in board_pages(app) {
        for it in cols.iter().flat_map(|c| &c.items) {
            if let Some(a) = it.id.as_deref().and_then(area_of) {
                *counts.entry(a.to_string()).or_default() += 1;
            }
        }
    }
    counts.into_iter().collect()
}

/// Functional-area trigrams already in use (the middle segment of valid ids),
/// so the agent reuses an existing code instead of inventing a synonym.
fn id_areas(app: &AppState) -> Vec<String> {
    let mut v: Vec<String> = board_pages(app)
        .iter()
        .flat_map(|(_, cols)| cols.iter().flat_map(|c| &c.items))
        .filter_map(|it| it.id.as_deref())
        .filter(|s| valid_id_shape(s))
        .map(|s| s.split('_').nth(1).unwrap_or("").to_string())
        .collect();
    v.sort();
    v.dedup();
    v
}

/// Set an item's stable reference id (`None` clears). Validates the
/// `level_area_NNN` shape and rejects a duplicate (uniqueness).
pub fn act_id(app: &mut AppState, page: &str, col: usize, item: usize, id: Option<String>) -> Result<String> {
    if let Some(new) = id.as_deref() {
        if !valid_id_shape(new) {
            let areas = id_areas(app);
            let hint = if areas.is_empty() { String::new() } else { format!("; areas in use: {}", areas.join(", ")) };
            bail!("id must be lvl_abc_NNN (innt|impl|ctrl|tsts _ functional trigram _ 3 digits), got '{new}'{hint}");
        }
        let dup = board_pages(app).iter().any(|(pid, cols)| {
            cols.iter().enumerate().any(|(ci, c)| {
                c.items.iter().enumerate().any(|(ii, it)| {
                    it.id.as_deref() == Some(new) && !(*pid == page && ci == col && ii == item)
                })
            })
        });
        if dup {
            bail!("id '{new}' already in use");
        }
    }
    edit_item(app, page, col, item, move |it| it.id = id)
}
/// Set an item's result comment (`None` clears).
pub fn act_comment(app: &mut AppState, page: &str, col: usize, item: usize, comment: Option<String>) -> Result<String> {
    edit_item(app, page, col, item, move |it| it.comment = comment)
}
/// Mark/unmark an item as a derived requirement (parentless by design, not an orphan).
pub fn act_derive(app: &mut AppState, page: &str, col: usize, item: usize, on: bool) -> Result<String> {
    edit_item(app, page, col, item, move |it| it.derived = on)
}
/// Set the commit SHA the item was tested/verified against (`None` clears).
pub fn act_sha(app: &mut AppState, page: &str, col: usize, item: usize, sha: Option<String>) -> Result<String> {
    edit_item(app, page, col, item, move |it| it.sha = sha)
}

/// Append a column (pillar). `key` is its stable handle (optional).
pub fn act_addcol(app: &mut AppState, page: &str, header: &str, key: Option<&str>) -> Result<String> {
    app.snapshot(page);
    let cols = board_mut(app, page)?;
    cols.push(BoardColumn { header: header.to_string(), key: key.map(|s| s.to_string()), items: vec![] });
    app.dirty = true;
    Ok(observe(app, Some(page)))
}

/// Remove a column (pillar) and all its items.
pub fn act_delcol(app: &mut AppState, page: &str, col: usize) -> Result<String> {
    app.snapshot(page);
    let cols = board_mut(app, page)?;
    if col >= cols.len() {
        bail!("no column {col}");
    }
    cols.remove(col);
    app.dirty = true;
    Ok(observe(app, Some(page)))
}

/// Reorder an item within its column by `delta` (clamped at the edges).
pub fn act_move(app: &mut AppState, page: &str, col: usize, item: usize, delta: isize) -> Result<String> {
    app.snapshot(page);
    let cols = board_mut(app, page)?;
    let c = cols.get_mut(col).ok_or_else(|| anyhow::anyhow!("no column {col}"))?;
    let n = c.items.len();
    let target = item as isize + delta;
    if item < n && target >= 0 && (target as usize) < n {
        c.items.swap(item, target as usize);
        app.dirty = true;
    }
    Ok(observe(app, Some(&format!("{page}/{col}"))))
}

/// Rename a column's header (its guiding prompt).
pub fn act_setcol(app: &mut AppState, page: &str, col: usize, header: &str) -> Result<String> {
    app.snapshot(page);
    let cols = board_mut(app, page)?;
    let c = cols.get_mut(col).ok_or_else(|| anyhow::anyhow!("no column {col}"))?;
    c.header = header.to_string();
    app.dirty = true;
    Ok(observe(app, Some(page)))
}

fn parse_style(s: &str) -> Result<BoardStyle> {
    match s {
        "cards" => Ok(BoardStyle::Cards),
        "panel" => Ok(BoardStyle::Panel),
        "sheet" => Ok(BoardStyle::Sheet),
        "list" | "" => Ok(BoardStyle::List),
        other => bail!("style = list|cards|panel|sheet (got '{other}')"),
    }
}

/// Create a new board page (a layer): empty columns, chosen render style.
pub fn act_addpage(app: &mut AppState, id: &str, title: &str, style: BoardStyle) -> Result<String> {
    if id.is_empty() {
        bail!("addpage needs <id>");
    }
    if app.surfaces.contains_key(id) {
        bail!("page '{id}' already exists");
    }
    let title = if title.is_empty() { id } else { title };
    app.apply(UiMessage::CreateSurface {
        id: id.to_string(),
        title: title.to_string(),
        root: UiNode::Board { id: format!("{id}_b"), columns: Vec::new(), style },
    })?;
    app.dirty = true;
    Ok(observe(app, None))
}

/// Switch a page's board render style (list ↔ cards).
pub fn act_setstyle(app: &mut AppState, page: &str, style: BoardStyle) -> Result<String> {
    set_page_style(app, page, style)?;
    Ok(observe(app, Some(page)))
}

/// Rename a page's display title; its `id` (the address) stays stable.
pub fn act_renamepage(app: &mut AppState, id: &str, title: &str) -> Result<String> {
    app.surfaces.get_mut(id).ok_or_else(|| anyhow::anyhow!("no page '{id}'"))?.title = title.to_string();
    app.dirty = true;
    Ok(observe(app, None))
}

/// Delete a page and drop it from the order; clamp the active page.
pub fn act_delpage(app: &mut AppState, id: &str) -> Result<String> {
    if app.surfaces.remove(id).is_none() {
        bail!("no page '{id}'");
    }
    app.order.retain(|x| x != id);
    if app.ui.active >= app.order.len() {
        app.ui.active = app.order.len().saturating_sub(1);
    }
    app.dirty = true;
    Ok(observe(app, None))
}

/// Delete an item.
pub fn act_del(app: &mut AppState, page: &str, col: usize, item: usize) -> Result<String> {
    app.snapshot(page);
    let cols = board_mut(app, page)?;
    let c = cols.get_mut(col).ok_or_else(|| anyhow::anyhow!("no column {col}"))?;
    if item >= c.items.len() {
        bail!("no item {item}");
    }
    c.items.remove(item);
    app.dirty = true;
    Ok(observe(app, Some(&format!("{page}/{col}"))))
}

// --- PARSE layer (terse wire). MCP skips this and calls act_*/observe. -------
/// Run one terse command line. Confirmation is the returned digest (the delta).
pub fn dispatch(app: &mut AppState, line: &str) -> Result<String> {
    // retire a worked intent that THIS command finishes, whatever the verb (status,
    // add of the final validation case, link, …) — one chokepoint for every command.
    let before = app.done_work_titles();
    let out = dispatch_inner(app, line);
    app.retire_newly_done(&before);
    out
}
fn dispatch_inner(app: &mut AppState, line: &str) -> Result<String> {
    let line = line.trim();
    let (verb, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
    let rest = rest.trim();
    // payload text is the quoted span, if any
    let text = rest.find('"').and_then(|a| rest.rfind('"').filter(|b| *b > a).map(|b| &rest[a + 1..b]));
    let addr_str = rest.split('"').next().unwrap_or(rest).trim();
    let ad = parse_addr(addr_str);

    match verb {
        "usage" | "help" => return Ok(USAGE.to_string()),
        "observe" => return Ok(observe(app, if addr_str.is_empty() { None } else { Some(addr_str) })),
        "coverage" => return Ok(coverage_digest(app)),
        "trace" | "depends" => return Ok(trace(app, addr_str)), // depends = back-compat alias
        "page" => return act_page(app, ad.page),
        // Page/layer ops — build the map itself, not just its contents. dispatch
        // only PARSES; the real work is in the act_* execute layer (MCP-reusable).
        "addpage" => {
            let parts: Vec<&str> = rest.split('"').collect();
            let id = parts.first().map(|s| s.trim()).unwrap_or("");
            let title = parts.get(1).map(|s| s.trim()).unwrap_or("");
            let style = parse_style(parts.get(2).map(|s| s.trim()).unwrap_or(""))?;
            return act_addpage(app, id, title, style);
        }
        "setstyle" => {
            let mut p = rest.split_whitespace();
            let page = p.next().unwrap_or("");
            return act_setstyle(app, page, parse_style(p.next().unwrap_or(""))?);
        }
        "renamepage" => {
            let parts: Vec<&str> = rest.split('"').collect();
            let id = parts.first().map(|s| s.trim()).unwrap_or("");
            let title = parts.get(1).filter(|s| !s.is_empty()).ok_or_else(|| anyhow::anyhow!("renamepage needs \"title\""))?;
            return act_renamepage(app, id, title);
        }
        "delpage" => return act_delpage(app, rest.trim()),
        "undo" => return Ok(if app.undo() { observe(app, None) } else { "nothing to undo".into() }),
        "redo" => return Ok(if app.redo() { observe(app, None) } else { "nothing to redo".into() }),
        // addcol: the column doesn't exist yet, so don't resolve it. `ad.col` (if
        // given) is the NEW key; text is the header.
        "addcol" => {
            let page = ad.page.to_string();
            return act_addcol(app, &page, text.ok_or_else(|| anyhow::anyhow!("addcol needs \"header\""))?, ad.col);
        }
        // work "intent title"…: pulse every item that IS or `serves` these intents,
        // across all layers. Replaces the set; `work` with no titles clears it.
        "work" => {
            app.ui.working =
                rest.split('"').skip(1).step_by(2).map(str::to_string).filter(|s| !s.is_empty()).collect();
            return Ok(observe(app, None));
        }
        // link <addr> "title" ["title2"…]: set an item's `serves` links (replaces).
        // `link <addr>` with no titles clears them.
        // dispatch only PARSES; the act_* execute layer does the work (+ snapshot).
        "link" => {
            let titles: Vec<String> =
                rest.split('"').skip(1).step_by(2).map(str::to_string).filter(|s| !s.is_empty()).collect();
            let (page, col, item) = resolve3(app, &ad)?;
            return act_link(app, &page, col, item, titles);
        }
        "status" => {
            let mut parts = rest.split_whitespace();
            let m = parse_addr(parts.next().unwrap_or(""));
            let st = parse_status(parts.next().unwrap_or(""))?;
            let (page, col, item) = resolve3(app, &m)?;
            return act_status(app, &page, col, item, st);
        }
        "note" | "attach" | "at" => {
            let m = parse_addr(rest.split('"').next().unwrap_or("").trim());
            let quoted: Vec<String> =
                rest.split('"').skip(1).step_by(2).map(str::to_string).filter(|s| !s.is_empty()).collect();
            let (page, col, item) = resolve3(app, &m)?;
            return match verb {
                "note" => act_note(app, &page, col, item, quoted),
                "attach" => act_attach(app, &page, col, item, quoted),
                _ => act_at(app, &page, col, item, quoted),
            };
        }
        "cat" | "category" => {
            let m = parse_addr(rest.split('"').next().unwrap_or("").trim());
            let val = rest.split('"').nth(1).map(str::to_string).filter(|s| !s.is_empty());
            let (page, col, item) = resolve3(app, &m)?;
            return act_cat(app, &page, col, item, val);
        }
        "id" => {
            let m = parse_addr(rest.split('"').next().unwrap_or("").trim());
            let val = rest.split('"').nth(1).map(str::to_string).filter(|s| !s.is_empty());
            let (page, col, item) = resolve3(app, &m)?;
            return act_id(app, &page, col, item, val);
        }
        "comment" => {
            let m = parse_addr(rest.split('"').next().unwrap_or("").trim());
            let val = rest.split('"').nth(1).map(str::to_string).filter(|s| !s.is_empty());
            let (page, col, item) = resolve3(app, &m)?;
            return act_comment(app, &page, col, item, val);
        }
        "derive" => {
            let mut parts = rest.split_whitespace();
            let m = parse_addr(parts.next().unwrap_or(""));
            let on = !matches!(parts.next(), Some("off" | "no" | "false" | "clear"));
            let (page, col, item) = resolve3(app, &m)?;
            return act_derive(app, &page, col, item, on);
        }
        "sha" => {
            let m = parse_addr(rest.split('"').next().unwrap_or("").trim());
            let val = rest.split('"').nth(1).map(str::to_string).filter(|s| !s.is_empty());
            let (page, col, item) = resolve3(app, &m)?;
            return act_sha(app, &page, col, item, val);
        }
        // mv <addr> <delta>: reorder. item defaults to 0, so it can't use resolve3.
        "mv" => {
            let mut parts = rest.split_whitespace();
            let m = parse_addr(parts.next().unwrap_or(""));
            let delta: isize = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
            let page = m.page.to_string();
            let cols = board(app, &page).ok_or_else(|| anyhow::anyhow!("no board on '{page}'"))?;
            let col = resolve_col(cols, m.col.ok_or_else(|| anyhow::anyhow!("need a column"))?)?;
            let item = m.item.map(|t| resolve_item(&cols[col], t)).transpose()?.unwrap_or(0);
            return act_move(app, &page, col, item, delta);
        }
        // Compound add/set — one shot per element, works on any column (pillar or
        // card): `add <col> "text" ["serve title"…] [status ok|ko|pending|na]`,
        // `set <addr> "text" […]`. `set <col>/h "name"` still renames the header.
        "add" | "set" => {
            let parts: Vec<&str> = rest.split('"').collect();
            let m = parse_addr(parts.first().map(|s| s.trim()).unwrap_or(""));
            let quoted: Vec<&str> = parts.iter().skip(1).step_by(2).copied().collect();
            let text =
                quoted.first().copied().ok_or_else(|| anyhow::anyhow!("{verb} needs \"text\""))?.to_string();
            let serves: Vec<String> = quoted.iter().skip(1).map(|s| s.to_string()).collect();
            let serves_given = quoted.len() > 1;
            // status = the bare token after `status`, in the unquoted segments.
            let structural: String = parts.iter().step_by(2).copied().collect::<Vec<_>>().join(" ");
            let stoks: Vec<&str> = structural.split_whitespace().collect();
            let status_pos = stoks.iter().position(|t| *t == "status");
            let status = match status_pos {
                Some(p) => parse_status(stoks.get(p + 1).copied().unwrap_or("na"))?,
                None => None,
            };

            let page = m.page.to_string();
            // resolve col (+ item for set) while the immutable board borrow is live
            let (col, item_opt) = {
                let cols = board(app, &page).ok_or_else(|| anyhow::anyhow!("no board on '{page}'"))?;
                let col = resolve_col(cols, m.col.ok_or_else(|| anyhow::anyhow!("need a column"))?)?;
                let item = if verb == "set" && m.item != Some("h") {
                    Some(resolve_item(&cols[col], m.item.ok_or_else(|| anyhow::anyhow!("need an item"))?)?)
                } else {
                    None
                };
                (col, item)
            };
            if verb == "set" && m.item == Some("h") {
                return act_setcol(app, &page, col, &text); // rename the header
            }
            if verb == "add" {
                return act_add_full(app, &page, col, text, serves, status);
            }
            // set existing item: serves/status applied only when given (None = leave)
            return act_set_full(
                app,
                &page,
                col,
                item_opt.unwrap(),
                text,
                serves_given.then_some(serves),
                status_pos.is_some().then_some(status),
            );
        }
        _ => {}
    }

    // Reject an unknown verb BEFORE touching the address, so a typo'd command gets
    // "unknown command" (with a pointer to usage), not a confusing address error.
    if !matches!(verb, "sel" | "del" | "delcol" | "setcol") {
        bail!("unknown command '{verb}' — run `usage` for the verb list");
    }
    // Mutating/selecting verbs: resolve the address (key or index) first.
    // Row `h` addresses the column header (the grid's row 0).
    let page = ad.page.to_string();
    let cols = board(app, &page).ok_or_else(|| anyhow::anyhow!("no board on '{page}'"))?;
    let col = resolve_col(cols, ad.col.ok_or_else(|| anyhow::anyhow!("need a column"))?)?;
    let on_header = ad.item == Some("h");
    let item = if on_header {
        0
    } else {
        ad.item.map(|t| resolve_item(&cols[col], t)).transpose()?.unwrap_or(0)
    };

    match verb {
        "sel" if on_header => {
            act_page(app, &page)?;
            app.ui.cursor = crate::state::Cursor { col, row: 0 };
            Ok(observe(app, Some(&page)))
        }
        "sel" => act_sel(app, &page, col, item),
        "del" if on_header => act_delcol(app, &page, col),
        "del" => act_del(app, &page, col, item),
        "delcol" => act_delcol(app, &page, col),
        "setcol" => act_setcol(app, &page, col, text.ok_or_else(|| anyhow::anyhow!("setcol needs \"header\""))?),
        other => bail!("unknown verb '{other}'"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::UiMessage;

    fn app() -> AppState {
        let mut a = AppState::default();
        a.apply(UiMessage::CreateSurface {
            id: "intent".into(),
            title: "intent".into(),
            root: UiNode::Board {
                id: "b".into(),
                style: Default::default(),
                columns: vec![
                    BoardColumn { header: "WHO".into(), key: Some("who".into()), items: vec!["a".into()] },
                    BoardColumn { header: "JOBS".into(), key: Some("jobs".into()), items: vec!["x".into(), "y".into()] },
                ],
            },
        })
        .unwrap();
        a
    }

    #[test]
    fn observe_is_compact() {
        let a = app();
        let d = observe(&a, None);
        assert!(d.contains("intent board[list] 2c 3i"));
        assert!(d.lines().any(|l| l.starts_with('#') && l.contains("usage")), "points to the protocol");
        // the digest itself (sans the one-line protocol pointer) stays terse
        let body: String = d.lines().filter(|l| !l.starts_with('#')).collect::<Vec<_>>().join("\n");
        assert!(body.len() < 60, "terse: {body:?}");
    }

    #[test]
    fn act_add_then_observe_roundtrip() {
        let mut a = app();
        dispatch(&mut a, "add intent/0 \"New\"").unwrap();
        assert!(a.dirty);
        assert!(observe(&a, Some("intent/0")).contains("New"));
    }

    #[test]
    fn act_set_and_del() {
        let mut a = app();
        dispatch(&mut a, "set intent/1/0 \"zz\"").unwrap();
        assert!(observe(&a, Some("intent/1")).contains("zz"));
        dispatch(&mut a, "del intent/1/0").unwrap();
        // "y" was item 1, now item 0; "zz" gone
        assert!(!observe(&a, Some("intent/1")).contains("zz"));
    }

    #[test]
    fn direct_act_call_is_undoable() {
        // MCP seam: act_* snapshot themselves, so undo works WITHOUT dispatch.
        let mut a = app();
        act_status(&mut a, "intent", 1, 0, Some(crate::protocol::Status::Ok)).unwrap();
        assert!(observe(&a, Some("intent/1")).contains("[ok]"));
        assert!(a.undo(), "act_status registered an undo snapshot");
        assert!(!observe(&a, Some("intent/1")).contains("[ok]"), "status reverted");
    }

    #[test]
    fn address_by_key() {
        let mut a = app();
        // absolute jump by column key (page/key) — no index counting
        dispatch(&mut a, "add intent/jobs \"by-key\"").unwrap();
        assert!(observe(&a, Some("intent/jobs")).contains("by-key"));
    }

    #[test]
    fn reorder_item() {
        let mut a = app(); // jobs has x, y
        dispatch(&mut a, "mv intent/jobs/0 1").unwrap(); // move x down
        let j = observe(&a, Some("intent/jobs"));
        // lines: [title, "h header", "0 …", "1 …"] — item 0 is now "y".
        let item0 = j.lines().find(|l| l.trim_start().starts_with("0 ")).unwrap_or("");
        assert!(item0.contains("y"));
    }

    #[test]
    fn work_sets_and_clears_intent_titles() {
        let mut a = app();
        dispatch(&mut a, "work \"Build the radar\" \"Norms editor\"").unwrap();
        assert_eq!(a.ui.working.len(), 2);
        assert!(a.ui.working.contains("Build the radar"));
        // the working set must be observable, not write-only
        assert!(observe(&a, None).contains("~work"), "observe surfaces the working set");
        assert!(observe(&a, None).contains("Build the radar"));
        dispatch(&mut a, "work").unwrap(); // no titles → clear
        assert!(a.ui.working.is_empty());
        assert!(!observe(&a, None).contains("~work"));
    }

    #[test]
    fn coverage_reports_per_intent_layer_gaps() {
        use crate::protocol::{Item, Status};
        let mut a = AppState::default();
        let board = |id: &str, cols: Vec<BoardColumn>| UiMessage::CreateSurface {
            id: id.into(),
            title: id.into(),
            root: UiNode::Board { id: format!("{id}b"), columns: cols, style: Default::default() },
        };
        let it = |t: &str, serves: &[&str], status| Item {
            text: t.into(),
            serves: serves.iter().map(|s| s.to_string()).collect(),
            status,
            ..Default::default()
        };
        // spine (first board) = intents A, B
        a.apply(board("intent", vec![BoardColumn { header: "WHY".into(), key: Some("why".into()), items: vec![Item::new("A"), Item::new("B")] }])).unwrap();
        // chain: impl serves intent A (pending); control serves the impl item; nothing serves B
        a.apply(board("impl", vec![BoardColumn { header: "HOW".into(), key: None, items: vec![it("build A", &["A"], Some(Status::Pending))] }])).unwrap();
        a.apply(board("control", vec![BoardColumn { header: "RULES".into(), key: None, items: vec![it("guard A", &["build A"], None)] }])).unwrap();

        let cov = coverage(&a);
        let a_cov = cov.iter().find(|c| c.title == "A").unwrap();
        assert!(a_cov.whole, "A served down the chain (impl←control)");
        assert_eq!(a_cov.status, Some(Status::Pending), "status rolls up");
        let b_cov = cov.iter().find(|c| c.title == "B").unwrap();
        assert!(!b_cov.whole && b_cov.layers.iter().all(|l| !l.served), "B has gaps");

        let dg = dispatch(&mut a, "coverage").unwrap();
        assert!(dg.contains("A — impl✓ control✓"), "{dg}");
        assert!(dg.contains("B — impl✗ control✗"), "{dg}");

        // val item links straight to the intent (skipping control) → chain doesn't count it
        a.apply(board("val", vec![BoardColumn { header: "PROOF".into(), key: None, items: vec![it("X", &["A"], None)] }])).unwrap();
        let cov = coverage(&a);
        let a_val = cov.iter().find(|c| c.title == "A").unwrap().layers.iter().find(|l| l.layer == "val").unwrap();
        assert!(!a_val.served, "a direct-to-intent skip is not a valid chain link");
    }

    #[test]
    fn compound_add_links_and_stamps_in_one() {
        let mut a = app();
        // add an element to a pillar with serves + status in a single command
        dispatch(&mut a, "add intent/who \"Clinicians\" \"5-axis eval\" status pending").unwrap();
        let out = observe(&a, Some("intent/who"));
        assert!(out.contains("Clinicians"), "{out}");
        assert!(out.contains("⇒ 5-axis eval"), "serves set: {out}");
        assert!(out.contains("[pending]"), "status set: {out}");
        // extra quoted titles beyond the text are all serves
        dispatch(&mut a, "add intent/who \"Multi\" \"A\" \"B\"").unwrap();
        assert!(observe(&a, Some("intent/who")).contains("⇒ A, B"));
    }

    #[test]
    fn compound_set_updates_text_serves_status_and_header() {
        let mut a = app();
        dispatch(&mut a, "set intent/jobs/0 \"Do Y\" \"5-axis eval\" status ok").unwrap();
        let out = observe(&a, Some("intent/jobs/0"));
        assert!(out.contains("Do Y") && out.contains("⇒ 5-axis eval") && out.contains("[ok]"), "{out}");
        // a plain set (no extra quotes / no status) keeps serves+status intact
        dispatch(&mut a, "set intent/jobs/0 \"Do Z\"").unwrap();
        let out = observe(&a, Some("intent/jobs/0"));
        assert!(out.contains("Do Z") && out.contains("⇒ 5-axis eval") && out.contains("[ok]"), "kept: {out}");
        // set on /h still renames the column header
        dispatch(&mut a, "set intent/who/h \"USERS\"").unwrap();
        assert!(observe(&a, Some("intent")).contains("USERS"));
    }

    #[test]
    fn compound_add_works_on_a_cards_board() {
        use crate::protocol::{BoardColumn, BoardStyle};
        let mut a = AppState::default();
        a.apply(UiMessage::CreateSurface {
            id: "impl".into(),
            title: "impl".into(),
            root: UiNode::Board {
                id: "m".into(),
                style: BoardStyle::Cards,
                columns: vec![BoardColumn { header: "Frontend".into(), key: Some("fe".into()), items: vec![] }],
            },
        })
        .unwrap();
        dispatch(&mut a, "add impl/fe \"App shell\" \"5-axis eval\" status ok").unwrap();
        let out = observe(&a, Some("impl/fe"));
        assert!(out.contains("App shell") && out.contains("⇒ 5-axis eval") && out.contains("[ok]"), "{out}");
    }

    #[test]
    fn agent_fills_and_edits_a_whole_design_via_the_wire() {
        let mut a = AppState::default();
        let ok = |a: &mut AppState, s: &str| dispatch(a, s).unwrap_or_else(|e| panic!("{s}: {e}"));
        // build the 4 layers from empty — agent can construct the map, not just fill it
        ok(&mut a, "addpage intent \"intent.map\"");
        ok(&mut a, "addpage impl \"impl.arch\" cards");
        ok(&mut a, "addpage control \"control.spec\" panel");
        ok(&mut a, "addpage validation \"validation.rep\"");
        for s in ["addcol intent/jobs \"JOBS\"", "addcol impl/be \"Backend\"", "addcol control/rules \"RULES\"", "addcol validation/proof \"PROOF\""] {
            ok(&mut a, s);
        }
        // fill the chain + EVERY field on one control
        ok(&mut a, "add intent/jobs \"Do X\"");
        ok(&mut a, "add impl/be \"Engine\" \"Do X\" status ok");
        ok(&mut a, "add control/rules \"axis 0-4\" \"Engine\" status pending");
        ok(&mut a, "cat control/rules/0 \"guards\"");
        ok(&mut a, "note control/rules/0 \"clamp inputs\"");
        ok(&mut a, "attach control/rules/0 \"spec.pdf\"");
        ok(&mut a, "add validation/proof \"mean(4,4,4)=100\" \"axis 0-4\" status ok");
        // every field is observable
        let o = observe(&a, Some("control/rules/0"));
        for s in ["axis 0-4", "·guards", "⇒ Engine", "[pending]", "note: clamp inputs", "@ spec.pdf"] {
            assert!(o.contains(s), "observe missing {s:?}:\n{o}");
        }
        // the chain rolls up: intent←impl←control←validation
        assert!(coverage(&a).into_iter().find(|c| c.title == "Do X").unwrap().whole, "Do X fully covered");
        // EDIT every field (renaming text cascades to the validation link)
        ok(&mut a, "set control/rules/0 \"axis 0-4 scaled\"");
        ok(&mut a, "status control/rules/0 ok");
        ok(&mut a, "cat control/rules/0 \"validation\"");
        ok(&mut a, "note control/rules/0"); // clear
        ok(&mut a, "setstyle impl list");
        ok(&mut a, "renamepage control \"rules.spec\"");
        let o2 = observe(&a, Some("control/rules/0"));
        assert!(o2.contains("axis 0-4 scaled") && o2.contains("[ok]") && o2.contains("·validation") && !o2.contains("note:"), "edits:\n{o2}");
        assert!(coverage(&a).into_iter().find(|c| c.title == "Do X").unwrap().whole, "rename cascaded → chain intact");
        // delete item · column · page
        ok(&mut a, "del validation/proof/0");
        ok(&mut a, "del validation/proof/h"); // delcol
        ok(&mut a, "delpage validation");
        assert!(!observe(&a, None).contains("validation board"), "page deleted");
        ok(&mut a, "undo"); // history works
    }

    #[test]
    fn justified_test_is_accepted_not_a_failure() {
        let mut a = AppState::default();
        let ok = |a: &mut AppState, s: &str| dispatch(a, s).unwrap_or_else(|e| panic!("{s}: {e}"));
        ok(&mut a, "addpage intent \"i\"");
        ok(&mut a, "addcol intent/g \"G\"");
        ok(&mut a, "add intent/g \"I\"");
        ok(&mut a, "addpage impl \"m\" cards");
        ok(&mut a, "addcol impl/c \"C\"");
        ok(&mut a, "add impl/c \"Comp\" \"I\" status ok");
        ok(&mut a, "addpage control \"ctl\" panel");
        ok(&mut a, "addcol control/r \"R\"");
        ok(&mut a, "add control/r \"rule\" \"Comp\" status ok");
        ok(&mut a, "addpage validation \"val\"");
        ok(&mut a, "addcol validation/p \"P\"");
        ok(&mut a, "add validation/p \"flaky test\" \"rule\" status justified");
        ok(&mut a, "note validation/p/0 \"intermittent in CI; accepted\"");
        let cov = coverage(&a).into_iter().find(|c| c.title == "I").unwrap();
        assert!(cov.whole, "chain complete through the justified test");
        // a justified (accepted) failure surfaces but does NOT read as a failure
        assert_eq!(cov.status, Some(crate::protocol::Status::Justified));
        assert_ne!(cov.status, Some(crate::protocol::Status::Fail));
    }

    #[test]
    fn audit_validated_and_orphaned_stale_link() {
        use crate::protocol::{BoardColumn, BoardStyle, Item, Status, UiNode};
        let board = |id: &str, style: BoardStyle, items: Vec<Item>| UiMessage::CreateSurface {
            id: id.into(),
            title: id.into(),
            root: UiNode::Board { id: format!("{id}b"), columns: vec![BoardColumn { header: "h".into(), key: None, items }], style },
        };
        let item = |text: &str, serves: &[&str], status: Option<Status>, modified: Option<u64>| Item {
            text: text.into(),
            serves: serves.iter().map(|s| s.to_string()).collect(),
            status,
            modified,
            ..Default::default()
        };
        let mut a = AppState::default();
        a.apply(board("intent", BoardStyle::List, vec![item("I", &[], None, None)])).unwrap();
        a.apply(board("impl", BoardStyle::Cards, vec![item("Comp", &["I"], Some(Status::Ok), Some(100))])).unwrap();
        a.apply(board("validation", BoardStyle::Sheet, vec![item("Test", &["Comp"], Some(Status::Ok), Some(50))])).unwrap();
        let au = audit(&a);
        assert_eq!(au.validated, 1, "I rolls up green (impl ok, test ok)");
        // a stale link (parent changed after the child) counts as orphaned
        assert!(au.orphans.iter().any(|o| o.contains("Test")), "stale link orphaned: {:?}", au.orphans);
    }

    #[test]
    fn edit_by_bare_id_and_title() {
        let mut a = AppState::default();
        let ok = |a: &mut AppState, s: &str| dispatch(a, s).unwrap_or_else(|e| panic!("{s}: {e}"));
        ok(&mut a, "addpage val \"v\" sheet");
        ok(&mut a, "addcol val/proof \"P\"");
        ok(&mut a, "add val/proof \"unit test\"");
        ok(&mut a, "id val/proof/0 \"tsts_abc_001\"");
        // address by stable id — no page/col/item, no observe round-trip
        ok(&mut a, "status tsts_abc_001 ok");
        // address by single-word title
        ok(&mut a, "add val/proof \"smoke\"");
        ok(&mut a, "status smoke pending");
        let cols = board(&a, "val").unwrap();
        assert_eq!(cols[0].items[0].status, Some(crate::protocol::Status::Ok), "id addressing hit the right item");
        assert_eq!(cols[0].items[1].status, Some(crate::protocol::Status::Pending), "title addressing works");
    }

    #[test]
    fn finished_work_stops_pulsing() {
        let mut a = AppState::default();
        let ok = |a: &mut AppState, s: &str| dispatch(a, s).unwrap_or_else(|e| panic!("{s}: {e}"));
        ok(&mut a, "addpage intent \"i\"");
        ok(&mut a, "addcol intent/g \"G\"");
        ok(&mut a, "add intent/g \"I\"");
        ok(&mut a, "addpage impl \"m\" cards");
        ok(&mut a, "addcol impl/c \"C\"");
        ok(&mut a, "add impl/c \"Comp\" \"I\"");
        ok(&mut a, "addpage control \"ctl\" panel");
        ok(&mut a, "addcol control/r \"R\"");
        ok(&mut a, "add control/r \"rule\" \"Comp\"");
        ok(&mut a, "addpage validation \"val\" sheet");
        ok(&mut a, "addcol validation/p \"P\"");
        ok(&mut a, "add validation/p \"test\" \"rule\"");
        ok(&mut a, "work \"I\"");
        assert!(a.ui.working.contains("I"), "focused → pulsing");
        // partial: only impl ok → not finished → still pulsing
        ok(&mut a, "status impl/c/0 ok");
        assert!(a.ui.working.contains("I"), "not yet whole+green → still pulsing");
        // complete the chain → fully covered + green → auto-retired
        ok(&mut a, "status control/r/0 ok");
        ok(&mut a, "status validation/p/0 ok");
        assert!(!a.ui.working.contains("I"), "finished task stops pulsing on its own");
    }

    #[test]
    fn focused_done_item_survives_unrelated_edits() {
        let mut a = AppState::default();
        let ok = |a: &mut AppState, s: &str| dispatch(a, s).unwrap_or_else(|e| panic!("{s}: {e}"));
        ok(&mut a, "addpage intent \"i\"");
        ok(&mut a, "addcol intent/g \"G\"");
        ok(&mut a, "add intent/g \"I\"");
        ok(&mut a, "addpage impl \"m\" cards");
        ok(&mut a, "addcol impl/c \"C\"");
        ok(&mut a, "add impl/c \"Comp\" \"I\"");
        ok(&mut a, "status impl/c/0 ok");
        ok(&mut a, "addpage control \"ctl\" panel");
        ok(&mut a, "addcol control/r \"R\"");
        ok(&mut a, "add control/r \"rule\" \"Comp\"");
        ok(&mut a, "status control/r/0 ok");
        ok(&mut a, "addpage validation \"val\" sheet");
        ok(&mut a, "addcol validation/p \"P\"");
        ok(&mut a, "add validation/p \"test\" \"rule\"");
        ok(&mut a, "status validation/p/0 ok"); // "I" is now fully done
        ok(&mut a, "work \"I\""); // deliberately focus an already-done item
        assert!(a.ui.working.contains("I"), "focusing a done item still pulses");
        // an unrelated edit must NOT silently clear that focus (it wasn't newly finished)
        ok(&mut a, "note validation/p/0 \"ran again\"");
        assert!(a.ui.working.contains("I"), "stays pulsing — only a task you just FINISH retires");
    }

    #[test]
    fn auto_id_derives_area_from_chain() {
        let mut a = AppState::default();
        let ok = |a: &mut AppState, s: &str| dispatch(a, s).unwrap_or_else(|e| panic!("{s}: {e}"));
        ok(&mut a, "addpage intent \"i\"");
        ok(&mut a, "addcol intent/g \"G\"");
        ok(&mut a, "add intent/g \"Polls\"");
        ok(&mut a, "addpage control \"c\" panel");
        ok(&mut a, "addcol control/r \"R\"");
        ok(&mut a, "add control/r \"one vote\" \"Polls\"");
        auto_id(&mut a).unwrap();
        assert!(observe(&a, Some("control/r/0")).contains("ctrl_pol_001"), "area 'pol' derived from served intent Polls");
        assert!(observe(&a, Some("intent/g/0")).contains("innt_pol_001"));
        // idempotent: a second pass adds nothing
        assert!(auto_id(&mut a).unwrap().contains("already"));
    }

    #[test]
    fn reconcile_matches_dangling_unrealized() {
        use std::collections::HashMap;
        let mut a = AppState::default();
        let ok = |a: &mut AppState, s: &str| dispatch(a, s).unwrap_or_else(|e| panic!("{s}: {e}"));
        ok(&mut a, "addpage intent \"i\"");
        ok(&mut a, "addcol intent/g \"G\"");
        ok(&mut a, "add intent/g \"Polls\"");
        ok(&mut a, "addpage control \"c\" panel");
        ok(&mut a, "addcol control/r \"R\"");
        ok(&mut a, "add control/r \"one vote\" \"Polls\"");
        ok(&mut a, "add control/r \"untagged rule\" \"Polls\"");
        auto_id(&mut a).unwrap();
        let tags: HashMap<String, String> =
            [("ctrl_pol_001".into(), "src/x.rs#L5".into()), ("ctrl_zzz_999".into(), "y.rs#L1".into())].into_iter().collect();
        let rep = reconcile(&a, &tags);
        assert_eq!(rep.matched.len(), 1, "ctrl_pol_001 matched");
        assert_eq!(rep.dangling.len(), 1, "ctrl_zzz_999 dangles");
        assert_eq!(rep.unrealized.len(), 1, "the untagged control is unrealized");
        // sync writes the locator onto the matched item
        sync_at(&mut a, &tags).unwrap();
        assert!(observe(&a, Some("control/r/0")).contains("at src/x.rs#L5"));
    }

    #[test]
    fn merge3_unions_links_and_resolves_status_conflict() {
        let board = |extra: &[&str]| {
            let mut a = AppState::default();
            for s in [
                "addpage intent \"i\"",
                "addcol intent/g \"G\"",
                "add intent/g \"I\"",
                "add intent/g \"J\"",
                "addpage control \"c\" panel",
                "addcol control/r \"R\"",
                "add control/r \"rule\" \"I\"",
            ]
            .iter()
            .chain(extra)
            {
                dispatch(&mut a, s).unwrap_or_else(|e| panic!("{s}: {e}"));
            }
            a
        };
        let base = board(&[]);
        let ours = board(&["status control/r/0 ok"]);
        let theirs = board(&["status control/r/0 ko", "link control/r/0 \"I\" \"J\""]);
        let out = merge3(&base, &ours, &theirs);
        let v = observe(&out.app, Some("control/r/0"));
        assert!(v.contains("[ko]"), "status conflict resolves worst-wins (ko): {v}");
        assert!(v.contains("conflict"), "conflict annotated in comment: {v}");
        assert!(out.app.surfaces.contains_key("control"));
        // serves union: ours [I] + theirs [I,J] → both
        assert!(out.conflicts.iter().any(|c| c.contains("status")), "status flagged: {:?}", out.conflicts);
    }

    #[test]
    fn gate_requires_backed_focus() {
        use std::collections::HashSet;
        let mut a = AppState::default();
        let ok = |a: &mut AppState, s: &str| dispatch(a, s).unwrap_or_else(|e| panic!("{s}: {e}"));
        ok(&mut a, "addpage intent \"i\"");
        ok(&mut a, "addcol intent/g \"G\"");
        ok(&mut a, "add intent/g \"I\"");
        assert!(!gate(&a, &HashSet::new()).0, "empty focus is blocked");
        let f: HashSet<String> = ["I".to_string()].into_iter().collect();
        assert!(!gate(&a, &f).0, "intent with no impl/control is blocked");
        ok(&mut a, "addpage impl \"m\" cards");
        ok(&mut a, "addcol impl/c \"C\"");
        ok(&mut a, "add impl/c \"Comp\" \"I\"");
        ok(&mut a, "addpage control \"ctl\" panel");
        ok(&mut a, "addcol control/r \"R\"");
        ok(&mut a, "add control/r \"rule\" \"Comp\"");
        assert!(gate(&a, &f).0, "intent backed by impl + control is allowed");
        let bad: HashSet<String> = ["Nope".to_string()].into_iter().collect();
        assert!(!gate(&a, &bad).0, "unknown intent is blocked");
    }

    #[test]
    fn derived_clears_an_orphan() {
        let mut a = AppState::default();
        let ok = |a: &mut AppState, s: &str| dispatch(a, s).unwrap_or_else(|e| panic!("{s}: {e}"));
        ok(&mut a, "addpage intent \"i\"");
        ok(&mut a, "addcol intent/g \"G\"");
        ok(&mut a, "add intent/g \"I\"");
        ok(&mut a, "addpage control \"ctl\" panel");
        ok(&mut a, "addcol control/r \"R\"");
        ok(&mut a, "add control/r \"a rule\""); // no serves → orphan
        assert!(audit(&a).orphans.iter().any(|o| o.contains("a rule")), "orphan before");
        assert_eq!(audit(&a).derived, 0);
        ok(&mut a, "derive control/r/0");
        let au = audit(&a);
        assert!(!au.orphans.iter().any(|o| o.contains("a rule")), "no longer orphan: {:?}", au.orphans);
        assert_eq!(au.derived, 1, "counted as derived");
        ok(&mut a, "derive control/r/0 off"); // toggling off restores the orphan
        assert!(audit(&a).orphans.iter().any(|o| o.contains("a rule")));
    }

    #[test]
    fn audit_flags_gaps_orphans_and_deviations() {
        let mut a = AppState::default();
        let ok = |a: &mut AppState, s: &str| dispatch(a, s).unwrap_or_else(|e| panic!("{s}: {e}"));
        ok(&mut a, "addpage intent \"i\"");
        ok(&mut a, "addcol intent/g \"G\"");
        ok(&mut a, "add intent/g \"I1\"");
        ok(&mut a, "add intent/g \"I2\""); // will be uncovered
        ok(&mut a, "addpage impl \"m\" cards");
        ok(&mut a, "addcol impl/c \"C\"");
        ok(&mut a, "add impl/c \"Comp\" \"I1\"");
        ok(&mut a, "addpage validation \"val\" sheet");
        ok(&mut a, "addcol validation/p \"P\"");
        ok(&mut a, "add validation/p \"t1\" \"Comp\" status justified"); // justified, no comment → deviation
        ok(&mut a, "add validation/p \"floating\""); // no serves → orphan
        let au = audit(&a);
        assert_eq!(au.total, 2);
        assert!(au.gaps.iter().any(|g| g.contains("I2")), "I2 uncovered: {:?}", au.gaps);
        assert!(au.orphans.iter().any(|o| o.contains("floating")), "orphan: {:?}", au.orphans);
        assert!(au.deviations.iter().any(|d| d.contains("t1")), "deviation: {:?}", au.deviations);
    }

    #[test]
    fn glossary_codes_and_counts() {
        assert_eq!(area_of("tsts_sco_001"), Some("sco"));
        assert_eq!(area_of("nope"), None);
        let mut a = AppState::default();
        let ok = |a: &mut AppState, s: &str| dispatch(a, s).unwrap_or_else(|e| panic!("{s}: {e}"));
        ok(&mut a, "addpage validation \"val\" sheet");
        ok(&mut a, "addcol validation/p \"P\"");
        ok(&mut a, "add validation/p \"t1\"");
        ok(&mut a, "id validation/p/0 \"tsts_sco_001\"");
        ok(&mut a, "add validation/p \"t2\"");
        ok(&mut a, "id validation/p/1 \"tsts_sco_002\"");
        ok(&mut a, "add validation/p \"t3\"");
        ok(&mut a, "id validation/p/2 \"tsts_nrm_001\"");
        let g = glossary(&a); // derived from ids in use — no glossary page
        assert_eq!(g.iter().find(|(c, _)| c == "sco").unwrap().1, 2, "two cases use sco");
        assert_eq!(g.iter().find(|(c, _)| c == "nrm").unwrap().1, 1);
    }

    #[test]
    fn trace_walks_the_chain_upward() {
        let mut a = AppState::default();
        let ok = |a: &mut AppState, s: &str| dispatch(a, s).unwrap_or_else(|e| panic!("{s}: {e}"));
        ok(&mut a, "addpage intent \"i\"");
        ok(&mut a, "addcol intent/g \"G\"");
        ok(&mut a, "add intent/g \"Top\"");
        ok(&mut a, "addpage impl \"m\" cards");
        ok(&mut a, "addcol impl/c \"C\"");
        ok(&mut a, "add impl/c \"Comp\" \"Top\"");
        ok(&mut a, "addpage control \"ctl\" panel");
        ok(&mut a, "addcol control/r \"R\"");
        ok(&mut a, "add control/r \"rule\" \"Comp\"");
        ok(&mut a, "addpage validation \"val\" sheet");
        ok(&mut a, "addcol validation/p \"P\"");
        ok(&mut a, "add validation/p \"the test\" \"rule\"");
        ok(&mut a, "id validation/p/0 \"tsts_top_001\"");
        // reverse trace from the test (by id) reaches the control, impl, and intent
        let d = dispatch(&mut a, "trace tsts_top_001").unwrap();
        for s in ["the test", "⇒ rule", "⇒ Comp", "⇒ Top"] {
            assert!(d.contains(s), "trace missing {s:?}:\n{d}");
        }
        assert!(dispatch(&mut a, "depends tsts_top_001").is_ok(), "depends still works as an alias");
        // an orphan is reported
        ok(&mut a, "add validation/p \"floating\"");
        assert!(dispatch(&mut a, "trace validation/p/1").unwrap().contains("orphan"));
    }

    #[test]
    fn validation_case_fields_and_id_uniqueness() {
        let mut a = AppState::default();
        let ok = |a: &mut AppState, s: &str| dispatch(a, s).unwrap_or_else(|e| panic!("{s}: {e}"));
        ok(&mut a, "addpage validation \"validation.rep\"");
        ok(&mut a, "addcol validation/proof \"PROOF\"");
        ok(&mut a, "add validation/proof \"Scoring self-check\" status ok");
        ok(&mut a, "id validation/proof/0 \"tsts_sco_001\"");
        ok(&mut a, "note validation/proof/0 \"feed 4,4,4 expect 100\""); // what it does
        ok(&mut a, "comment validation/proof/0 \"green since v2\""); // remark on result
        ok(&mut a, "attach validation/proof/0 \"core/src/agent.rs:1100\""); // test code link
        let o = observe(&a, Some("validation/proof/0"));
        for s in ["#tsts_sco_001", "[ok]", "~green since v2", "note: feed 4,4,4", "@ core/src/agent.rs:1100"] {
            assert!(o.contains(s), "observe missing {s:?}:\n{o}");
        }
        // a malformed id is rejected (shape), and a duplicate is rejected (uniqueness)
        ok(&mut a, "add validation/proof \"Another\"");
        assert!(dispatch(&mut a, "id validation/proof/1 \"TC-9\"").is_err(), "bad shape rejected");
        assert!(dispatch(&mut a, "id validation/proof/1 \"tsts_sco_001\"").is_err(), "duplicate rejected");
        dispatch(&mut a, "id validation/proof/1 \"tsts_sco_002\"").unwrap();
    }

    #[test]
    fn usage_documents_the_drive_verbs() {
        // guard against the manual drifting from the verbs an agent can call
        for v in [
            "observe", "coverage", "trace", "snapshot", "add", "set", "link", "status", "note", "attach", "at", "cat",
            "id", "comment", "derive", "sha", "work", "mv", "del", "addcol", "setcol", "delcol", "addpage",
            "setstyle", "renamepage", "delpage", "sel", "page", "undo", "redo",
        ] {
            assert!(USAGE.contains(v), "usage omits the verb: {v}");
        }
    }

    #[test]
    fn iso_date_from_unix_secs() {
        assert_eq!(ymd_iso(0), "1970-01-01");
        assert_eq!(ymd_iso(1_700_000_000), "2023-11-14");
    }

    #[test]
    fn note_and_attach_ride_on_item() {
        let mut a = app();
        dispatch(&mut a, "note intent/jobs/0 \"use radix sort\"").unwrap();
        dispatch(&mut a, "attach intent/jobs/0 \"spec.pdf\" \"diagram.png\"").unwrap();
        let out = observe(&a, Some("intent/jobs/0"));
        assert!(out.contains("note: use radix sort"), "{out}");
        assert!(out.contains("@ spec.pdf") && out.contains("@ diagram.png"), "{out}");
        // list digest shows compact markers
        let list = observe(&a, Some("intent/jobs"));
        assert!(list.contains(" *") && list.contains("@2"), "{list}");
        // bare note clears
        dispatch(&mut a, "note intent/jobs/0").unwrap();
        assert!(!observe(&a, Some("intent/jobs/0")).contains("note:"));
    }

    #[test]
    fn status_sets_and_clears() {
        let mut a = app();
        dispatch(&mut a, "status intent/jobs/0 pending").unwrap();
        assert!(observe(&a, Some("intent/jobs/0")).contains("[pending]"));
        dispatch(&mut a, "status intent/jobs/0 ok").unwrap();
        assert!(observe(&a, Some("intent/jobs/0")).contains("[ok]"));
        dispatch(&mut a, "status intent/jobs/0 na").unwrap();
        assert!(!observe(&a, Some("intent/jobs/0")).contains("["));
        assert!(dispatch(&mut a, "status intent/jobs/0 bogus").is_err());
    }

    #[test]
    fn link_sets_serves_on_item() {
        let mut a = app();
        dispatch(&mut a, "link intent/jobs/0 \"WHO IT'S FOR\"").unwrap();
        let out = observe(&a, Some("intent/jobs/0"));
        assert!(out.contains("⇒") && out.contains("WHO IT'S FOR"), "serves shown: {out}");
        dispatch(&mut a, "link intent/jobs/0").unwrap(); // clear
        assert!(!observe(&a, Some("intent/jobs/0")).contains("⇒"));
    }

    #[test]
    fn header_addressable_as_row_h() {
        let mut a = app();
        // observe shows the header as row "h"
        assert!(observe(&a, Some("intent/who")).contains("h  WHO"));
        // set / del via the /h row maps to header / column ops
        dispatch(&mut a, "set intent/who/h \"WHO NOW\"").unwrap();
        assert!(observe(&a, Some("intent/who/h")).contains("WHO NOW"));
        dispatch(&mut a, "del intent/who/h").unwrap(); // deletes the column
        assert!(!observe(&a, Some("intent")).contains("who"));
    }

    #[test]
    fn pillar_add_delete_undo_redo_via_chat() {
        let mut a = app(); // 2 columns
        let n = |a: &AppState| match find_primary(&a.surfaces["intent"].root) {
            Some(UiNode::Board { columns, .. }) => columns.len(),
            _ => 0,
        };
        let c0 = n(&a);
        dispatch(&mut a, "addcol intent \"RISKS\"").unwrap();
        assert_eq!(n(&a), c0 + 1);
        dispatch(&mut a, "undo").unwrap();
        assert_eq!(n(&a), c0); // add undone
        dispatch(&mut a, "redo").unwrap();
        assert_eq!(n(&a), c0 + 1); // re-added
        dispatch(&mut a, "delcol intent/0").unwrap();
        assert_eq!(n(&a), c0);
        dispatch(&mut a, "undo").unwrap();
        assert_eq!(n(&a), c0 + 1); // delete undone
    }

    #[test]
    fn columns_are_modular() {
        let mut a = app();
        // add a pillar with a key, rename it, then remove it
        dispatch(&mut a, "addcol intent/risks \"RISKS — what could go wrong\"").unwrap();
        assert!(observe(&a, Some("intent")).contains("risks"));
        dispatch(&mut a, "setcol intent/risks \"RISKS & MITIGATIONS\"").unwrap();
        assert!(observe(&a, Some("intent/risks")).contains("MITIGATIONS"));
        dispatch(&mut a, "delcol intent/risks").unwrap();
        assert!(!observe(&a, Some("intent")).contains("risks"));
    }

    #[test]
    fn undo_redo_roundtrip() {
        let mut a = app();
        dispatch(&mut a, "add intent/who \"temp\"").unwrap();
        assert!(observe(&a, Some("intent/who")).contains("temp"));
        assert!(a.undo());
        assert!(!observe(&a, Some("intent/who")).contains("temp"));
        assert!(a.redo());
        assert!(observe(&a, Some("intent/who")).contains("temp"));
    }
}
