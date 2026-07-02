//! Input controllers, partitioned per page so they don't tangle.
//!
//! Flow: the active page's controller gets first crack at a key. If it consumes
//! the key (returns true), we stop. Otherwise the key falls through to the
//! global handler (quit, page switch). This keeps each page's controls
//! self-contained — adding a new interactive page means adding one `*_keys`
//! function and a `PageKind` arm, touching nothing else.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use arte_core::protocol::BoardStyle;
use arte_core::state::{AppState, InspectorMode, Mode, PageKind};

pub enum Outcome {
    Quit,
    Continue,
}

/// Top-level key handling for the editable store.
pub fn handle_key(app: &mut AppState, key: KeyEvent) -> Outcome {
    // Quit confirmation: y/q quits, anything else cancels.
    if app.ui.confirm_quit {
        return match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Char('q') => Outcome::Quit,
            _ => {
                app.ui.confirm_quit = false;
                Outcome::Continue
            }
        };
    }
    // Delete confirmation (any page, incl. the panel): y removes, else cancels.
    if matches!(app.ui.mode, Mode::ConfirmDelete) {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => app.confirm_delete(),
            _ => app.cancel(),
        }
        return Outcome::Continue;
    }
    // Help overlay: any key closes it; `?` opens it (unless typing).
    if app.ui.help {
        app.ui.help = false;
        return Outcome::Continue;
    }
    if matches!(key.code, KeyCode::Char('?')) && !matches!(app.ui.mode, Mode::Edit(_)) {
        app.ui.help = true;
        return Outcome::Continue;
    }
    // Glossary pop-up (modal): trigram reference + area-filter picker.
    if app.ui.glossary {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => app.glossary_move(-1),
            KeyCode::Down | KeyCode::Char('j') => app.glossary_move(1),
            KeyCode::Enter => app.glossary_apply(),
            KeyCode::Esc | KeyCode::Char('g') => app.ui.glossary = false,
            _ => {}
        }
        return Outcome::Continue;
    }
    let on_sheet = app.active_board_style() == BoardStyle::Sheet && !matches!(app.ui.mode, Mode::Edit(_));
    // `g` opens the glossary on a sheet page; Esc clears an active area filter.
    if on_sheet && matches!(key.code, KeyCode::Char('g')) {
        app.open_glossary();
        return Outcome::Continue;
    }
    if on_sheet && matches!(key.code, KeyCode::Esc) && app.ui.sheet_filter.is_some() {
        app.set_sheet_filter(None);
        return Outcome::Continue;
    }
    // Search overlay (modal): all chars type into the query (live filter);
    // arrows move the result cursor, Enter jumps to it, Esc closes.
    if app.ui.search.is_some() {
        match key.code {
            KeyCode::Esc => app.ui.search = None,
            KeyCode::Enter => app.search_jump(),
            KeyCode::Up => app.search_move(-1),
            KeyCode::Down => app.search_move(1),
            KeyCode::Backspace => app.search_backspace(),
            KeyCode::Char(c) => app.search_char(c),
            _ => {}
        }
        return Outcome::Continue;
    }
    // File browser overlay (modal): owns all keys while open.
    if app.ui.browse.is_some() {
        browse_keys(app, key);
        return Outcome::Continue;
    }
    // Inspector overlay: `o` opens it on the selected element; while open, `O`/Enter
    // opens the first attachment externally, any other key closes it.
    if app.ui.inspect {
        // Inspector text edit (note / new attachment): feed the inline editor.
        if matches!(app.ui.mode, Mode::Edit(_)) {
            match key.code {
                // Enter accepts + closes; Shift+Enter adds another line (note/attach).
                KeyCode::Enter => {
                    let cont = key.modifiers.contains(KeyModifiers::SHIFT)
                        && matches!(app.ui.inspector, InspectorMode::Note | InspectorMode::Attach);
                    app.input_commit(cont);
                }
                KeyCode::Esc => app.cancel(),
                KeyCode::Backspace => app.edit_backspace(),
                KeyCode::Left => app.edit_left(),
                KeyCode::Right => app.edit_right(),
                // category combobox: ↑↓ pick an existing value; typing makes a new one
                KeyCode::Up if app.ui.inspector == InspectorMode::Category => app.cat_pick_move(-1),
                KeyCode::Down if app.ui.inspector == InspectorMode::Category => app.cat_pick_move(1),
                KeyCode::Char(c) => app.edit_char(c),
                _ => {}
            }
            return Outcome::Continue;
        }
        // Link picker: j/k move, Space/Enter toggle, l/Esc close the picker.
        if app.ui.inspector == InspectorMode::LinkPick {
            match key.code {
                KeyCode::Char('j') | KeyCode::Down => app.link_pick_move(1),
                KeyCode::Char('k') | KeyCode::Up => app.link_pick_move(-1),
                KeyCode::Char(' ') => app.link_pick_toggle(), // Space toggles (applied live)
                _ => app.ui.inspector = InspectorMode::None, // Enter/Esc/l/anything → accept + close
            }
            return Outcome::Continue;
        }
        match key.code {
            KeyCode::Char('e') => app.begin_detail_edit(), // edit the details (text)
            KeyCode::Char('s') => app.cycle_status(),       // cycle status none→ok→pending→ko
            KeyCode::Char('l') => app.begin_link_pick(),    // pick an existing parent to link to
            KeyCode::Char('n') => app.begin_note_edit(),    // type the note
            KeyCode::Char('c') => app.begin_category_edit(), // category combobox
            KeyCode::Char('a') => app.begin_attach_add(),   // type an attachment ref (URL/path)
            KeyCode::Char('b') => browse_attach(app),       // pick a local file via the OS dialog
            KeyCode::Char('m') => app.begin_comment_edit(),  // edit the result comment
            KeyCode::Char('d') => app.toggle_derived(),      // mark parentless-by-design (derived)
            KeyCode::Char('O') => open_attachment(app),
            _ => app.ui.inspect = false, // o / Esc / anything else closes
        }
        return Outcome::Continue;
    }
    // Panel quick-edit: inline detail/method edit (no inspector overlay). Tab walks
    // detail → method combobox → the next control. Enter accepts, Esc cancels.
    if app.active_board_style() == BoardStyle::Panel && !app.ui.inspect && matches!(app.ui.mode, Mode::Edit(_)) {
        match key.code {
            KeyCode::Enter => app.input_commit(false),
            KeyCode::Tab => app.panel_quick_tab(),
            KeyCode::Esc => app.cancel(),
            KeyCode::Up if app.ui.inspector == InspectorMode::Category => app.cat_pick_move(-1),
            KeyCode::Down if app.ui.inspector == InspectorMode::Category => app.cat_pick_move(1),
            KeyCode::Backspace => app.edit_backspace(),
            KeyCode::Left => app.edit_left(),
            KeyCode::Right => app.edit_right(),
            KeyCode::Char(c) => app.edit_char(c),
            _ => {}
        }
        return Outcome::Continue;
    }
    if matches!(key.code, KeyCode::Char('o')) && !matches!(app.ui.mode, Mode::Edit(_)) {
        // o = inspect everywhere; the panel must sync its cursor first.
        if app.active_board_style() == BoardStyle::Panel {
            app.panel_open();
            return Outcome::Continue;
        }
        if app.selected_item().is_some() {
            app.ui.inspect = true;
            return Outcome::Continue;
        }
    }
    if dispatch_page(app, key) {
        return Outcome::Continue; // a page consumed it
    }
    // Global keys — only reached when the active page didn't want the key.
    match key.code {
        KeyCode::Char('q') => app.ui.confirm_quit = true, // ask first; Esc no longer quits
        KeyCode::Tab => app.cycle(1),
        KeyCode::BackTab => app.cycle(-1),
        KeyCode::Char(c @ '1'..='9') => app.set_active(c as usize - '1' as usize),
        _ => {}
    }
    Outcome::Continue
}

/// Open the selected element's first attachment with the OS default handler.
fn open_attachment(app: &AppState) {
    if let Some(path) = app.selected_item().and_then(|i| i.attachments.first()) {
        let _ = open_external(path);
    }
}

/// Hand a path/URL to the OS default handler. Same behaviour on all three OSes.
fn open_external(target: &str) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = std::process::Command::new("open");
        c.arg(target);
        c
    };
    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = std::process::Command::new("cmd");
        c.args(["/C", "start", "", target]); // empty title arg so paths with spaces work
        c
    };
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    let mut cmd = {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(target);
        c
    };
    cmd.spawn().map(|_| ())
}

// --- in-TUI file browser (cross-platform: std::fs only) ----------------------
/// `b` in the inspector: open the browser at the current working directory.
fn browse_attach(app: &mut AppState) {
    let dir = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    open_browser(app, dir);
}

fn open_browser(app: &mut AppState, dir: std::path::PathBuf) {
    let entries = read_entries(&dir);
    app.ui.browse = Some(arte_core::state::Browse { dir: dir.display().to_string(), entries, cursor: 0 });
}

/// Directory listing: dirs first (A→Z), then files; hidden entries skipped; `..` first.
fn read_entries(dir: &std::path::Path) -> Vec<(String, bool)> {
    let mut v: Vec<(String, bool)> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue; // ponytail: skip dotfiles; toggle later if needed
            }
            v.push((name, e.path().is_dir()));
        }
    }
    v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase())));
    v.insert(0, ("..".to_string(), true));
    v
}

/// Browser keys: j/k move, Enter open-dir/pick-file, Esc/b cancel.
fn browse_keys(app: &mut AppState, key: KeyEvent) {
    let (n, dir, entry) = match &app.ui.browse {
        Some(b) => (b.entries.len(), b.dir.clone(), b.entries.get(b.cursor).cloned()),
        None => return,
    };
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            if let (Some(b), true) = (app.ui.browse.as_mut(), n > 0) {
                b.cursor = (b.cursor + 1) % n;
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if let (Some(b), true) = (app.ui.browse.as_mut(), n > 0) {
                b.cursor = (b.cursor + n - 1) % n;
            }
        }
        KeyCode::Esc | KeyCode::Char('b') => app.ui.browse = None,
        KeyCode::Enter => {
            if let Some((name, is_dir)) = entry {
                let cur = std::path::Path::new(&dir);
                if name == ".." {
                    if let Some(p) = cur.parent() {
                        open_browser(app, p.to_path_buf());
                    }
                } else if is_dir {
                    open_browser(app, cur.join(name));
                } else {
                    app.attach_path(cur.join(name).display().to_string());
                    app.ui.browse = None; // back to the inspector (still open)
                }
            }
        }
        _ => {}
    }
}

fn dispatch_page(app: &mut AppState, key: KeyEvent) -> bool {
    match app.active_page_kind() {
        PageKind::Board if app.active_board_style() == BoardStyle::Panel => panel_keys(app, key),
        PageKind::Board => board_keys(app, key),
        PageKind::Other => false,
    }
}

/// Registry panel (control.spec): focused on the controls pane by default (nav,
/// add, delete, inspect); `/` enters the fine filter, Esc leaves it back to nav.
fn panel_keys(app: &mut AppState, key: KeyEvent) -> bool {
    if app.ui.panel_list_focus {
        match key.code {
            KeyCode::Char('/') => app.ui.panel_list_focus = false, // enter the filter
            KeyCode::Char('c') => app.ui.panel_hide_cats = !app.ui.panel_hide_cats, // hide/show categories
            KeyCode::Char('n') => app.add_column(), // n = new section (consistent across pages)
            KeyCode::Backspace | KeyCode::Delete => app.panel_delete(), // delete (asks to confirm)
            KeyCode::Char('j') | KeyCode::Down => app.panel_move(1),
            KeyCode::Char('k') | KeyCode::Up => app.panel_move(-1),
            KeyCode::Char('h') | KeyCode::Left => app.panel_cat_move(-1),
            KeyCode::Char('l') | KeyCode::Right => app.panel_cat_move(1),
            KeyCode::Enter => app.panel_quick_edit(), // quick-edit the detail inline
            KeyCode::Char('u') => {
                app.undo();
            }
            KeyCode::Char('r') => {
                app.redo();
            }
            _ => return false, // o (inspect) / q / Tab / digits fall through to global
        }
        true
    } else {
        match key.code {
            KeyCode::Esc => app.ui.panel_list_focus = true, // leave the filter → list nav
            KeyCode::Enter => app.panel_quick_edit(),
            KeyCode::Up => app.panel_move(-1),
            KeyCode::Down => app.panel_move(1),
            KeyCode::Left => app.panel_cat_move(-1),
            KeyCode::Right => app.panel_cat_move(1),
            KeyCode::Backspace => app.panel_backspace(),
            KeyCode::Char(c) => app.panel_char(c),
            _ => return false, // Tab/BackTab → global page switch
        }
        true
    }
}

/// Board (intent.map): items + headers. Ctrl+←/→ moves across headers (and
/// between a header's name/prompt while editing); plain arrows move items.
fn board_keys(app: &mut AppState, key: KeyEvent) -> bool {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

    // Inline editing swallows everything until Enter/Esc.
    if matches!(app.ui.mode, Mode::Edit(_)) {
        // name ↔ prompt: Ctrl+←/→ (where supported) or Tab (works everywhere).
        if (ctrl && matches!(key.code, KeyCode::Left | KeyCode::Right))
            || matches!(key.code, KeyCode::Tab | KeyCode::BackTab)
        {
            app.switch_header_part();
            return true;
        }
        match key.code {
            KeyCode::Esc => app.cancel(),
            KeyCode::Enter => app.input_commit(false),
            KeyCode::Left => app.edit_left(),
            KeyCode::Right => app.edit_right(),
            KeyCode::Backspace => app.edit_backspace(),
            KeyCode::Delete => app.edit_delete_forward(),
            KeyCode::Char(c) => app.edit_char(c),
            _ => {}
        }
        return true;
    }
    // Esc leaves a header back to items (else falls through to global quit).
    if app.on_header() && key.code == KeyCode::Esc {
        app.focus_items();
        return true;
    }

    // Header layer (aliases for `t`): Ctrl+arrows where the terminal sends them.
    if ctrl {
        match key.code {
            KeyCode::Up => {
                app.header_vert(-1);
                return true;
            }
            KeyCode::Down => {
                app.header_vert(1);
                return true;
            }
            KeyCode::Left => {
                app.header_move(-1);
                return true;
            }
            KeyCode::Right => {
                app.header_move(1);
                return true;
            }
            _ => {}
        }
    }

    // lowercase hjkl = items (loops); SHIFT HJKL = the header layer; t = header.
    if let KeyCode::Char(c) = key.code {
        let shift = key.modifiers.contains(KeyModifiers::SHIFT) || c.is_ascii_uppercase();
        let lc = c.to_ascii_lowercase();
        if shift {
            match lc {
                'k' => {
                    app.header_vert(-1);
                    return true;
                }
                'j' => {
                    app.header_vert(1);
                    return true;
                }
                'h' => {
                    app.header_move(-1);
                    return true;
                }
                'l' => {
                    app.header_move(1);
                    return true;
                }
                _ => {}
            }
        }
        match lc {
            'h' => app.board_select_horiz(-1),
            'l' => app.board_select_horiz(1),
            'k' | 'K' => app.board_select_vert(-1),
            'j' | 'J' => app.board_select_vert(1),
            '<' => app.move_item(-1),
            '>' => app.move_item(1),
            '/' => app.open_search(), // filter the board (yozefu-style query bar)
            't' => app.focus_header(), // deliberate detour to the column header
            '[' => app.header_move(-1), // navigate headers (reliable, no modifier)
            ']' => app.header_move(1),
            'n' => app.add_column(),
            'u' => {
                app.undo();
            }
            'r' => {
                app.redo();
            }
            _ => return false,
        }
        return true;
    }
    match key.code {
        KeyCode::Up => app.board_select_vert(-1),
        KeyCode::Down => app.board_select_vert(1),
        KeyCode::Left => app.board_select_horiz(-1),
        KeyCode::Right => app.board_select_horiz(1),
        KeyCode::Enter => app.activate(),
        KeyCode::Backspace | KeyCode::Delete => app.begin_delete(),
        _ => return false, // q / Tab / digits / ? fall through to global
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use arte_core::protocol::{BoardColumn, Status, UiMessage, UiNode};
    use arte_core::state::AppState;
    use crossterm::event::KeyEvent;

    // A human can audit + modify every field of an element through the inspector UI.
    #[test]
    fn ui_inspector_edits_every_field() {
        let mut a = board_app(); // list board: WHO[a], JOBS[x]
        a.select_board_item(0, 0); // the WHO item "a"
        let key = |a: &mut AppState, c: KeyCode| {
            handle_key(a, KeyEvent::new(c, KeyModifiers::NONE));
        };
        let typ = |a: &mut AppState, s: &str| {
            for ch in s.chars() {
                handle_key(a, KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
            }
        };
        key(&mut a, KeyCode::Char('o')); // open inspector
        assert!(a.ui.inspect);
        key(&mut a, KeyCode::Char('e')); // details (text)
        typ(&mut a, "renamed");
        key(&mut a, KeyCode::Enter);
        key(&mut a, KeyCode::Char('s')); // status: none → ok
        key(&mut a, KeyCode::Char('c')); // category
        typ(&mut a, "authz");
        key(&mut a, KeyCode::Enter);
        key(&mut a, KeyCode::Char('n')); // note: Enter accepts + closes
        typ(&mut a, "guard it");
        key(&mut a, KeyCode::Enter);
        key(&mut a, KeyCode::Char('a')); // attachment: Enter accepts + closes
        typ(&mut a, "f.pdf");
        key(&mut a, KeyCode::Enter);

        let it = a.selected_item().unwrap();
        assert_eq!(it.text, "renamed");
        assert_eq!(it.status, Some(Status::Ok));
        assert_eq!(it.category.as_deref(), Some("authz"));
        assert_eq!(it.note, vec!["guard it"]);
        assert_eq!(it.attachments, vec!["f.pdf"]);
    }

    fn board_app() -> AppState {
        let mut a = AppState::default();
        a.apply(UiMessage::CreateSurface {
            id: "b".into(),
            title: "b".into(),
            root: UiNode::Board {
                id: "bd".into(),
                style: Default::default(),
                columns: vec![
                    BoardColumn { header: "WHO".into(), key: Some("who".into()), items: vec!["a".into()] },
                    BoardColumn { header: "JOBS".into(), key: Some("jobs".into()), items: vec!["x".into()] },
                ],
            },
        })
        .unwrap();
        a
    }

    fn ncols(a: &AppState) -> usize {
        match &a.active_surface().unwrap().root {
            UiNode::Board { columns, .. } => columns.len(),
            _ => 0,
        }
    }

    // k never steps onto a header (it loops through items + button); t does.
    #[test]
    fn k_never_hits_header_t_does() {
        let mut a = board_app();
        a.select_board_item(0, 0);
        handle_key(&mut a, KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE));
        assert!(!a.on_header(), "k must not reach the header");
        a.select_board_item(0, 0);
        handle_key(&mut a, KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE));
        assert!(a.on_header(), "t goes to the header");
    }

    // Plain j/k walk through the add-pillar button (the last grid cell).
    #[test]
    fn jk_walk_through_button() {
        let mut a = board_app(); // WHO[a], JOBS[x], k=1: WHO · JOBS · button
        a.select_board_item(0, 0); // (0,1)
        // down: WHO+ , JOBS item, JOBS+ , button , loop to top
        let downs = ['j'; 4];
        for c in downs {
            handle_key(&mut a, KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
        assert!(a.on_add_button(), "j reaches the button after the last item");
        handle_key(&mut a, KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
        assert!(!a.on_add_button(), "j past the button loops back to the top");
    }

    // Enter on the button adds a pillar.
    #[test]
    fn enter_on_button_adds_pillar() {
        let mut a = board_app();
        while !a.on_add_button() {
            handle_key(&mut a, KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
        }
        let before = ncols(&a);
        handle_key(&mut a, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(ncols(&a), before + 1);
    }

    // Header-nav aliases all reach/move headers: ] , Shift+L, Ctrl+Right.
    #[test]
    fn header_nav_aliases() {
        for ev in [
            KeyEvent::new(KeyCode::Char(']'), KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Char('L'), KeyModifiers::SHIFT),
            KeyEvent::new(KeyCode::Right, KeyModifiers::CONTROL),
            KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE),
        ] {
            let mut a = board_app();
            a.select_board_item(0, 0);
            handle_key(&mut a, ev);
            assert!(a.on_header(), "{ev:?} should reach a header");
        }
    }

    // Shift+K/J jump header→header (from items too), walking the button + rolling.
    // board_app: WHO[a] · JOBS[x] · button, one lane.
    #[test]
    fn shift_kj_vertical_header_jump() {
        let mut a = board_app();
        a.select_board_item(0, 0); // WHO item
        let j = |a: &mut AppState| handle_key(a, KeyEvent::new(KeyCode::Char('J'), KeyModifiers::SHIFT));
        // K from an item → that pillar's own header
        handle_key(&mut a, KeyEvent::new(KeyCode::Char('K'), KeyModifiers::SHIFT));
        assert!(a.on_header() && a.ui.cursor.col == 0, "K → own header");
        j(&mut a);
        assert!(a.on_header() && a.ui.cursor.col == 1, "J → JOBS header");
        j(&mut a);
        assert!(a.on_add_button(), "J → the add-pillar button");
        j(&mut a);
        assert!(a.on_header() && a.ui.cursor.col == 0, "J rolls over to WHO");
    }

    // From the header, j (and Esc) return to items.
    #[test]
    fn j_from_header_returns_to_items() {
        let mut a = board_app();
        a.select_board_item(0, 0);
        handle_key(&mut a, KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE)); // → header
        assert!(a.on_header());
        handle_key(&mut a, KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)); // → items
        assert!(!a.on_header(), "j leaves header to items");
    }
}
