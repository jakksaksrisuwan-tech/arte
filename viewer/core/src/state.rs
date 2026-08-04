//! App state: surfaces keyed by id, plus the order they arrived so the renderer
//! can show the latest one. Messages mutate state through `apply`.

use std::collections::{HashMap, VecDeque};

use anyhow::{anyhow, bail, Result};

use crate::protocol::{BoardStyle, UiMessage, UiNode, UiPatch};
use crate::validate::validate_tree;

#[derive(Debug)]
pub struct Surface {
    pub title: String,
    pub root: UiNode,
}

/// The document: persisted data + edit history. No view/interaction state.
#[derive(Debug, Default)]
pub struct AppState {
    pub surfaces: HashMap<String, Surface>,
    /// Insertion/update order; last entry is the latest surface to render.
    pub order: Vec<String>,
    /// Set when an edit changed the data and the store should be re-saved.
    pub dirty: bool,
    /// Undo/redo: snapshots of (page, board columns) before each data edit.
    /// A ring (drop oldest past UNDO_DEPTH) — VecDeque so the cap is O(1).
    undo: VecDeque<Snapshot>,
    redo: VecDeque<Snapshot>,
    /// Ephemeral interaction state — not persisted (lives in ui.rs).
    pub ui: Ui,
}

/// The single inspector edit in progress (mutually exclusive by construction).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InspectorMode {
    #[default]
    None,
    /// Picking the element's `linked_to` (serves) from the parent layer.
    LinkPick,
    /// Typing a note line (continue-add).
    Note,
    /// Typing a new attachment ref (continue-add).
    Attach,
    /// Editing the category tag (combobox).
    Category,
    /// Editing the result comment.
    Comment,
    /// Editing the item's text (the "details").
    Detail,
}

/// View/interaction state: where the cursor is, what's on screen, edit mode.
/// Separate from the document so the truth and the view don't entangle.
#[derive(Debug, Default)]
pub struct Ui {
    /// Index into `order` of the page currently shown (page routing).
    pub active: usize,
    /// Board cursor: a formal grid coordinate. `col` = pillar; `row` = position
    /// inside that pillar's table (0 = header, 1..=N = item, N+1 = the "+").
    pub cursor: Cursor,
    /// Interaction mode for the active board.
    pub mode: Mode,
    /// While editing a header (name " — " prompt): the inactive half, stashed,
    /// and which half is in the edit buffer.
    pub header_other: String,
    pub editing_prompt: bool,
    /// Which inspector field is being edited, one at a time (replaces a pile of
    /// bools that could desync). `None` = not editing an inspector field.
    pub inspector: InspectorMode,
    /// Cursor into the link-candidate list while picking (InspectorMode::LinkPick).
    pub link_cursor: usize,
    /// Cursor into category suggestions while editing the category.
    pub cat_pick: usize,
    /// In-TUI file browser (for picking a local attachment). The TUI fills the
    /// entries (core stays fs-free); a picked file calls `attach_path`.
    pub browse: Option<Browse>,
    /// Search/filter overlay over the active board (yozefu-style query bar).
    pub search: Option<Search>,
    /// Registry-panel view state (BoardStyle::Panel): coarse category pick, fine
    /// filter text, selected row, and whether focus is the list (else the filter).
    pub panel_cat: usize, // 0 = all; else 1+index into panel_categories()
    pub panel_query: String,
    pub panel_sel: usize,
    pub panel_list_focus: bool, // false = typing the fine filter (default)
    pub panel_hide_cats: bool,  // collapse the category block
    /// Lane count the board last rendered with (width-dependent). The renderer
    /// writes it each frame so navigation can follow the visual columns.
    pub lanes: std::cell::Cell<usize>,
    /// First visible masonry-row / section the board last scrolled to. The renderer
    /// keeps it STABLE — only scrolling when the cursor leaves the window — so
    /// selecting an always-visible (pinned) control doesn't jump the view.
    pub board_scroll: std::cell::Cell<usize>,
    /// Help/manual overlay visible.
    pub help: bool,
    /// Quit confirmation prompt visible (q asks; y confirms).
    pub confirm_quit: bool,
    /// Inspector overlay visible (the selected element's note / links / attachments).
    pub inspect: bool,
    /// Click hit-map: the renderer records each clickable region here each frame,
    /// the mouse handler reads it. (Interior mutability so render stays read-only.)
    pub hits: std::cell::RefCell<Vec<HitRegion>>,
    /// Intents the agent is "working on" (by title). Every item that IS one of
    /// these or `serves` one pulses — so a whole intent lights up across layers.
    pub working: std::collections::HashSet<String>,
    /// Blink phase, toggled by the event loop (~every 500ms).
    pub blink: std::cell::Cell<bool>,
    /// Glossary pop-up (trigram reference + area-filter picker) visible + its cursor.
    pub glossary: bool,
    pub glossary_sel: usize,
    /// Active functional-area filter on the sheet (the trigram), if any.
    pub sheet_filter: Option<String>,
}

/// In-TUI file browser state for picking a local attachment. Cross-platform:
/// the TUI populates `entries` via `std::fs`; nothing here is OS-specific.
#[derive(Debug, Default)]
pub struct Browse {
    /// Current directory (display string).
    pub dir: String,
    /// Entries in `dir`: (name, is_dir). `..` is always first.
    pub entries: Vec<(String, bool)>,
    pub cursor: usize,
}

/// Search/filter state for the active board. Query is plain text + `field:value`
/// tokens (cat:, status:, link:, intent:, since:); see `AppState::search_results`.
#[derive(Debug, Default)]
pub struct Search {
    pub query: String,
    pub cursor: usize,
}

/// A generic UI button. Add variants as new buttons appear; each is handled in
/// one place — `AppState::press`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    AddPillar,
}

/// What a clicked region maps to.
#[derive(Debug, Clone, Copy)]
pub enum Hit {
    Cell { col: usize, row: usize }, // a grid cell (row 0 = header)
    Button(Button),                  // a button (routes through press)
}

/// A clickable region on screen (height is always 1 row).
#[derive(Debug, Clone, Copy)]
pub struct HitRegion {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub hit: Hit,
}

/// Board cursor — a grid coordinate over the table-of-tables.
/// `col` is the pillar index; `row` is the position within that pillar:
/// 0 = header, 1..=items = an item, items+1 = the "+" add slot.
/// Default starts on the first item (row 1) — the header is a deliberate detour.
#[derive(Debug, PartialEq, Clone, Copy)]
pub struct Cursor {
    pub col: usize,
    pub row: usize,
}
impl Default for Cursor {
    fn default() -> Self {
        Cursor { col: 0, row: 1 }
    }
}

type Snapshot = (String, Vec<crate::protocol::BoardColumn>);
/// Max undo history kept (bounds per-edit board-clone growth).
const UNDO_DEPTH: usize = 100;

/// What the board is currently doing.
#[derive(Debug, Default, PartialEq)]
pub enum Mode {
    #[default]
    Normal,
    /// Inline text editing — adding a new item or editing an existing one.
    Edit(Edit),
    /// Awaiting y/n to delete the selected item.
    ConfirmDelete,
}

/// Kind of interactive page currently active (for controller dispatch).
#[derive(Debug, PartialEq)]
pub enum PageKind {
    Board,
    Other,
}

/// Inline text-edit state for the selected slot.
#[derive(Debug, PartialEq)]
pub struct Edit {
    pub buffer: String,
    /// Cursor position as a char index into `buffer`.
    pub cursor: usize,
    /// Whole field highlighted: the next keystroke replaces it, an arrow
    /// collapses the cursor to that edge (classic select-all behavior).
    pub select_all: bool,
}

impl AppState {
    pub fn apply(&mut self, msg: UiMessage) -> Result<()> {
        match msg {
            UiMessage::CreateSurface { id, title, mut root } => {
                validate_tree(&root)?;
                dedupe_board(&mut root); // item text is identity → keep it unique
                self.surfaces.insert(id.clone(), Surface { title, root });
                self.touch(id);
            }
            UiMessage::UpdateNode { surface_id, node_id, patch } => {
                let surface =
                    self.surfaces.get_mut(&surface_id).ok_or_else(|| anyhow!("unknown surface '{surface_id}'"))?;
                if !patch_node(&mut surface.root, &node_id, &patch) {
                    bail!("unknown node '{node_id}' in surface '{surface_id}'");
                }
                validate_tree(&surface.root)?;
                self.touch(surface_id);
            }
            UiMessage::DeleteNode { surface_id, node_id } => {
                let surface =
                    self.surfaces.get_mut(&surface_id).ok_or_else(|| anyhow!("unknown surface '{surface_id}'"))?;
                if !delete_node(&mut surface.root, &node_id) {
                    bail!("cannot delete node '{node_id}' in surface '{surface_id}'");
                }
                self.touch(surface_id);
            }
        }
        Ok(())
    }

    pub fn latest(&self) -> Option<&Surface> {
        self.order.last().and_then(|id| self.surfaces.get(id))
    }

    /// Page titles in order — for the tab bar.
    pub fn pages(&self) -> Vec<&str> {
        self.order
            .iter()
            .filter_map(|id| self.surfaces.get(id).map(|s| s.title.as_str()))
            .collect()
    }

    /// The page currently selected for rendering.
    pub fn active_surface(&self) -> Option<&Surface> {
        self.order.get(self.ui.active).and_then(|id| self.surfaces.get(id))
    }

    /// The active page's board columns, if it's a board (for clients/renderers).
    pub fn active_board(&self) -> Option<&[crate::protocol::BoardColumn]> {
        self.active_surface().and_then(|s| find_board(&s.root))
    }

    /// The active board's render style (drives the section vs grid navigation).
    pub fn active_board_style(&self) -> BoardStyle {
        self.active_surface().and_then(|s| find_board_style(&s.root)).unwrap_or_default()
    }

    /// (col, item) cells on the active board that belong to the working set — an
    /// item that IS a working intent or `serves` one. Independent of the cursor,
    /// so a selected item that's also working stays in the set (keeps pulsing).
    /// Pure + derived; the renderer and any client share it.
    pub fn working_cells(&self) -> std::collections::HashSet<(usize, usize)> {
        let mut out = std::collections::HashSet::new();
        if self.ui.working.is_empty() {
            return out;
        }
        if let Some(cols) = self.active_board() {
            for (ci, col) in cols.iter().enumerate() {
                for (ii, it) in col.items.iter().enumerate() {
                    if self.ui.working.contains(&it.text)
                        || it.serves.iter().any(|s| self.ui.working.contains(s))
                    {
                        out.insert((ci, ii));
                    }
                }
            }
        }
        out
    }

    pub fn set_active(&mut self, i: usize) {
        if i < self.order.len() {
            self.ui.active = i;
            self.reset_view(); // sets controls-pane focus for panels
        }
    }

    /// Move the active page by `delta`, wrapping.
    pub fn cycle(&mut self, delta: isize) {
        let n = self.order.len();
        if n > 0 {
            self.ui.active = (self.ui.active as isize + delta).rem_euclid(n as isize) as usize;
            self.reset_view();
        }
    }

    fn reset_view(&mut self) {
        self.ui.cursor = Cursor::default();
        self.ui.mode = Mode::Normal;
        self.ui.header_other = String::new();
        self.ui.editing_prompt = false;
        // control.spec opens on the controls pane (not the filter); `/` enters search.
        self.ui.panel_list_focus = true;
    }

    // --- the table-of-tables grid ---
    /// Items per pillar on the active board (the inner-table sizes).
    fn board_items(&self) -> Option<Vec<usize>> {
        self.active_surface()
            .and_then(|s| find_board(&s.root))
            .map(|cols| cols.iter().map(|c| c.items.len()).collect())
    }
    fn n_cols(&self) -> usize {
        self.board_items().map(|i| i.len()).unwrap_or(0)
    }
    /// The cursor is on the "+ add pillar" button (the cell past the last pillar).
    pub fn on_add_button(&self) -> bool {
        self.board_items().is_some() && self.ui.cursor.col >= self.n_cols()
    }
    /// The button the cursor is on, if any (so Enter can press it).
    pub fn focused_button(&self) -> Option<Button> {
        self.on_add_button().then_some(Button::AddPillar)
    }
    /// The one place buttons are handled — keyboard (Enter) and mouse both call it.
    pub fn press(&mut self, button: Button) {
        match button {
            Button::AddPillar => self.add_column(),
        }
    }
    /// Clamp the cursor to a valid pillar (col, row). (Not for the add button.)
    fn cursor(&self) -> (usize, usize, usize) {
        let items = self.board_items().unwrap_or_default();
        if items.is_empty() {
            return (0, 0, 0);
        }
        let col = self.ui.cursor.col.min(items.len() - 1);
        let max_row = items[col] + 1; // header(0) .. items .. "+"(items+1)
        (col, self.ui.cursor.row.min(max_row), items[col])
    }
    fn active_header(&self, col: usize) -> Option<String> {
        self.active_surface()
            .and_then(|s| find_board(&s.root))
            .and_then(|cols| cols.get(col))
            .map(|c| c.header.clone())
    }

    /// The element under the cursor, if any (None on a header or the add button).
    pub fn selected_item(&self) -> Option<&crate::protocol::Item> {
        if self.on_add_button() {
            return None;
        }
        let (col, row, _) = self.cursor();
        if row == 0 {
            return None;
        }
        self.active_board()?.get(col)?.items.get(row - 1)
    }

    /// Titles available to link to: every item on the board page ONE LEVEL UP in
    /// the chain (intent←impl←ctrl←val). The spine (first board) has no parent.
    pub fn link_candidates(&self) -> Vec<String> {
        let board_ids: Vec<&String> =
            self.order.iter().filter(|id| self.surfaces.get(*id).and_then(|s| find_board(&s.root)).is_some()).collect();
        let active = self.order.get(self.ui.active);
        let Some(pos) = board_ids.iter().position(|id| Some(*id) == active) else { return Vec::new() };
        if pos == 0 {
            return Vec::new(); // spine: nothing higher to link to
        }
        self.surfaces
            .get(board_ids[pos - 1])
            .and_then(|s| find_board(&s.root))
            .map(|cols| cols.iter().flat_map(|c| c.items.iter().map(|i| i.text.clone())).collect())
            .unwrap_or_default()
    }
    /// Open the link picker on the selected element (no-op if it has no parent layer).
    pub fn begin_link_pick(&mut self) {
        if self.selected_item().is_some() && !self.link_candidates().is_empty() {
            self.ui.inspector = InspectorMode::LinkPick;
            self.ui.link_cursor = 0;
        }
    }
    /// Move the candidate cursor (loops).
    pub fn link_pick_move(&mut self, delta: isize) {
        let n = self.link_candidates().len();
        if n > 0 {
            self.ui.link_cursor = (self.ui.link_cursor as isize + delta).rem_euclid(n as isize) as usize;
        }
    }
    /// Toggle the highlighted candidate in/out of the selected element's `serves`.
    pub fn link_pick_toggle(&mut self) {
        let cands = self.link_candidates();
        let Some(title) = cands.get(self.ui.link_cursor).cloned() else { return };
        let (col, row, _) = self.cursor();
        if row == 0 {
            return;
        }
        if let Some(page) = self.active_page_id() {
            self.snapshot(&page);
        }
        if let Some(it) = self.active_board_mut().and_then(|c| c.get_mut(col)).and_then(|c| c.items.get_mut(row - 1)) {
            if let Some(p) = it.serves.iter().position(|s| s == &title) {
                it.serves.remove(p);
            } else {
                it.serves.push(title);
                it.serves.sort();
                it.serves.dedup();
            }
            it.touch();
            self.dirty = true;
        }
    }

    /// Inspector: start adding note lines / attachment refs (continue-add: each
    /// Enter appends a line and reopens an empty editor; empty Enter / Esc ends).
    pub fn begin_note_edit(&mut self) {
        if self.selected_item().is_some() {
            self.ui.inspector = InspectorMode::Note;
            self.ui.mode = Mode::Edit(Edit { buffer: String::new(), cursor: 0, select_all: false });
        }
    }
    pub fn begin_attach_add(&mut self) {
        if self.selected_item().is_some() {
            self.ui.inspector = InspectorMode::Attach;
            self.ui.mode = Mode::Edit(Edit { buffer: String::new(), cursor: 0, select_all: false });
        }
    }
    /// Inspector: edit the item's text (its "details"), prefilled.
    pub fn begin_detail_edit(&mut self) {
        if let Some(it) = self.selected_item() {
            let buffer = it.text.clone();
            let cursor = buffer.chars().count();
            self.ui.inspector = InspectorMode::Detail;
            self.ui.mode = Mode::Edit(Edit { buffer, cursor, select_all: true });
        }
    }
    /// Titles whose chain is fully DONE (covered + every serving item ok/justified).
    pub fn done_work_titles(&self) -> std::collections::HashSet<String> {
        crate::agent::coverage(self).into_iter().filter(|c| c.green).map(|c| c.title).collect()
    }
    /// Retire a "working" intent only if THIS edit just finished it — i.e. it's done
    /// now but wasn't in `before`. So focusing an already-done item keeps pulsing
    /// until you clear it; only a task you *complete* stops on its own.
    pub fn retire_newly_done(&mut self, before: &std::collections::HashSet<String>) {
        if self.ui.working.is_empty() {
            return;
        }
        let after = self.done_work_titles();
        self.ui.working.retain(|t| !(after.contains(t) && !before.contains(t)));
    }
    /// Inspector: cycle the selected item's status (none→ok→pending→ko→none).
    pub fn cycle_status(&mut self) {
        use crate::protocol::Status::*;
        let (col, row, _) = self.cursor();
        if row == 0 {
            return;
        }
        let before = self.done_work_titles(); // to retire only what THIS edit finishes
        if let Some(page) = self.active_page_id() {
            self.snapshot(&page);
        }
        if let Some(it) = self.active_board_mut().and_then(|c| c.get_mut(col)).and_then(|c| c.items.get_mut(row - 1)) {
            it.status = match it.status {
                None => Some(Ok),
                Some(Ok) => Some(Pending),
                Some(Pending) => Some(Fail),
                Some(Fail) => Some(Justified),
                Some(Justified) => None,
            };
            it.touch();
            self.dirty = true;
        }
        self.retire_newly_done(&before); // stop pulsing a task only when THIS edit finishes it
    }
    /// Inspector: edit the result comment (single value).
    pub fn begin_comment_edit(&mut self) {
        if let Some(it) = self.selected_item() {
            let buffer = it.comment.clone().unwrap_or_default();
            let cursor = buffer.chars().count();
            self.ui.inspector = InspectorMode::Comment;
            self.ui.mode = Mode::Edit(Edit { buffer, cursor, select_all: true });
        }
    }
    /// Inspector: toggle the selected item's `derived` flag (parentless by design).
    pub fn toggle_derived(&mut self) {
        let (col, row, _) = self.cursor();
        if row == 0 {
            return;
        }
        if let Some(page) = self.active_page_id() {
            self.snapshot(&page);
        }
        if let Some(it) = self.active_board_mut().and_then(|c| c.get_mut(col)).and_then(|c| c.items.get_mut(row - 1)) {
            it.derived = !it.derived;
            it.touch();
            self.dirty = true;
        }
    }
    /// Inspector: edit the category (a combobox — pick an existing one with ↑↓ or
    /// type a new value).
    pub fn begin_category_edit(&mut self) {
        if let Some(it) = self.selected_item() {
            let buffer = it.category.clone().unwrap_or_default();
            let cursor = buffer.chars().count();
            self.ui.inspector = InspectorMode::Category;
            self.ui.cat_pick = 0;
            self.ui.mode = Mode::Edit(Edit { buffer, cursor, select_all: true });
        }
    }
    /// Distinct category values in use on the active board (the project's live vocab).
    pub fn category_candidates(&self) -> Vec<String> {
        let Some(cols) = self.active_board() else { return Vec::new() };
        let mut v: Vec<String> = cols.iter().flat_map(|c| &c.items).filter_map(|i| i.category.clone()).collect();
        v.sort();
        v.dedup();
        v
    }
    /// ↑↓ in the category combobox: fill the field from the next/prev suggestion.
    pub fn cat_pick_move(&mut self, delta: isize) {
        let cands = self.category_candidates();
        if cands.is_empty() {
            return;
        }
        self.ui.cat_pick = (self.ui.cat_pick as isize + delta).rem_euclid(cands.len() as isize) as usize;
        let pick = cands[self.ui.cat_pick].clone();
        if let Mode::Edit(e) = &mut self.ui.mode {
            e.cursor = pick.chars().count();
            e.buffer = pick;
            e.select_all = false;
        }
    }
    /// Append an attachment ref (e.g. a path chosen via a file browser) to the
    /// selected element. The driver supplies the string; state just stores it.
    pub fn attach_path(&mut self, path: String) {
        if path.is_empty() {
            return;
        }
        let (col, row, _) = self.cursor();
        if row == 0 {
            return;
        }
        if let Some(page) = self.active_page_id() {
            self.snapshot(&page);
        }
        if let Some(it) = self.active_board_mut().and_then(|c| c.get_mut(col)).and_then(|c| c.items.get_mut(row.saturating_sub(1))) {
            it.attachments.push(path);
            it.touch();
            self.dirty = true;
        }
    }
    /// Commit one inspector line (note or attachment). Returns true if handled.
    fn commit_inspector_edit(&mut self, label: &str, continue_add: bool) -> bool {
        // details = the item's text (identity); rename cascades serves links
        if self.ui.inspector == InspectorMode::Detail {
            self.ui.inspector = InspectorMode::None;
            self.ui.mode = Mode::Normal;
            if label.is_empty() {
                return true; // don't blank an identity
            }
            let (col, row, _) = self.cursor();
            let old = self.selected_item().map(|i| i.text.clone()).unwrap_or_default();
            if let Some(page) = self.active_page_id() {
                self.snapshot(&page);
            }
            if let Some(it) = self.active_board_mut().and_then(|c| c.get_mut(col)).and_then(|c| c.items.get_mut(row.saturating_sub(1))) {
                it.text = label.to_string();
                it.touch();
                self.dirty = true;
            }
            if old != label {
                self.rename_intent(&old, label);
            }
            return true;
        }
        // category is a single value (set + done), not a continue-add list
        if self.ui.inspector == InspectorMode::Category {
            let (col, row, _) = self.cursor();
            if let Some(page) = self.active_page_id() {
                self.snapshot(&page);
            }
            if let Some(it) = self.active_board_mut().and_then(|c| c.get_mut(col)).and_then(|c| c.items.get_mut(row.saturating_sub(1))) {
                it.category = (!label.is_empty()).then(|| label.to_string());
                it.touch();
                self.dirty = true;
            }
            self.ui.inspector = InspectorMode::None;
            self.ui.mode = Mode::Normal;
            return true;
        }
        // comment is a single value (result remark)
        if self.ui.inspector == InspectorMode::Comment {
            let (col, row, _) = self.cursor();
            if let Some(page) = self.active_page_id() {
                self.snapshot(&page);
            }
            if let Some(it) = self.active_board_mut().and_then(|c| c.get_mut(col)).and_then(|c| c.items.get_mut(row.saturating_sub(1))) {
                it.comment = (!label.is_empty()).then(|| label.to_string());
                it.touch();
                self.dirty = true;
            }
            self.ui.inspector = InspectorMode::None;
            self.ui.mode = Mode::Normal;
            return true;
        }
        if !matches!(self.ui.inspector, InspectorMode::Note | InspectorMode::Attach) {
            return false;
        }
        // commit the current line (if any), then: Shift+Enter (continue_add) opens a
        // fresh line; plain Enter accepts and closes the editor.
        let note = self.ui.inspector == InspectorMode::Note;
        if !label.is_empty() {
            let (col, row, _) = self.cursor();
            if let Some(page) = self.active_page_id() {
                self.snapshot(&page);
            }
            if let Some(it) = self.active_board_mut().and_then(|c| c.get_mut(col)).and_then(|c| c.items.get_mut(row.saturating_sub(1))) {
                if note {
                    it.note.push(label.to_string());
                } else {
                    it.attachments.push(label.to_string());
                }
                it.touch();
                self.dirty = true;
            }
        }
        if continue_add {
            self.ui.mode = Mode::Edit(Edit { buffer: String::new(), cursor: 0, select_all: false });
        } else {
            self.ui.inspector = InspectorMode::None;
            self.ui.mode = Mode::Normal;
        }
        true
    }

    /// Active-board items matching the current search query: (col, item) coords.
    /// Query = plain terms (match name/category/notes) + AND'd `field:value` tokens:
    /// `cat:` `status:` `id:` `link:` `intent:` (transitive) `since:` (e.g. 7d).
    pub fn search_results(&self) -> Vec<(usize, usize)> {
        let query = self.ui.search.as_ref().map(|s| s.query.clone()).unwrap_or_default();
        let Some(cols) = self.active_board() else { return Vec::new() };
        let idx = self.chain_index(&query);
        let mut out = Vec::new();
        for (ci, col) in cols.iter().enumerate() {
            for (ii, it) in col.items.iter().enumerate() {
                if self.item_matches(it, col.key.as_deref().unwrap_or(""), &query, &idx) {
                    out.push((ci, ii));
                }
            }
        }
        out
    }

    /// title → its `serves` links, across all boards (first occurrence wins, like
    /// the old per-name lookup). Built once per search so `intent:` chain-walking
    /// is O(items) instead of a full-tree scan per hop. Empty unless the query
    /// actually uses `intent:` (the only consumer).
    fn chain_index(&self, query: &str) -> std::collections::HashMap<String, Vec<String>> {
        let mut m = std::collections::HashMap::new();
        if !query.contains("intent:") {
            return m;
        }
        for id in &self.order {
            if let Some(cols) = self.surfaces.get(id).and_then(|s| find_board(&s.root)) {
                for it in cols.iter().flat_map(|c| &c.items) {
                    m.entry(it.text.clone()).or_insert_with(|| it.serves.clone());
                }
            }
        }
        m
    }

    fn item_matches(&self, it: &crate::protocol::Item, extra: &str, query: &str, idx: &std::collections::HashMap<String, Vec<String>>) -> bool {
        use crate::protocol::Status::*;
        // `extra` = column-level text the item belongs to (its subset) — searchable too
        let hay = format!("{} {} {} {extra}", it.text, it.category.clone().unwrap_or_default(), it.note.join(" "))
            .to_lowercase();
        let sw = match it.status {
            Some(Ok) => "ok",
            Some(Fail) => "ko",
            Some(Pending) => "pending",
            Some(Justified) => "justified",
            None => "",
        };
        for tok in query.split_whitespace() {
            let lt = tok.to_lowercase();
            let ok = if let Some(v) = lt.strip_prefix("cat:") {
                it.category.as_deref().map(|c| c.to_lowercase().contains(v)).unwrap_or(false)
            } else if let Some(v) = lt.strip_prefix("status:") {
                if v == "na" || v == "none" { it.status.is_none() } else { sw == v }
            } else if let Some(v) = lt.strip_prefix("id:") {
                it.id.as_deref().map(|i| i.to_lowercase().contains(v)).unwrap_or(false)
            } else if let Some(v) = lt.strip_prefix("link:") {
                it.serves.iter().any(|s| s.to_lowercase().contains(v))
            } else if let Some(v) = lt.strip_prefix("intent:") {
                reaches_intent(it, v, idx)
            } else if let Some(v) = lt.strip_prefix("since:") {
                it.modified.map(|t| now_secs().saturating_sub(t) <= parse_age(v)).unwrap_or(false)
            } else {
                hay.contains(&lt)
            };
            if !ok {
                return false; // AND across tokens
            }
        }
        true
    }

    // --- registry panel (BoardStyle::Panel) -------------------------------
    /// Sidebar groups: the components controls link to (their `serves` targets) +
    /// counts. Bounded by the impl layer — the structural navigation axis.
    pub fn panel_components(&self) -> Vec<(String, usize)> {
        let Some(cols) = self.active_board() else { return Vec::new() };
        let mut map: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
        for col in cols {
            for it in &col.items {
                for s in &it.serves {
                    *map.entry(s.clone()).or_default() += 1;
                }
            }
        }
        map.into_iter().collect()
    }
    /// Controls passing the coarse (component) + fine (filter text) filters: (col,item).
    pub fn panel_results(&self) -> Vec<(usize, usize)> {
        let Some(cols) = self.active_board() else { return Vec::new() };
        let comps = self.panel_components();
        let sel = if self.ui.panel_cat == 0 { None } else { comps.get(self.ui.panel_cat - 1).map(|(c, _)| c.clone()) };
        let q = self.ui.panel_query.clone();
        let idx = self.chain_index(&q);
        let mut out = Vec::new();
        for (ci, col) in cols.iter().enumerate() {
            for (ii, it) in col.items.iter().enumerate() {
                let comp_ok = sel.as_deref().map(|c| it.serves.iter().any(|s| s == c)).unwrap_or(true);
                if comp_ok && self.item_matches(it, col.key.as_deref().unwrap_or(""), &q, &idx) {
                    out.push((ci, ii));
                }
            }
        }
        out
    }
    pub fn panel_cat_move(&mut self, delta: isize) {
        let n = self.panel_components().len() + 1; // +1 for "all"
        self.ui.panel_cat = (self.ui.panel_cat as isize + delta).rem_euclid(n as isize) as usize;
        self.ui.panel_sel = 0;
    }
    pub fn panel_move(&mut self, delta: isize) {
        let n = self.panel_results().len() + 1; // +1 for the trailing "＋ add control" row
        if n > 0 {
            self.ui.panel_sel = (self.ui.panel_sel as isize + delta).rem_euclid(n as isize) as usize;
        }
        self.sync_panel_cursor();
    }
    /// Keep the board cursor pointing at the panel-selected control, so o/inspect
    /// and quick-edit act on the right item.
    fn sync_panel_cursor(&mut self) {
        if let Some(&(c, i)) = self.panel_results().get(self.ui.panel_sel) {
            self.ui.cursor = Cursor { col: c, row: i + 1 };
        }
    }
    /// Enter on a control: quick-edit its detail inline. Enter on the trailing
    /// "＋ add control" row adds a new control (and quick-edits it).
    pub fn panel_quick_edit(&mut self) {
        if self.ui.panel_sel >= self.panel_results().len() {
            self.panel_add();
            return;
        }
        self.sync_panel_cursor();
        self.begin_detail_edit();
    }
    /// Tab during quick-edit: commit the current field and advance — detail → the
    /// method combobox → the next control's detail (spreadsheet-style).
    pub fn panel_quick_tab(&mut self) {
        let buf = match &self.ui.mode {
            Mode::Edit(e) => e.buffer.trim().to_string(),
            _ => return,
        };
        match self.ui.inspector {
            InspectorMode::Detail => {
                self.apply_quick_detail(&buf);
                self.begin_category_edit(); // → the control/method combobox
            }
            InspectorMode::Category => {
                self.apply_quick_category(&buf);
                self.panel_move(1); // next control (syncs cursor)
                if self.panel_results().get(self.ui.panel_sel).is_some() {
                    self.begin_detail_edit();
                } else {
                    self.cancel(); // ran off the end (the add row) → leave edit mode
                }
            }
            _ => {}
        }
    }
    fn apply_quick_detail(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let (col, row, _) = self.cursor();
        if let Some(page) = self.active_page_id() {
            self.snapshot(&page);
        }
        let mut renamed = None;
        if let Some(it) = self.active_board_mut().and_then(|c| c.get_mut(col)).and_then(|c| c.items.get_mut(row.saturating_sub(1))) {
            let old = std::mem::replace(&mut it.text, text.to_string());
            it.touch();
            self.dirty = true;
            if old != text {
                renamed = Some((old, text.to_string()));
            }
        }
        if let Some((o, n)) = renamed {
            self.rename_intent(&o, &n);
        }
    }
    fn apply_quick_category(&mut self, cat: &str) {
        let (col, row, _) = self.cursor();
        if let Some(page) = self.active_page_id() {
            self.snapshot(&page);
        }
        if let Some(it) = self.active_board_mut().and_then(|c| c.get_mut(col)).and_then(|c| c.items.get_mut(row.saturating_sub(1))) {
            it.category = (!cat.is_empty()).then(|| cat.to_string());
            it.touch();
            self.dirty = true;
        }
    }
    pub fn panel_char(&mut self, ch: char) {
        self.ui.panel_query.push(ch);
        self.ui.panel_sel = 0;
    }
    pub fn panel_backspace(&mut self) {
        self.ui.panel_query.pop();
        self.ui.panel_sel = 0;
    }
    /// Enter on a control opens its inspector; Enter on the trailing add row (the
    /// "＋ add control" button) adds a new control instead.
    pub fn panel_open(&mut self) {
        let res = self.panel_results();
        if self.ui.panel_sel >= res.len() {
            self.panel_add();
            return;
        }
        if let Some(&(col, item)) = res.get(self.ui.panel_sel) {
            self.ui.cursor = Cursor { col, row: item + 1 };
            self.ui.inspect = true;
        }
    }
    /// Add a new control to the panel's first column, select it, and open the
    /// inspector editing its detail — so a person can author a control from the UI.
    pub fn panel_add(&mut self) {
        if let Some(page) = self.active_page_id() {
            self.snapshot(&page);
        }
        let idx = match self.active_board_mut().and_then(|c| c.first_mut()) {
            Some(c) => {
                c.items.push(crate::protocol::Item::new("new control"));
                c.items.len() - 1
            }
            None => return,
        };
        self.dirty = true;
        self.ui.panel_cat = 0; // clear the component filter so the new row is visible
        self.ui.cursor = Cursor { col: 0, row: idx + 1 };
        if let Some(p) = self.panel_results().iter().position(|&(c, i)| c == 0 && i == idx) {
            self.ui.panel_sel = p;
        }
        self.begin_detail_edit(); // quick-edit the detail inline (no inspector overlay)
    }
    /// Delete the selected control (with confirmation). No-op on the add row.
    pub fn panel_delete(&mut self) {
        let res = self.panel_results();
        if let Some(&(col, item)) = res.get(self.ui.panel_sel) {
            self.ui.cursor = Cursor { col, row: item + 1 };
            self.begin_delete();
        }
    }

    // --- search overlay control -------------------------------------------
    pub fn open_search(&mut self) {
        self.ui.search = Some(Search::default());
    }
    pub fn search_char(&mut self, ch: char) {
        if let Some(s) = self.ui.search.as_mut() {
            s.query.push(ch);
            s.cursor = 0; // results shift as the query changes
        }
    }
    pub fn search_backspace(&mut self) {
        if let Some(s) = self.ui.search.as_mut() {
            s.query.pop();
            s.cursor = 0;
        }
    }
    pub fn search_move(&mut self, delta: isize) {
        let n = self.search_results().len();
        if let (Some(s), true) = (self.ui.search.as_mut(), n > 0) {
            s.cursor = (s.cursor as isize + delta).rem_euclid(n as isize) as usize;
        }
    }
    /// Jump the cursor to the selected result and close the overlay.
    pub fn search_jump(&mut self) {
        let res = self.search_results();
        let idx = self.ui.search.as_ref().map(|s| s.cursor).unwrap_or(0);
        if let Some(&(col, item)) = res.get(idx) {
            self.ui.cursor = Cursor { col, row: item + 1 };
        }
        self.ui.search = None;
    }

    /// True when the cursor is on a header (row 0). Never the add button.
    pub fn on_header(&self) -> bool {
        !self.on_add_button() && self.cursor().1 == 0
    }
    /// Flat slot index for the renderer (None on a header or the add button).
    pub fn selected_flat(&self) -> Option<usize> {
        if self.on_add_button() {
            return None;
        }
        let (col, row, _) = self.cursor();
        if row == 0 {
            return None;
        }
        let base: usize = self
            .board_items()
            .unwrap_or_default()
            .iter()
            .take(col)
            .map(|n| n + 1)
            .sum();
        Some(base + (row - 1))
    }
    /// Column whose header the cursor is on (for the renderer / delete prompt).
    pub fn header_col(&self) -> Option<usize> {
        if self.on_add_button() {
            return None;
        }
        let (col, row, _) = self.cursor();
        (row == 0).then_some(col)
    }

    /// Leave a header, down to the first item (used by Esc / j).
    pub fn focus_items(&mut self) {
        if self.ui.cursor.row == 0 {
            self.ui.cursor.row = 1;
        }
    }

    /// Add a new column (pillar) and put the cursor on its header to rename.
    pub fn add_column(&mut self) {
        if !matches!(self.active_page_kind(), PageKind::Board) {
            return;
        }
        if let Some(page) = self.active_page_id() {
            self.snapshot(&page);
        }
        let idx = self.board_items().map(|i| i.len()).unwrap_or(0);
        if let Some(cols) = self.active_board_mut() {
            cols.push(crate::protocol::BoardColumn {
                header: "NEW — rename".into(),
                key: None,
                items: vec![],
            });
            self.dirty = true;
        }
        self.ui.cursor = Cursor { col: idx, row: 0 };
    }

    /// Cascade an item rename to every `serves` link that named the old text,
    /// across all pages — so links survive editing the thing they point at.
    pub fn rename_intent(&mut self, old: &str, new: &str) {
        if old == new {
            return;
        }
        for s in self.surfaces.values_mut() {
            for_each_item_mut(&mut s.root, &mut |it| {
                let mut hit = false;
                for sv in it.serves.iter_mut() {
                    if sv == old {
                        *sv = new.to_string();
                        hit = true;
                    }
                }
                if hit {
                    it.serves.sort();
                    it.serves.dedup();
                }
            });
        }
    }

    /// Reorder the item under the cursor within its column (cursor follows).
    pub fn move_item(&mut self, delta: isize) {
        if self.on_add_button() {
            return;
        }
        let (col, row, items) = self.cursor();
        if row == 0 || row > items {
            return; // header or "+"
        }
        let i = row - 1;
        let target = i as isize + delta;
        if target < 0 || target >= items as isize {
            return;
        }
        let target = target as usize;
        if let Some(page) = self.active_page_id() {
            self.snapshot(&page);
        }
        if let Some(cols) = self.active_board_mut() {
            cols[col].items.swap(i, target);
            self.dirty = true;
        }
        self.ui.cursor = Cursor { col, row: target + 1 };
    }

    /// Which kind of interactive page is active — drives controller dispatch.
    pub fn active_page_kind(&self) -> PageKind {
        let Some(s) = self.active_surface() else { return PageKind::Other };
        if find_board(&s.root).is_some() {
            PageKind::Board
        } else {
            PageKind::Other
        }
    }

    fn lanes(&self) -> usize {
        self.ui.lanes.get().max(1)
    }

    fn active_page_id(&self) -> Option<String> {
        self.order.get(self.ui.active).cloned()
    }
    fn board_cols_clone(&self, page: &str) -> Option<Vec<crate::protocol::BoardColumn>> {
        self.surfaces.get(page).and_then(|s| find_board(&s.root)).map(|c| c.to_vec())
    }
    fn set_board_cols(&mut self, page: &str, cols: Vec<crate::protocol::BoardColumn>) {
        if let Some(s) = self.surfaces.get_mut(page) {
            if let Some(c) = find_board_mut(&mut s.root) {
                *c = cols;
            }
        }
    }

    /// Snapshot a page's board before a data edit (clears the redo stack).
    pub fn snapshot(&mut self, page: &str) {
        if let Some(cols) = self.board_cols_clone(page) {
            self.undo.push_back((page.to_string(), cols));
            if self.undo.len() > UNDO_DEPTH {
                self.undo.pop_front(); // bound memory; drop the oldest (O(1))
            }
            self.redo.clear();
        }
    }

    /// Undo the last board edit. Returns false if nothing to undo.
    pub fn undo(&mut self) -> bool {
        let Some((page, prev)) = self.undo.pop_back() else { return false };
        if let Some(cur) = self.board_cols_clone(&page) {
            self.redo.push_back((page.clone(), cur));
        }
        self.set_board_cols(&page, prev);
        self.dirty = true;
        true
    }

    /// Redo the last undone edit. Returns false if nothing to redo.
    pub fn redo(&mut self) -> bool {
        let Some((page, next)) = self.redo.pop_back() else { return false };
        if let Some(cur) = self.board_cols_clone(&page) {
            self.undo.push_back((page.clone(), cur));
        }
        self.set_board_cols(&page, next);
        self.dirty = true;
        true
    }

    /// Put the cursor on a board item by (column, item).
    pub fn select_board_item(&mut self, col: usize, item: usize) {
        self.ui.cursor = Cursor { col, row: item + 1 };
    }

    // --- Board controls: grid motion over the table-of-tables ---
    /// Up/Down over items + "+" + the add-pillar button (the last cell). Headers
    /// are skipped (reach them with the header key). Loops: past the bottom of a
    /// lane → top of the next lane, wrapping last→first.
    pub fn board_select_vert(&mut self, delta: isize) {
        // Filtered sheet: 1-D nav over the matching rows of the first column (plus
        // the add slot), so the cursor only ever lands on a visible case.
        if self.active_board_style() == BoardStyle::Sheet {
            if let Some(area) = self.ui.sheet_filter.clone() {
                if let Some(col) = self.active_board().and_then(|c| c.first()) {
                    let mut rows: Vec<usize> = col
                        .items
                        .iter()
                        .enumerate()
                        .filter(|(_, it)| it.id.as_deref().and_then(crate::agent::area_of) == Some(area.as_str()))
                        .map(|(i, _)| i + 1)
                        .collect();
                    rows.push(col.items.len() + 1); // the add-case slot is always reachable
                    let cur = self.ui.cursor.row;
                    let pos = rows.iter().position(|&r| r >= cur).unwrap_or(0);
                    let npos = (pos as isize + delta).clamp(0, rows.len() as isize - 1) as usize;
                    self.ui.cursor = Cursor { col: 0, row: rows[npos] };
                }
                return;
            }
        }
        let Some(items) = self.board_items() else { return };
        // Cards = stacked sections whose boxes wrap into rows of `per_row`. Vertical
        // moves one VISUAL row within a section; past the section's top/bottom it
        // crosses to the neighbour (or the add-card button), keeping the column.
        // Invariant: walks sections top→bottom in order, looping at the ends — see
        // tests `cards_header_vert_walks_sections_in_order` / the cards-nav tests.
        // NB: `col == n` is the add-card button (NOT a section), so items[col] is
        // only indexed after the `col == n` guard returns.
        if self.active_board_style() == BoardStyle::Cards {
            let n = items.len();
            let pr = self.lanes().max(1); // boxes per row (from the renderer)
            let col = self.ui.cursor.col.min(n); // n == add-card button
            if col == n {
                // button (very bottom): up → last section's last box; down loops to top
                let cur = if delta < 0 && n > 0 {
                    (n - 1, items[n - 1])
                } else if delta > 0 && n > 0 {
                    (0, 0) // loop back to the first box
                } else {
                    (n, 0)
                };
                self.ui.cursor = Cursor { col: cur.0, row: if cur.0 == n { 0 } else { cur.1 + 1 } };
                return;
            }
            let boxes = items[col] + 1; // components + the section's "+"
            let bi = self.ui.cursor.row.max(1) - 1; // box index 0..=items
            let vc = bi % pr; // visual column
            let cur = if delta > 0 {
                if bi + pr < boxes {
                    (col, bi + pr) // down a row in this section
                } else if col + 1 < n {
                    (col + 1, vc.min(items[col + 1])) // top row of next section, same column
                } else {
                    (n, 0) // fell off the last section → add-card button
                }
            } else if bi >= pr {
                (col, bi - pr) // up a row
            } else if col >= 1 {
                let nb = items[col - 1] + 1; // prev section box count
                let last_row_start = (nb - 1) / pr * pr;
                ((col - 1), (last_row_start + vc).min(nb - 1)) // bottom row of prev section
            } else {
                (n, 0) // off the top of the first section → loop to the add-card button
            };
            self.ui.cursor = Cursor { col: cur.0, row: if cur.0 == n { 0 } else { cur.1 + 1 } };
            return;
        }
        let (k, n) = (self.lanes(), items.len());
        let cells = n + 1; // pillars + the add button at index n
        let col = self.ui.cursor.col.min(cells - 1);
        let is_btn = col == n;
        let mi = if is_btn { 0 } else { items[col] + 1 }; // bottom row of this cell
        let row = self.ui.cursor.row.min(mi);
        let lane = col % k;
        // first/last navigable row of a cell (pillar: items 1..="+"; button: just 0)
        let top = |c: usize| if c < n { 1 } else { 0 };
        let bot = |c: usize| if c < n { items[c] + 1 } else { 0 };

        // header row on a pillar = header layer (button row 0 is the button itself)
        if !is_btn && row == 0 {
            let (mut nc, mut nr) = (col, 0);
            if delta > 0 {
                nr = 1;
            } else if col >= k {
                nc = col - k;
            }
            self.ui.cursor = Cursor { col: nc, row: nr };
            return;
        }
        let (nc, nr) = if delta > 0 {
            if !is_btn && row < mi {
                (col, row + 1)
            } else if col + k < cells {
                (col + k, top(col + k)) // next cell down in lane
            } else {
                let c = (lane + 1) % k; // loop to top of next lane
                (c, top(c))
            }
        } else if !is_btn && row > 1 {
            (col, row - 1)
        } else if col >= k {
            (col - k, bot(col - k)) // cell above in lane
        } else {
            let p = last_in_lane((lane + k - 1) % k, cells, k); // loop to bottom of prev lane
            (p, bot(p))
        };
        self.ui.cursor = Cursor { col: nc, row: nr };
    }

    /// Left/Right: adjacent cell in the grid row (wraps), incl. the add button.
    pub fn board_select_horiz(&mut self, delta: isize) {
        // Cards = sections: horizontal moves between boxes, wrapping within the
        // current VISUAL ROW (the `per_row` chunk), not the whole section.
        if self.active_board_style() == BoardStyle::Cards {
            let Some(items) = self.board_items() else { return };
            let col = self.ui.cursor.col;
            if col >= items.len() {
                return; // on the add button — no boxes to move through
            }
            let pr = self.lanes().max(1);
            let boxes = items[col] + 1; // components + the "+"
            let bi = self.ui.cursor.row.max(1) - 1; // box index 0..=items
            let row_start = bi / pr * pr;
            let row_end = (row_start + pr - 1).min(boxes - 1);
            let nb = if delta > 0 {
                if bi >= row_end { row_start } else { bi + 1 }
            } else if bi <= row_start {
                row_end
            } else {
                bi - 1
            };
            self.ui.cursor = Cursor { col, row: nb + 1 };
            return;
        }
        let Some(items) = self.board_items() else { return };
        let (k, n) = (self.lanes(), items.len());
        let cells = n + 1; // pillars + the add button
        let col = self.ui.cursor.col.min(cells - 1);
        let from_btn = col == n;
        let (gc, gr) = (col % k, col / k);
        let nc = if delta < 0 {
            if gc == 0 { (gr * k + k - 1).min(cells - 1) } else { col - 1 }
        } else if gc == k - 1 || col + 1 >= cells {
            gr * k
        } else {
            col + 1
        };
        // Button has only row 0; leaving the button lands on a first item, not a
        // header; otherwise keep the header row, else clamp within items/"+".
        let nr = if nc == n {
            0
        } else if from_btn || self.ui.cursor.row > 0 {
            self.ui.cursor.row.clamp(1, items[nc] + 1)
        } else {
            0
        };
        self.ui.cursor = Cursor { col: nc, row: nr };
    }

    /// Jump to the current column's header (the deliberate detour).
    pub fn focus_header(&mut self) {
        if self.board_items().is_some_and(|i| !i.is_empty()) {
            self.ui.cursor.row = 0;
        }
    }

    /// Move across headers horizontally (Shift+H/L, [ ]): wraps over all columns.
    pub fn header_move(&mut self, delta: isize) {
        let Some(items) = self.board_items() else { return };
        let n = items.len();
        if n == 0 {
            return;
        }
        let col = self.cursor().0;
        let nc = (col as isize + delta).rem_euclid(n as isize) as usize;
        self.ui.cursor = Cursor { col: nc, row: 0 };
    }

    /// Vertical header jump (Shift+K/J, Ctrl+↑/↓): always lands on a header,
    /// from items too. Up → this pillar's header, then the header above in the
    /// lane (col-k); down → the next header below (col+k). Rolls over lanes.
    pub fn header_vert(&mut self, delta: isize) {
        let n = self.n_cols();
        if n == 0 {
            return;
        }
        // Cards stack one section per row, so headers walk by ±1 in section order
        // (lanes() is boxes-per-row here, not a header step). Button is the last cell.
        if self.active_board_style() == BoardStyle::Cards {
            let cells = n + 1; // section headers + the add-card button
            let col = self.ui.cursor.col.min(cells - 1);
            let on_item = !self.on_header() && !self.on_add_button();
            let nc = if delta < 0 {
                if on_item { col } else { (col + cells - 1) % cells }
            } else if on_item {
                (col + 1).min(cells - 1)
            } else {
                (col + 1) % cells
            };
            self.ui.cursor = Cursor { col: nc, row: 0 };
            return;
        }
        let k = self.lanes();
        let cells = n + 1; // headers + the add button (last cell)
        let col = self.ui.cursor.col.min(cells - 1);
        let lane = col % k;
        let on_item = !self.on_header() && !self.on_add_button();
        self.ui.cursor.col = if delta < 0 {
            if on_item {
                col // from an item → this pillar's own header
            } else if col >= k {
                col - k
            } else {
                last_in_lane((lane + k - 1) % k, cells, k) // roll to prev lane bottom
            }
        } else if col + k < cells {
            col + k
        } else {
            (lane + 1) % k // roll to next lane top
        };
        self.ui.cursor.row = 0;
    }


    fn item_text(&self, col: usize, i: usize) -> Option<String> {
        let cols = self.active_surface().and_then(|s| find_board(&s.root))?;
        cols.get(col).and_then(|c| c.items.get(i)).map(|it| it.text.clone())
    }

    /// Enter: header (row 0) → edit it; "+" → add a blank item; item → edit it.
    pub fn activate(&mut self) {
        if let Some(button) = self.focused_button() {
            self.press(button); // Enter on a button
            return;
        }
        let (col, row, items) = self.cursor();
        if row == 0 {
            // header: split name " — " prompt, edit the name first
            if let Some(page) = self.active_page_id() {
                self.snapshot(&page);
            }
            let (name, prompt) = split_header(&self.active_header(col).unwrap_or_default());
            self.ui.header_other = prompt;
            self.ui.editing_prompt = false;
            let cursor = name.chars().count();
            self.ui.mode = Mode::Edit(Edit { buffer: name, cursor, select_all: true });
            return;
        }
        if row == items + 1 {
            self.ui.mode = Mode::Edit(Edit { buffer: String::new(), cursor: 0, select_all: false });
        } else if let Some(text) = self.item_text(col, row - 1) {
            let cursor = text.chars().count();
            self.ui.mode = Mode::Edit(Edit { buffer: text, cursor, select_all: true });
        }
    }

    /// Tab / Ctrl+←/→ while editing a header: swap between name and prompt halves.
    pub fn switch_header_part(&mut self) {
        if !self.on_header() {
            return;
        }
        if let Mode::Edit(e) = &mut self.ui.mode {
            std::mem::swap(&mut e.buffer, &mut self.ui.header_other);
            e.cursor = e.buffer.chars().count();
            e.select_all = true;
            self.ui.editing_prompt = !self.ui.editing_prompt;
        }
    }

    /// Backspace/Delete → confirm removing the header's column (row 0) or the item.
    pub fn begin_delete(&mut self) {
        if self.on_add_button() {
            return; // nothing to delete on the button
        }
        let (_, row, items) = self.cursor();
        if row == 0 || (row >= 1 && row <= items) {
            self.ui.mode = Mode::ConfirmDelete; // header→column, or an item
        }
    }

    /// A typed character: replaces the field if it was fully selected, else inserts.
    pub fn edit_char(&mut self, ch: char) {
        if let Mode::Edit(e) = &mut self.ui.mode {
            if e.select_all {
                e.buffer.clear();
                e.cursor = 0;
                e.select_all = false;
            }
            let at = byte_idx(&e.buffer, e.cursor);
            e.buffer.insert(at, ch);
            e.cursor += 1;
        }
    }
    pub fn edit_backspace(&mut self) {
        if let Mode::Edit(e) = &mut self.ui.mode {
            if e.select_all {
                e.buffer.clear();
                e.cursor = 0;
                e.select_all = false;
            } else if e.cursor > 0 {
                let s = byte_idx(&e.buffer, e.cursor - 1);
                let t = byte_idx(&e.buffer, e.cursor);
                e.buffer.replace_range(s..t, "");
                e.cursor -= 1;
            }
        }
    }
    pub fn edit_delete_forward(&mut self) {
        if let Mode::Edit(e) = &mut self.ui.mode {
            let len = e.buffer.chars().count();
            if e.select_all {
                e.buffer.clear();
                e.cursor = 0;
                e.select_all = false;
            } else if e.cursor < len {
                let s = byte_idx(&e.buffer, e.cursor);
                let t = byte_idx(&e.buffer, e.cursor + 1);
                e.buffer.replace_range(s..t, "");
            }
        }
    }
    /// Arrow: if fully selected, collapse the cursor to that edge; else move it.
    pub fn edit_left(&mut self) {
        if let Mode::Edit(e) = &mut self.ui.mode {
            if e.select_all {
                e.cursor = 0;
                e.select_all = false;
            } else {
                e.cursor = e.cursor.saturating_sub(1);
            }
        }
    }
    pub fn edit_right(&mut self) {
        if let Mode::Edit(e) = &mut self.ui.mode {
            let len = e.buffer.chars().count();
            if e.select_all {
                e.cursor = len;
                e.select_all = false;
            } else {
                e.cursor = (e.cursor + 1).min(len);
            }
        }
    }
    pub fn cancel(&mut self) {
        self.ui.mode = Mode::Normal;
        self.ui.inspector = InspectorMode::None;
        self.ui.glossary = false;
    }

    // --- glossary pop-up (trigram reference + area filter) ------------------
    pub fn open_glossary(&mut self) {
        self.ui.glossary = true;
        self.ui.glossary_sel = 0;
    }
    /// Move the glossary cursor (wraps). Row 0 is the "all / clear filter" entry.
    pub fn glossary_move(&mut self, delta: isize) {
        let len = crate::agent::glossary(self).len() + 1; // +1 for the "all" row
        self.ui.glossary_sel = (self.ui.glossary_sel as isize + delta).rem_euclid(len as isize) as usize;
    }
    /// Apply the highlighted entry: row 0 clears the filter, else filter to that
    /// area; then close the pop-up.
    pub fn glossary_apply(&mut self) {
        let sel = self.ui.glossary_sel;
        let code = if sel == 0 { None } else { crate::agent::glossary(self).get(sel - 1).map(|(c, _)| c.clone()) };
        self.set_sheet_filter(code);
        self.ui.glossary = false;
    }
    /// Set (or clear) the sheet's functional-area filter; snap the cursor to the
    /// first matching case so selection stays on a visible row.
    pub fn set_sheet_filter(&mut self, area: Option<String>) {
        self.ui.sheet_filter = area.clone();
        if let Some(a) = area {
            if let Some(cols) = self.active_board() {
                if let Some(col) = cols.first() {
                    if let Some((i, _)) = col
                        .items
                        .iter()
                        .enumerate()
                        .find(|(_, it)| it.id.as_deref().and_then(crate::agent::area_of) == Some(a.as_str()))
                    {
                        self.ui.cursor = Cursor { col: 0, row: i + 1 };
                    }
                }
            }
        }
    }

    /// Commit the edit: replace the existing item, or append a new one at "+".
    pub fn input_commit(&mut self, continue_add: bool) {
        let Mode::Edit(e) = &self.ui.mode else { return };
        let label = e.buffer.trim().to_string();

        if self.commit_inspector_edit(&label, continue_add) {
            return; // inspector note/attachment edit
        }

        let (col, row, items) = self.cursor();
        // Header edit (row 0): recombine name " — " prompt and write the header.
        if row == 0 {
            let (name, prompt) = if self.ui.editing_prompt {
                (self.ui.header_other.trim().to_string(), label)
            } else {
                (label, self.ui.header_other.trim().to_string())
            };
            let header = if prompt.is_empty() { name } else { format!("{name} — {prompt}") };
            self.ui.mode = Mode::Normal;
            if !header.trim().is_empty() {
                if let Some(cols) = self.active_board_mut() {
                    if let Some(c) = cols.get_mut(col) {
                        c.header = header;
                        self.dirty = true;
                    }
                }
            }
            return;
        }

        self.ui.mode = Mode::Normal;
        if label.is_empty() {
            return;
        }
        if let Some(page) = self.active_page_id() {
            self.snapshot(&page);
        }
        let mut renamed: Option<(String, String)> = None;
        if let Some(cols) = self.active_board_mut() {
            if let Some(c) = cols.get_mut(col) {
                if row == items + 1 {
                    c.items.push(crate::protocol::Item::new(label)); // append at "+"
                    dedupe_items(c);
                    if let Some(it) = c.items.last_mut() {
                        it.touch();
                    }
                } else if let Some(it) = c.items.get_mut(row - 1) {
                    let old = std::mem::replace(&mut it.text, label.clone()); // serves rides along
                    it.touch();
                    if old != label {
                        renamed = Some((old, label));
                    }
                }
                self.dirty = true;
            }
        }
        // Renaming an item cascades to any `serves` link that named its old text.
        if let Some((old, new)) = renamed {
            self.rename_intent(&old, &new);
        }
    }

    /// Confirmed delete: header (row 0) → remove the column; else remove the item.
    pub fn confirm_delete(&mut self) {
        self.ui.mode = Mode::Normal;
        let (col, row, items) = self.cursor();
        if let Some(page) = self.active_page_id() {
            self.snapshot(&page);
        }
        if row == 0 {
            if let Some(cols) = self.active_board_mut() {
                if col < cols.len() {
                    cols.remove(col);
                    self.dirty = true;
                }
            }
            self.ui.cursor = Cursor::default();
        } else if row >= 1 && row <= items {
            if let Some(cols) = self.active_board_mut() {
                if col < cols.len() && row - 1 < cols[col].items.len() {
                    cols[col].items.remove(row - 1);
                    self.dirty = true;
                }
            }
            // clamp the row down if it pointed past the new end
            self.ui.cursor.row = self.ui.cursor.row.min(items); // items shrank by 1 → max row now items
        }
    }

    fn active_board_mut(&mut self) -> Option<&mut Vec<crate::protocol::BoardColumn>> {
        let id = self.order.get(self.ui.active)?.clone();
        find_board_mut(&mut self.surfaces.get_mut(&id)?.root)
    }

    fn touch(&mut self, id: String) {
        self.order.retain(|x| x != &id);
        self.order.push(id);
    }
}




/// Split a header into (name, prompt) on the first " — ". No separator → all name.
/// Bottom-most pillar in `lane` (largest `pi < n` with `pi % k == lane`).
fn last_in_lane(lane: usize, n: usize, k: usize) -> usize {
    lane + k * ((n - 1 - lane) / k)
}

fn split_header(h: &str) -> (String, String) {
    match h.split_once(" — ") {
        Some((n, p)) => (n.to_string(), p.to_string()),
        None => (h.to_string(), String::new()),
    }
}

/// Byte offset of char position `char_pos` in `s` (== s.len() if at/after end).
fn byte_idx(s: &str, char_pos: usize) -> usize {
    s.char_indices().nth(char_pos).map(|(b, _)| b).unwrap_or(s.len())
}

/// Make item texts unique within a column (text is identity) by appending " 2",
/// " 3", … to collisions. Ingest + add/edit run this so links stay unambiguous.
pub fn dedupe_items(col: &mut crate::protocol::BoardColumn) {
    let mut seen = std::collections::HashSet::new();
    for it in col.items.iter_mut() {
        if !seen.insert(it.text.clone()) {
            let mut n = 2;
            while !seen.insert(format!("{} {}", it.text, n)) {
                n += 1;
            }
            it.text = format!("{} {}", it.text, n);
        }
    }
}

/// Dedupe every column on a surface (called at ingest).
fn dedupe_board(node: &mut UiNode) {
    match node {
        UiNode::Board { columns, .. } => columns.iter_mut().for_each(dedupe_items),
        UiNode::Panel { children, .. } => children.iter_mut().for_each(dedupe_board),
        _ => {}
    }
}

/// Visit every board item under a node (mutable).
fn for_each_item_mut(node: &mut UiNode, f: &mut impl FnMut(&mut crate::protocol::Item)) {
    match node {
        UiNode::Board { columns, .. } => {
            for c in columns {
                c.items.iter_mut().for_each(&mut *f);
            }
        }
        UiNode::Panel { children, .. } => {
            for ch in children {
                for_each_item_mut(ch, f);
            }
        }
        _ => {}
    }
}

/// First Board node's columns in a UiNode subtree.
fn find_board(node: &UiNode) -> Option<&[crate::protocol::BoardColumn]> {
    match node {
        UiNode::Board { columns, .. } => Some(columns),
        UiNode::Panel { children, .. } => children.iter().find_map(find_board),
        _ => None,
    }
}

pub(crate) fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// True if `it` reaches intent `title` by walking `serves` up the chain via `idx`
/// (title → its serves). Bounded to 5 hops (intent←impl←ctrl←val is 3).
fn reaches_intent(it: &crate::protocol::Item, title: &str, idx: &std::collections::HashMap<String, Vec<String>>) -> bool {
    let t = title.to_lowercase();
    let mut frontier: Vec<String> = it.serves.clone();
    for _ in 0..5 {
        if frontier.iter().any(|s| s.to_lowercase().contains(&t)) {
            return true;
        }
        frontier = frontier.iter().filter_map(|n| idx.get(n)).flatten().cloned().collect();
        if frontier.is_empty() {
            return false;
        }
    }
    false
}

/// Parse a `since:` age like `7d` / `24h` / `30m` (bare = seconds) into seconds.
fn parse_age(s: &str) -> u64 {
    let s = s.trim();
    let (num, mult) = if let Some(n) = s.strip_suffix('d') {
        (n, 86400)
    } else if let Some(n) = s.strip_suffix('h') {
        (n, 3600)
    } else if let Some(n) = s.strip_suffix('m') {
        (n, 60)
    } else {
        (s, 1)
    };
    num.parse::<u64>().unwrap_or(0).saturating_mul(mult)
}

/// First Board node's render style in a UiNode subtree.
fn find_board_style(node: &UiNode) -> Option<BoardStyle> {
    match node {
        UiNode::Board { style, .. } => Some(*style),
        UiNode::Panel { children, .. } => children.iter().find_map(find_board_style),
        _ => None,
    }
}

pub(crate) fn find_board_mut(node: &mut UiNode) -> Option<&mut Vec<crate::protocol::BoardColumn>> {
    match node {
        UiNode::Board { columns, .. } => Some(columns),
        UiNode::Panel { children, .. } => children.iter_mut().find_map(find_board_mut),
        _ => None,
    }
}

/// Apply a patch to the node with `node_id`. Returns true if found.
fn patch_node(node: &mut UiNode, node_id: &str, patch: &UiPatch) -> bool {
    if node.id() == node_id {
        apply_patch(node, patch);
        return true;
    }
    if let UiNode::Panel { children, .. } = node {
        for child in children {
            if patch_node(child, node_id, patch) {
                return true;
            }
        }
    }
    false
}

fn apply_patch(node: &mut UiNode, patch: &UiPatch) {
    match node {
        UiNode::Panel { title, .. } => {
            if let Some(t) = &patch.title {
                *title = Some(t.clone());
            }
        }
        UiNode::Text { text, .. } => {
            if let Some(t) = &patch.text {
                *text = t.clone();
            }
        }
        UiNode::Metric { label, value, .. } => {
            if let Some(l) = &patch.label {
                *label = l.clone();
            }
            if let Some(v) = &patch.value {
                *value = v.clone();
            }
        }
        UiNode::Table { rows, .. } => {
            if let Some(r) = &patch.rows {
                *rows = r.clone();
            }
        }
        UiNode::Log { lines, .. } => {
            if let Some(l) = &patch.lines {
                *lines = l.clone();
            }
        }
        UiNode::Progress { value, .. } => {
            if let Some(v) = patch.progress {
                *value = v;
            }
        }
        UiNode::Board { columns, .. } => {
            if let Some(c) = &patch.board {
                *columns = c.clone();
            }
        }
    }
}

/// Delete a child node by id from any Panel in the tree. The root itself cannot
/// be deleted (no parent). Returns true if removed.
fn delete_node(node: &mut UiNode, node_id: &str) -> bool {
    if let UiNode::Panel { children, .. } = node {
        if let Some(pos) = children.iter().position(|c| c.id() == node_id) {
            children.remove(pos);
            return true;
        }
        for child in children {
            if delete_node(child, node_id) {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::LayoutKind;

    fn create() -> UiMessage {
        UiMessage::CreateSurface {
            id: "s".into(),
            title: "S".into(),
            root: UiNode::Panel {
                id: "root".into(),
                title: None,
                layout: LayoutKind::Vertical,
                children: vec![
                    UiNode::Progress { id: "p".into(), label: "l".into(), value: 0.1 },
                    UiNode::Text { id: "t".into(), text: "old".into() },
                ],
            },
        }
    }

    #[test]
    fn create_then_patch_then_delete() {
        let mut app = AppState::default();
        app.apply(create()).unwrap();
        assert!(app.latest().is_some());
        app.apply(UiMessage::UpdateNode {
            surface_id: "s".into(),
            node_id: "t".into(),
            patch: UiPatch { text: Some("new".into()), ..Default::default() },
        })
        .unwrap();
        app.apply(UiMessage::DeleteNode { surface_id: "s".into(), node_id: "p".into() }).unwrap();
        if let UiNode::Panel { children, .. } = &app.latest().unwrap().root {
            assert_eq!(children.len(), 1);
            assert!(matches!(&children[0], UiNode::Text { text, .. } if text == "new"));
        } else {
            panic!("root not a panel");
        }
    }

    #[test]
    fn patch_rejected_if_it_breaks_validation() {
        let mut app = AppState::default();
        app.apply(create()).unwrap();
        let err = app.apply(UiMessage::UpdateNode {
            surface_id: "s".into(),
            node_id: "p".into(),
            patch: UiPatch { progress: Some(9.0), ..Default::default() },
        });
        assert!(err.is_err());
    }

    fn board_app() -> AppState {
        let mut app = AppState::default();
        app.apply(UiMessage::CreateSurface {
            id: "s".into(),
            title: "S".into(),
            root: UiNode::Board {
                id: "b".into(),
                style: Default::default(),
                columns: vec![
                    crate::protocol::BoardColumn { header: "WHO".into(), key: Some("who".into()), items: vec!["a".into(), "b".into()] },
                    crate::protocol::BoardColumn { header: "JOBS".into(), key: Some("jobs".into()), items: vec!["x".into()] },
                ],
            },
        })
        .unwrap();
        app
    }

    fn who_items(app: &AppState) -> Vec<String> {
        match &app.active_surface().unwrap().root {
            UiNode::Board { columns, .. } => columns[0].items.iter().map(|it| it.text.clone()).collect(),
            _ => panic!(),
        }
    }

    fn ncols(app: &AppState) -> usize {
        find_board(&app.active_surface().unwrap().root).unwrap().len()
    }

    #[test]
    fn add_item_via_plus() {
        let mut app = board_app();
        app.ui.cursor = Cursor { col: 0, row: 3 }; // WHO "+" (2 items → row 3)
        app.activate();
        assert!(matches!(app.ui.mode, Mode::Edit(_)));
        for c in "new".chars() {
            app.edit_char(c);
        }
        app.input_commit(false);
        assert_eq!(who_items(&app), vec!["a", "b", "new"]);
    }

    #[test]
    fn edit_existing_replaces_on_type() {
        let mut app = board_app();
        app.select_board_item(0, 1); // WHO item "b" → row 2
        app.activate();
        match &app.ui.mode {
            Mode::Edit(e) => assert!(e.select_all && e.buffer == "b"),
            _ => panic!("not editing"),
        }
        app.edit_char('Z');
        app.edit_char('z');
        app.input_commit(false);
        assert_eq!(who_items(&app), vec!["a", "Zz"]);
    }

    #[test]
    fn edit_arrow_collapses_selection_then_inserts() {
        let mut app = board_app();
        app.select_board_item(0, 0); // item "a"
        app.activate(); // select_all
        app.edit_right(); // collapse to end
        app.edit_char('!');
        app.input_commit(false);
        assert_eq!(who_items(&app), vec!["a!", "b"]);
    }

    #[test]
    fn delete_item_needs_confirm() {
        let mut app = board_app();
        app.select_board_item(0, 1); // WHO item "b"
        app.begin_delete();
        assert_eq!(app.ui.mode, Mode::ConfirmDelete);
        app.confirm_delete();
        assert_eq!(who_items(&app), vec!["a"]);
    }

    #[test]
    fn keyboard_add_delete_column_with_undo() {
        let mut app = board_app();
        let c0 = ncols(&app);
        app.add_column();
        assert_eq!(ncols(&app), c0 + 1);
        assert!(app.on_header()); // cursor on the new header
        app.undo();
        assert_eq!(ncols(&app), c0);

        app.ui.cursor = Cursor { col: 0, row: 0 }; // a header
        app.begin_delete();
        assert_eq!(app.ui.mode, Mode::ConfirmDelete);
        app.confirm_delete();
        assert_eq!(ncols(&app), c0 - 1);
        app.undo();
        assert_eq!(ncols(&app), c0);
    }

    #[test]
    fn header_two_section_edit() {
        let mut app = board_app();
        app.ui.cursor = Cursor { col: 0, row: 0 }; // header
        app.activate();
        for c in "USERS".chars() {
            app.edit_char(c);
        }
        app.switch_header_part(); // → prompt half
        for c in "who decides".chars() {
            app.edit_char(c);
        }
        app.input_commit(false);
        let cols = find_board(&app.active_surface().unwrap().root).unwrap();
        assert_eq!(cols[0].header, "USERS — who decides");
    }

    #[test]
    fn vertical_flows_into_next_pillar_skipping_header() {
        // lanes default 1 → WHO above JOBS in one grid column.
        // From WHO "+" (col0,row3), down → JOBS first item (col1,row1) — header skipped.
        let mut app = board_app();
        app.ui.cursor = Cursor { col: 0, row: 3 };
        app.board_select_vert(1);
        assert_eq!(app.ui.cursor, Cursor { col: 1, row: 1 });
    }

    #[test]
    fn vertical_loops_around_through_button() {
        // One lane (k=1): WHO[a,b] · JOBS[x] · button(col2). Bottom of the items
        // is JOBS "+" (col1,row2); below that is the button, then it loops.
        let mut app = board_app();
        app.ui.cursor = Cursor { col: 1, row: 2 }; // JOBS "+"
        app.board_select_vert(1);
        assert_eq!(app.ui.cursor, Cursor { col: 2, row: 0 }, "down → button");
        app.board_select_vert(1);
        assert_eq!(app.ui.cursor, Cursor { col: 0, row: 1 }, "button → loops to top");
        // up from the top first item wraps to the bottom of the lane = the button.
        app.board_select_vert(-1);
        assert_eq!(app.ui.cursor, Cursor { col: 2, row: 0 }, "up loops to the button");
    }

    #[test]
    fn working_cells_span_by_title_and_serves_and_survive_cursor() {
        use crate::protocol::Item;
        let mut app = AppState::default();
        app.apply(UiMessage::CreateSurface {
            id: "intent".into(),
            title: "i".into(),
            root: UiNode::Board {
                id: "b".into(),
                style: Default::default(),
                columns: vec![
                    crate::protocol::BoardColumn { header: "WHO".into(), key: Some("who".into()), items: vec![Item::new("A")] },
                    crate::protocol::BoardColumn {
                        header: "JOBS".into(),
                        key: Some("jobs".into()),
                        items: vec![Item { text: "build A".into(), serves: vec!["A".into()], ..Default::default() }],
                    },
                ],
            },
        })
        .unwrap();
        app.ui.working.insert("A".to_string());

        let cells = app.working_cells();
        assert!(cells.contains(&(0, 0)), "the intent item itself pulses");
        assert!(cells.contains(&(1, 0)), "an item in another pillar that serves it pulses");
        assert_eq!(cells.len(), 2);

        // cursor ON the working item — still in the set (keeps blinking under the cursor)
        app.ui.cursor = Cursor { col: 0, row: 1 };
        assert!(app.working_cells().contains(&(0, 0)), "selection must not drop it from the pulse set");

        // clearing the working set empties it
        app.ui.working.clear();
        assert!(app.working_cells().is_empty());
    }

    #[test]
    fn panel_groups_by_component_and_fine_text() {
        use crate::protocol::{BoardColumn, BoardStyle, Item};
        // coarse axis = component (serves), not the free category
        let it = |t: &str, comp: &str| Item { text: t.into(), serves: vec![comp.into()], ..Default::default() };
        let mut app = AppState::default();
        app.apply(UiMessage::CreateSurface {
            id: "control".into(),
            title: "control".into(),
            root: UiNode::Board {
                id: "c".into(),
                style: BoardStyle::Panel,
                columns: vec![BoardColumn {
                    header: "RULES".into(),
                    key: None,
                    items: vec![it("RBAC", "Auth API"), it("max size", "Ingestion"), it("rate cap", "Ingestion")],
                }],
            },
        })
        .unwrap();
        // components sorted: Auth API, Ingestion → panel_cat 0=all, 1=Auth API, 2=Ingestion
        assert_eq!(app.panel_components(), vec![("Auth API".into(), 1), ("Ingestion".into(), 2)]);
        assert_eq!(app.panel_results().len(), 3, "all, no filter");
        app.ui.panel_cat = 2; // coarse: Ingestion
        assert_eq!(app.panel_results().len(), 2);
        app.ui.panel_query = "rate".into(); // fine
        assert_eq!(app.panel_results().len(), 1);
        app.ui.panel_cat = 1; // Auth API + fine "rate" → none
        assert_eq!(app.panel_results().len(), 0);
    }

    #[test]
    fn search_filters_text_category_status_and_chain() {
        use crate::protocol::{BoardColumn, BoardStyle, Item, Status};
        let mut app = AppState::default();
        let board = |id: &str, items: Vec<Item>| UiMessage::CreateSurface {
            id: id.into(),
            title: id.into(),
            root: UiNode::Board { id: format!("{id}b"), style: BoardStyle::List, columns: vec![BoardColumn { header: "H".into(), key: None, items }] },
        };
        let ctl = |t: &str, cat: &str, status| Item {
            text: t.into(),
            serves: vec!["Auth API".into()],
            category: Some(cat.into()),
            status,
            ..Default::default()
        };
        app.apply(board("intent", vec![Item::new("Login")])).unwrap();
        app.apply(board("impl", vec![Item { text: "Auth API".into(), serves: vec!["Login".into()], ..Default::default() }])).unwrap();
        app.apply(board("control", vec![ctl("rate cap", "limits", Some(Status::Pending)), ctl("RBAC", "authz", Some(Status::Ok))])).unwrap();
        app.set_active(2); // control board

        let run = |app: &mut AppState, q: &str| {
            app.ui.search = Some(Search { query: q.into(), cursor: 0 });
            app.search_results().len()
        };
        assert_eq!(run(&mut app, ""), 2, "empty = all");
        assert_eq!(run(&mut app, "cat:limits"), 1);
        assert_eq!(run(&mut app, "status:ok"), 1);
        assert_eq!(run(&mut app, "rate"), 1, "free term matches name");
        assert_eq!(run(&mut app, "link:auth"), 2, "both serve Auth API");
        assert_eq!(run(&mut app, "intent:login"), 2, "transitive: control→impl→intent");
        assert_eq!(run(&mut app, "cat:limits status:pending"), 1, "tokens AND");
        assert_eq!(run(&mut app, "cat:authz status:pending"), 0);
    }

    #[test]
    fn link_picker_targets_parent_layer() {
        use crate::protocol::{BoardColumn, BoardStyle, Item};
        let mut app = AppState::default();
        let board = |id: &str, style, items: Vec<Item>| UiMessage::CreateSurface {
            id: id.into(),
            title: id.into(),
            root: UiNode::Board { id: format!("{id}b"), style, columns: vec![BoardColumn { header: "H".into(), key: None, items }] },
        };
        app.apply(board("intent", BoardStyle::List, vec![Item::new("I1"), Item::new("I2")])).unwrap();
        app.apply(board("impl", BoardStyle::Cards, vec![Item::new("C1")])).unwrap();
        app.set_active(1); // impl
        app.ui.cursor = Cursor { col: 0, row: 1 }; // C1
        assert_eq!(app.link_candidates(), vec!["I1", "I2"], "candidates = parent layer (intents)");
        app.begin_link_pick();
        assert_eq!(app.ui.inspector, InspectorMode::LinkPick);
        app.link_pick_toggle(); // I1
        assert_eq!(app.selected_item().unwrap().serves, vec!["I1"]);
        app.link_pick_move(1);
        app.link_pick_toggle(); // + I2
        assert_eq!(app.selected_item().unwrap().serves, vec!["I1", "I2"]);
        app.link_pick_move(-1);
        app.link_pick_toggle(); // - I1
        assert_eq!(app.selected_item().unwrap().serves, vec!["I2"]);
        // the spine has no parent to link to
        app.set_active(0);
        app.ui.cursor = Cursor { col: 0, row: 1 };
        assert!(app.link_candidates().is_empty());
    }

    #[test]
    fn cards_header_vert_walks_sections_in_order() {
        use crate::protocol::{BoardColumn, BoardStyle, Item};
        let mut app = AppState::default();
        let cols: Vec<BoardColumn> = ["A", "B", "C", "D"]
            .iter()
            .map(|h| BoardColumn { header: h.to_string(), key: None, items: vec![Item::new("x")] })
            .collect();
        app.apply(UiMessage::CreateSurface {
            id: "impl".into(),
            title: "impl".into(),
            root: UiNode::Board { id: "m".into(), style: BoardStyle::Cards, columns: cols },
        })
        .unwrap();
        app.ui.lanes.set(3); // per_row=3 — the old lane math would jump col+3 out of order
        app.ui.cursor = Cursor { col: 0, row: 0 }; // section A's header
        // Shift+J steps header→header in section order, not by per_row
        for expect in [1usize, 2, 3] {
            app.header_vert(1);
            assert!(app.on_header(), "lands on a header");
            assert_eq!(app.ui.cursor.col, expect, "next section header in order");
        }
        app.header_vert(1);
        assert!(app.on_add_button(), "after the last section → add-card button");
        app.header_vert(1);
        assert_eq!(app.ui.cursor.col, 0, "rolls back to the first section");
        // and upward is the reverse order
        app.ui.cursor = Cursor { col: 2, row: 0 };
        app.header_vert(-1);
        assert_eq!(app.ui.cursor.col, 1, "Shift+K → previous section header");
    }

    #[test]
    fn cards_2d_nav_rows_sections_and_button() {
        use crate::protocol::{BoardColumn, BoardStyle, Item};
        let mut app = AppState::default();
        app.apply(UiMessage::CreateSurface {
            id: "impl".into(),
            title: "impl".into(),
            root: UiNode::Board {
                id: "m".into(),
                style: BoardStyle::Cards,
                columns: vec![
                    BoardColumn { header: "FE".into(), key: None, items: vec![Item::new("A"), Item::new("B"), Item::new("C"), Item::new("D")] },
                    BoardColumn { header: "BE".into(), key: None, items: vec![Item::new("X")] },
                ],
            },
        })
        .unwrap();
        app.ui.lanes.set(2); // per_row = 2 (renderer would set this)
        // FE boxes wrap: [A B] [C D] [+] ; BE: [X +]
        app.ui.cursor = Cursor { col: 0, row: 1 }; // A
        app.board_select_horiz(1);
        assert_eq!(app.ui.cursor, Cursor { col: 0, row: 2 }, "l → B (next box)");
        app.board_select_vert(1);
        assert_eq!(app.ui.cursor, Cursor { col: 0, row: 4 }, "j → D (down a visual row)");
        app.board_select_vert(1);
        assert_eq!(app.ui.cursor, Cursor { col: 1, row: 2 }, "j off FE bottom → BE, same column");
        app.board_select_vert(1);
        assert!(app.on_add_button(), "j off the last section → add-card button");
        app.board_select_vert(-1);
        assert_eq!(app.ui.cursor, Cursor { col: 1, row: 2 }, "k from button → last section's last box");

        // overflow loops around
        app.ui.cursor = Cursor { col: 2, row: 0 }; // the add-card button
        app.board_select_vert(1);
        assert_eq!(app.ui.cursor, Cursor { col: 0, row: 1 }, "down off button loops to first box");
        app.board_select_vert(-1);
        assert!(app.on_add_button(), "up off the top loops to the button");
        // horizontal wraps within the current VISUAL ROW (per_row=2):
        // FE boxes [A B][C D][+] → row 0 is {A,B}
        app.ui.cursor = Cursor { col: 0, row: 1 }; // A (box 0)
        app.board_select_horiz(1);
        assert_eq!(app.ui.cursor, Cursor { col: 0, row: 2 }, "l → B (end of row 0)");
        app.board_select_horiz(1);
        assert_eq!(app.ui.cursor, Cursor { col: 0, row: 1 }, "l wraps within row 0 → A");
        app.board_select_horiz(-1);
        assert_eq!(app.ui.cursor, Cursor { col: 0, row: 2 }, "h wraps within row 0 → B");
        // a lone box on its own row (the "+") stays put horizontally
        app.ui.cursor = Cursor { col: 0, row: 5 };
        app.board_select_horiz(1);
        assert_eq!(app.ui.cursor, Cursor { col: 0, row: 5 }, "lone box row → no horizontal move");
    }

    #[test]
    fn button_is_a_grid_cell() {
        let mut app = board_app(); // 2 pillars → button at col 2
        // focused_button / on_add_button only when the cursor is past the pillars
        app.ui.cursor = Cursor { col: 0, row: 1 };
        assert!(!app.on_add_button() && app.focused_button().is_none());
        app.ui.cursor = Cursor { col: 2, row: 0 };
        assert!(app.on_add_button());
        assert_eq!(app.focused_button(), Some(Button::AddPillar));
        // not a header, no item selected
        assert!(!app.on_header() && app.selected_flat().is_none() && app.header_col().is_none());
    }

    #[test]
    fn button_press_and_enter_add_a_pillar() {
        let mut app = board_app();
        let before = ncols(&app);
        app.press(Button::AddPillar);
        assert_eq!(ncols(&app), before + 1, "press adds");
        // Enter (activate) on the button does the same via the listener.
        app.ui.cursor = Cursor { col: ncols(&app), row: 0 }; // the new button slot
        app.activate();
        assert_eq!(ncols(&app), before + 2, "Enter on button adds");
    }

    #[test]
    fn button_ignores_delete_and_move() {
        let mut app = board_app();
        app.ui.cursor = Cursor { col: 2, row: 0 }; // button
        app.begin_delete();
        assert_eq!(app.ui.mode, Mode::Normal, "no delete on the button");
        let before = ncols(&app);
        app.move_item(1);
        assert_eq!(ncols(&app), before, "move_item is a no-op on the button");
        assert!(app.on_add_button(), "still on the button");
    }

    #[test]
    fn horiz_reaches_button_in_its_row() {
        // 3 pillars, 2 lanes → grid row 1 holds P2 (col2) and the button (col3).
        let mut app = AppState::default();
        let cols: Vec<_> = (0..3)
            .map(|i| crate::protocol::BoardColumn {
                header: format!("P{i}"),
                key: None,
                items: vec![format!("i{i}").into()],
            })
            .collect();
        app.apply(UiMessage::CreateSurface {
            id: "s".into(),
            title: "S".into(),
            root: UiNode::Board { id: "b".into(), columns: cols, style: Default::default() },
        })
        .unwrap();
        app.ui.lanes.set(2);
        app.ui.cursor = Cursor { col: 2, row: 1 }; // P2 item
        app.board_select_horiz(1); // → button to its right
        assert!(app.on_add_button(), "h/l reaches the button in the same grid row");
    }

    #[test]
    fn grid_horiz_and_below() {
        // 4 pillars, 2 grid columns: (0,1 top row) (2,3 next row).
        let mut app = AppState::default();
        let cols: Vec<_> = (0..4)
            .map(|i| crate::protocol::BoardColumn {
                header: format!("P{i}"),
                key: None,
                items: vec![format!("i{i}").into()],
            })
            .collect();
        app.apply(UiMessage::CreateSurface {
            id: "s".into(),
            title: "S".into(),
            root: UiNode::Board { id: "b".into(), columns: cols, style: Default::default() },
        })
        .unwrap();
        app.ui.lanes.set(2);
        app.ui.cursor = Cursor { col: 0, row: 1 };
        app.board_select_horiz(1); // → P1 (right)
        assert_eq!(app.ui.cursor.col, 1);
        app.board_select_horiz(1); // last in row → wrap to P0
        assert_eq!(app.ui.cursor.col, 0);
        app.ui.cursor = Cursor { col: 0, row: 2 }; // P0 "+"
        app.board_select_vert(1); // → pillar below in the grid column (P2 first item)
        assert_eq!(app.ui.cursor, Cursor { col: 2, row: 1 });
    }

    #[test]
    fn delete_on_plus_is_noop() {
        let mut app = board_app();
        app.ui.cursor = Cursor { col: 0, row: 3 }; // WHO "+"
        app.begin_delete();
        assert_eq!(app.ui.mode, Mode::Normal); // can't delete the add affordance
    }

}
