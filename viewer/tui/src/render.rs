//! Ratatui rendering. Maps each fixed node kind to one widget. Layout nodes
//! split their area into equal slices.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use crate::theme;
use ratatui::widgets::{
    Block, BorderType, Borders, Cell, Gauge, Padding, Paragraph, Row, Scrollbar, ScrollbarOrientation,
    ScrollbarState, Table, Tabs, Wrap,
};
use ratatui::Frame;

use arte_core::protocol::{BoardColumn, BoardStyle, LayoutKind, UiNode};
use arte_core::state::{AppState, InspectorMode, Surface};

/// Shell: a tab bar of pages on top, the active page below.
pub fn render_app(frame: &mut Frame, app: &AppState) {
    let area = frame.area();
    let pages = app.pages();
    if pages.is_empty() {
        frame.render_widget(
            Paragraph::new("waiting for a page…")
                .block(Block::default().borders(Borders::ALL)),
            area,
        );
        return;
    }
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0), Constraint::Length(1)])
        .split(area);
    render_footer(frame, chunks[2], app); // persistent control guide on every page

    let titles: Vec<String> = pages
        .iter()
        .enumerate()
        .map(|(i, t)| format!(" {} {} ", i + 1, t))
        .collect();
    let tabs = Tabs::new(titles)
        .select(app.ui.active)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Tab pages · hjkl move · ? manual · q quit "),
        )
        .highlight_style(theme::selected());
    frame.render_widget(tabs, chunks[0]);

    app.ui.hits.borrow_mut().clear(); // rebuilt each frame by the active board
    // Working-set cells (titles → (col,item)) resolved in core; pulse renders here.
    let blink_items = app.working_cells();
    if let Some(surface) = app.active_surface() {
        if app.active_board_style() == BoardStyle::Panel {
            render_panel(frame, chunks[1], app, surface);
        } else {
            // Derive the old flat-index + header flag from the formal (col,row) cursor.
            let header_focus = app.header_col();
            let it = Interact {
                selected: app.selected_flat(),
                lanes: &app.ui.lanes,
                board_scroll: &app.ui.board_scroll,
                edit: match &app.ui.mode {
                    arte_core::state::Mode::Edit(e) => Some(EditView {
                        buffer: e.buffer.as_str(),
                        cursor: e.cursor,
                        select_all: e.select_all,
                    }),
                    _ => None,
                },
                add_focus: app.on_add_button(),
                blinking: &blink_items,
                blink: app.ui.blink.get(),
                header_focus,
                header_other: &app.ui.header_other,
                editing_prompt: app.ui.editing_prompt,
                hits: &app.ui.hits,
                audit: (app.active_board_style() == BoardStyle::Sheet).then(|| arte_core::agent::audit(app)),
                sheet_filter: app.ui.sheet_filter.clone(),
            };
            render_surface(frame, chunks[1], surface, &it);
        }
    }

    // Overlays are universal — rendered once, regardless of board style.
    if app.ui.help {
        render_help(frame);
    }
    maybe_render_inspector(frame, app);
    if let Some(b) = &app.ui.browse {
        render_browser(frame, b);
    }
    if app.ui.search.is_some() {
        render_search(frame, app);
    }
    if app.ui.glossary {
        render_glossary(frame, app);
    }
    if app.ui.confirm_quit {
        render_confirm_quit(frame);
    }
}

/// The inspector overlay — one place, called for any board style.
fn maybe_render_inspector(frame: &mut Frame, app: &AppState) {
    if !app.ui.inspect {
        return;
    }
    let Some(item) = app.selected_item() else { return };
    let picker = if app.ui.inspector == InspectorMode::LinkPick { Some((app.link_candidates(), app.ui.link_cursor)) } else { None };
    let ev = |on: bool| match (&on, &app.ui.mode) {
        (true, arte_core::state::Mode::Edit(e)) => Some(EditView { buffer: e.buffer.as_str(), cursor: e.cursor, select_all: e.select_all }),
        _ => None,
    };
    let cat_picker = if app.ui.inspector == InspectorMode::Category { Some((app.category_candidates(), app.ui.cat_pick)) } else { None };
    render_inspector(
        frame,
        item,
        picker,
        ev(app.ui.inspector == InspectorMode::Note),
        ev(app.ui.inspector == InspectorMode::Attach),
        ev(app.ui.inspector == InspectorMode::Category),
        cat_picker,
        ev(app.ui.inspector == InspectorMode::Detail),
        ev(app.ui.inspector == InspectorMode::Comment),
    );
}

/// Persistent control guide: the same common keys on every page + a right-aligned
/// slot for that page's one special action (the `g`/page-specific thing).
fn render_footer(frame: &mut Frame, area: Rect, app: &AppState) {
    use ratatui::text::{Line, Span};
    // vim-style prompt line: a pending delete shows here (bottom-left), every page.
    if app.ui.mode == arte_core::state::Mode::ConfirmDelete {
        let what = app.selected_item().map(|i| trunc(&i.text, 48)).unwrap_or_else(|| "this".into());
        let line = Line::from(vec![
            Span::styled(format!(" delete {what:?}? "), theme::alert()),
            Span::styled("y / n", theme::strong()),
        ]);
        frame.render_widget(Paragraph::new(line), area);
        return;
    }
    // Two-part guide: a MUTED static nav legend + a BRIGHT dynamic context hint
    // that changes with the focused component / mode (what you can do right now).
    let nav = " ↑↓←→ move · u undo · r redo · Tab page · ? help · q quit ";
    let ctx = context_hint(app);
    let used = nav.chars().count() + ctx.chars().count() + 2;
    let pad = (area.width as usize).saturating_sub(used);
    let line = Line::from(vec![
        Span::styled(nav, theme::hint()),
        Span::raw(" ".repeat(pad)),
        Span::styled(format!("{ctx} "), theme::header()),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

/// The bright, context-sensitive half of the footer: what the user can do given
/// the current overlay / edit mode / focused component.
fn context_hint(app: &AppState) -> &'static str {
    use arte_core::state::{InspectorMode, Mode};
    if app.ui.glossary {
        return "↑↓ pick · Enter filter · Esc close";
    }
    let editing = matches!(app.ui.mode, Mode::Edit(_));
    if app.ui.inspect {
        if editing {
            return match app.ui.inspector {
                InspectorMode::Note | InspectorMode::Attach => "Enter save · Shift+Enter add line · Esc cancel",
                InspectorMode::LinkPick => "Space toggle · Enter done",
                _ => "Enter save · Esc cancel",
            };
        }
        return "e detail · s status · l link · c cat · m comment · n note · a attach · d derived · Esc close";
    }
    if app.active_board_style() == BoardStyle::Panel {
        if editing {
            return match app.ui.inspector {
                InspectorMode::Detail => "Tab → method · Enter save · Esc cancel",
                InspectorMode::Category => "↑↓ pick · type to add · Tab next · Enter save",
                _ => "Enter save · Esc cancel",
            };
        }
        if !app.ui.panel_list_focus {
            return "type to filter · Esc back to controls";
        }
        if app.ui.panel_sel >= app.panel_results().len() {
            return "Enter add control";
        }
        return "Enter edit · Tab through fields · o inspect · n section";
    }
    if editing {
        return "Enter save · Esc cancel";
    }
    match app.active_board_style() {
        BoardStyle::Sheet => "Enter edit · o inspect · g filter by area",
        BoardStyle::Cards => "Enter edit · o inspect · n section",
        _ => "Enter edit · o inspect · n section",
    }
}

/// Glossary pop-up: trigram → description + count; doubles as the area-filter
/// picker (Enter applies the highlighted area as the sheet filter).
fn render_glossary(frame: &mut Frame, app: &AppState) {
    use ratatui::text::{Line, Span};
    use ratatui::widgets::Clear;
    let g = arte_core::agent::glossary(app);
    let area = frame.area();
    let w = 56u16.min(area.width.saturating_sub(4));
    let h = (g.len() as u16 + 4).clamp(5, area.height.saturating_sub(4));
    let rect = Rect { x: (area.width.saturating_sub(w)) / 2, y: (area.height.saturating_sub(h)) / 2, width: w, height: h };
    frame.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(" filter by area ")
        .border_style(theme::header());
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    let total: usize = g.iter().map(|(_, n)| n).sum();
    let mut lines: Vec<Line> = Vec::new();
    // row 0 = "all / clear filter"; rows 1.. = the area codes
    let sel0 = app.ui.glossary_sel == 0;
    lines.push(Line::from(Span::styled(
        format!(" all — clear filter   ({total})"),
        if sel0 { theme::selected() } else { theme::body() },
    )));
    for (i, (code, count)) in g.iter().enumerate() {
        let on = app.ui.glossary_sel == i + 1;
        let st = if on { theme::selected() } else { theme::body() };
        lines.push(Line::from(Span::styled(format!(" {code}   ({count})"), st)));
    }
    lines.push(Line::from(Span::styled(" ↑↓ select · Enter apply · Esc close", theme::hint())));
    frame.render_widget(Paragraph::new(lines), inner);
}

/// Small centered quit confirmation (q asks; y confirms).
fn render_confirm_quit(frame: &mut Frame) {
    use ratatui::text::{Line, Span};
    use ratatui::widgets::Clear;
    let area = frame.area();
    let w = 28u16.min(area.width);
    let rect = Rect { x: (area.width.saturating_sub(w)) / 2, y: area.height / 2, width: w, height: 3 };
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(" quit?  y / n", theme::strong())))
            .block(Block::default().borders(Borders::ALL).border_style(theme::alert())),
        rect,
    );
}

/// Registry panel (BoardStyle::Panel): category sidebar (coarse) + filtered list
/// + a permanent fine-grain filter box. Used by control.spec.
fn render_panel(frame: &mut Frame, area: Rect, app: &AppState, surface: &Surface) {
    use ratatui::text::{Line, Span};
    // Outer main block titled with the page (like every other style); the sidebar,
    // table, and filter live INSIDE it — not three floating boxes.
    let outer = Block::default().borders(Borders::ALL).title(format!(" {} ", surface.title));
    let pa = outer.inner(area);
    frame.render_widget(outer, area);
    let show_cats = !app.ui.panel_hide_cats && pa.width > 34;

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(3)])
        .split(pa);
    let (cats_area, spec_area) = if show_cats {
        let top = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(20), Constraint::Min(0)])
            .split(rows[0]);
        (Some(top[0]), top[1])
    } else {
        (None, rows[0])
    };

    let results = app.panel_results();
    let board = app.active_board().unwrap_or(&[]);

    if let Some(ca) = cats_area {
        render_panel_components(frame, ca, app);
    }
    render_panel_table(frame, spec_area, app, &results, board);

    // search block — query + active component + counts + key hints
    let cur = if app.ui.panel_list_focus { "" } else { "\u{2588}" };
    let comps = app.panel_components();
    let active = if app.ui.panel_cat == 0 {
        "all".to_string()
    } else {
        comps.get(app.ui.panel_cat - 1).map(|(c, _)| c.clone()).unwrap_or_default()
    };
    let filter = Line::from(vec![
        Span::styled("  ", theme::header()),
        Span::styled(format!("{}{cur}", app.ui.panel_query), theme::strong()),
        Span::styled(format!("    component:{active}    {} match", results.len()), theme::hint()),
    ]);
    // (key hints live in the global footer — no redundant in-box guide)
    let sbs = if app.ui.panel_list_focus { theme::hint() } else { theme::header() };
    let title = if app.ui.panel_list_focus { " search (/ to type) " } else { " search ▍ " };
    frame.render_widget(
        Paragraph::new(vec![filter]).block(Block::default().borders(Borders::ALL).title(title).border_style(sbs)),
        rows[1],
    );
}

fn render_panel_components(frame: &mut Frame, area: Rect, app: &AppState) {
    use ratatui::text::{Line, Span};
    let comps = app.panel_components();
    let total: usize = comps.iter().map(|(_, n)| n).sum();
    let mut lines = Vec::new();
    let mut push = |i: usize, label: String| {
        let on = i == app.ui.panel_cat;
        let st = if on { theme::selected() } else { theme::body() };
        lines.push(Line::from(Span::styled(format!("{} {label}", if on { "▸" } else { " " }), st)));
    };
    push(0, format!("all ({total})"));
    for (i, (c, n)) in comps.iter().enumerate() {
        push(i + 1, format!("{c} ({n})"));
    }
    frame.render_widget(
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(" components ").border_style(theme::hint())),
        area,
    );
}

/// The filtered controls as a columned table (status · name · category · linked · age)
/// with a header row and a scrollbar — yozefu's records-table look.
fn render_panel_table(
    frame: &mut Frame,
    area: Rect,
    app: &AppState,
    results: &[(usize, usize)],
    board: &[BoardColumn],
) {
    use arte_core::protocol::Status;
    // bright border when the list has focus, dim otherwise
    let bs = if app.ui.panel_list_focus { theme::header() } else { theme::hint() };
    let block = Block::default().borders(Borders::ALL).border_style(bs).title(format!(" controls ({}) ", results.len()));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height < 2 {
        return;
    }
    let view = inner.height.saturating_sub(1) as usize; // minus header row
    let start = if app.ui.panel_sel >= view { app.ui.panel_sel + 1 - view } else { 0 };
    let header = Row::new(["", "component", "subset", "details", "modified"]).style(theme::header());
    let working = app.working_cells(); // same blink set as list/cards
    let blink = app.ui.blink.get();
    // quick-edit buffer (inline detail/method edit, no inspector overlay)
    let qbuf = match &app.ui.mode {
        arte_core::state::Mode::Edit(e) if !app.ui.inspect => Some(e.buffer.clone()),
        _ => None,
    };
    let mut rows = Vec::new();
    for (i, (ci, ii)) in results.iter().enumerate().skip(start).take(view) {
        let Some(it) = board.get(*ci).and_then(|c| c.items.get(*ii)) else { continue };
        let (glyph, gstyle) = match it.status {
            Some(Status::Ok) => ("✓", theme::ok()),
            Some(Status::Fail) => ("✗", theme::fail()),
            Some(Status::Pending) => ("◷", theme::pending()),
            Some(Status::Justified) => ("⊘", theme::hint()),
            None => ("·", theme::hint()),
        };
        // columns: component (subject) · subset (browse axis) · details (text)
        let component = trunc(&it.serves.join(", "), 22);
        let mut marks = String::new();
        if !it.note.is_empty() {
            marks.push_str(" *");
        }
        if !it.attachments.is_empty() {
            marks.push_str(" @");
        }
        let mut detail = format!("{}{marks}", it.text);
        // quick-edit: show the live edit buffer in the field being edited.
        let editing_here = qbuf.is_some() && i == app.ui.panel_sel;
        if editing_here && app.ui.inspector == InspectorMode::Detail {
            detail = format!("{}█", qbuf.clone().unwrap_or_default());
        }
        let age = it.modified.map(arte_core::agent::rel_age).unwrap_or_default();
        // subset = the board column this control lives in (the browse/sort axis)
        let subset = board.get(*ci).and_then(|c| c.key.clone()).unwrap_or_default();
        let cells = vec![
            Cell::from(glyph).style(gstyle),
            Cell::from(component),
            Cell::from(subset).style(theme::hint()),
            Cell::from(detail),
            Cell::from(age).style(theme::hint()), // metadata recedes
        ];
        let mut style = if i == app.ui.panel_sel { theme::selected() } else { ratatui::style::Style::default() };
        if blink && working.contains(&(*ci, *ii)) {
            style = style.patch(theme::working());
        }
        rows.push(Row::new(cells).style(style));
    }
    if results.is_empty() {
        rows.push(Row::new(["", "(no matches)", "", "", ""]).style(theme::hint()));
    }
    // trailing "＋ add control" button row (↓ to it + Enter, or press n)
    let add_idx = results.len();
    if add_idx >= start && add_idx < start + view {
        let on = app.ui.panel_sel == add_idx;
        let st = if on { theme::selected() } else { theme::hint() };
        rows.push(Row::new(vec![Cell::from(""), Cell::from("+ add control"), Cell::from(""), Cell::from(""), Cell::from("")]).style(st));
    }
    let widths = [
        Constraint::Length(1),
        Constraint::Length(22),
        Constraint::Length(12),
        Constraint::Min(18),
        Constraint::Length(10),
    ];
    frame.render_widget(Table::new(rows, widths).header(header).column_spacing(1), inner);

    // scrollbar when the list (incl. the add row) overflows
    vscrollbar(frame, area.inner(ratatui::layout::Margin { vertical: 1, horizontal: 0 }), results.len() + 1, view, app.ui.panel_sel);
}

/// Search overlay: a query bar + the live-filtered results across the board.
fn render_search(frame: &mut Frame, app: &AppState) {
    use arte_core::protocol::Status;
    use ratatui::text::{Line, Span};
    use ratatui::widgets::Clear;
    let Some(s) = &app.ui.search else { return };
    let cols = app.active_board().unwrap_or(&[]);
    let results = app.search_results();
    let area = frame.area();
    let w = 76u16.min(area.width);
    let view = (area.height.saturating_sub(7)).clamp(3, 22) as usize;
    let start = if s.cursor >= view { s.cursor - view + 1 } else { 0 };

    let mut lines = vec![
        Line::from(vec![Span::styled("  / ", theme::header()), Span::styled(format!("{}\u{2588}", s.query), theme::strong())]),
        Line::from(Span::styled(format!("  {} match(es)", results.len()), theme::hint())),
    ];
    for (i, (ci, ii)) in results.iter().enumerate().skip(start).take(view) {
        let Some(col) = cols.get(*ci) else { continue };
        let Some(it) = col.items.get(*ii) else { continue };
        let cat = it.category.as_deref().map(|c| format!(" ·{c}")).unwrap_or_default();
        let st = match it.status {
            Some(Status::Ok) => " [ok]",
            Some(Status::Fail) => " [ko]",
            Some(Status::Pending) => " [pending]",
            Some(Status::Justified) => " [justified]",
            None => "",
        };
        let sect = trunc(col.header.split(" — ").next().unwrap_or(""), 10);
        let row = format!("  {sect:<10} {}{cat}{st}", trunc(&it.text, 34));
        let style = if i == s.cursor { theme::selected() } else { theme::body() };
        lines.push(Line::from(Span::styled(row, style)));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  filters: cat: status: link: intent: since:7d · ↑↓ select · Enter go · Esc",
        theme::hint(),
    )));

    let h = (lines.len() as u16 + 2).min(area.height);
    let rect = Rect { x: (area.width.saturating_sub(w)) / 2, y: (area.height.saturating_sub(h)) / 2, width: w, height: h };
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(" Search ")),
        rect,
    );
}

/// File-browser overlay (cross-platform): the current dir + a scrolling list.
fn render_browser(frame: &mut Frame, b: &arte_core::state::Browse) {
    use ratatui::text::{Line, Span};
    use ratatui::widgets::Clear;
    let area = frame.area();
    let w = 72u16.min(area.width);
    let view = (area.height.saturating_sub(7)).clamp(3, 24) as usize; // list rows shown
    let start = if b.cursor >= view { b.cursor - view + 1 } else { 0 };
    let mut lines = vec![Line::from(Span::styled(format!(" {}", b.dir), theme::hint())), Line::from("")];
    for (i, (name, is_dir)) in b.entries.iter().enumerate().skip(start).take(view) {
        let label = if *is_dir { format!("{name}/") } else { name.clone() };
        let style = if i == b.cursor {
            theme::selected()
        } else if *is_dir {
            theme::header()
        } else {
            theme::body()
        };
        lines.push(Line::from(Span::styled(format!("  {label}"), style)));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("  j/k move · Enter open dir / pick file · Esc cancel", theme::hint())));

    let h = (lines.len() as u16 + 2).min(area.height);
    let rect = Rect {
        x: (area.width.saturating_sub(w)) / 2,
        y: (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    };
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(" Attach file ")),
        rect,
    );
}

/// Centered keymap overlay (toggled with `?`).
fn render_help(frame: &mut Frame) {
    use ratatui::text::{Line, Span};
    use ratatui::widgets::Clear;
    let rows: &[(&str, &str)] = &[
        ("Pages", "Tab / 1-4 switch · q quit"),
        ("Items", "h j k l  move (loops around)"),
        ("", "Enter add/edit · Bksp delete · < > reorder"),
        ("", "u undo · r redo · / search · o inspect"),
        ("Headers", "t go to header · [ ] across · j/Esc back"),
        ("", "(also Shift+HJKL / Ctrl+arrows) · Enter rename · Tab name↔prompt"),
        ("", "n new column · Bksp delete column"),
        ("Agent", "observe / act over .surface.jsonl"),
    ];
    let mut lines = vec![Line::from("")];
    for (k, v) in rows {
        lines.push(Line::from(vec![
            Span::styled(format!("  {k:<9}"), theme::header()),
            Span::raw(format!("{v}  ")),
        ]));
    }
    lines.push(Line::from(""));

    let area = frame.area();
    let w = 58u16.min(area.width);
    let h = (lines.len() as u16 + 2).min(area.height);
    let rect = Rect {
        x: (area.width.saturating_sub(w)) / 2,
        y: (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    };
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Manual — ? or any key to close "),
        ),
        rect,
    );
}

/// Interaction state passed to nodes that support selection/editing.
pub struct Interact<'a> {
    pub selected: Option<usize>,
    /// Renderer writes the lane count here so navigation can follow the columns.
    pub lanes: &'a std::cell::Cell<usize>,
    pub board_scroll: &'a std::cell::Cell<usize>,
    pub edit: Option<EditView<'a>>,
    /// Cursor is on the "+ add pillar" button.
    pub add_focus: bool,
    /// Items (col, item) the agent is working on — pulse them.
    pub blinking: &'a std::collections::HashSet<(usize, usize)>,
    /// Blink phase (true = highlighted half of the pulse).
    pub blink: bool,
    /// Column whose header is focused (Ctrl+←/→), if any.
    pub header_focus: Option<usize>,
    /// The inactive half of a header being edited, and which half is active.
    pub header_other: &'a str,
    pub editing_prompt: bool,
    /// Renderer records clickable regions here for the mouse handler.
    pub hits: &'a std::cell::RefCell<Vec<arte_core::state::HitRegion>>,
    /// Cross-layer audit (computed for the sheet page) — surfaced in its stats block.
    pub audit: Option<arte_core::agent::Audit>,
    /// Active functional-area filter on the sheet (the trigram), if any.
    pub sheet_filter: Option<String>,
}

/// View of the inline editor for the selected slot.
pub struct EditView<'a> {
    pub buffer: &'a str,
    pub cursor: usize,
    pub select_all: bool,
}

/// The buffer rendered with select-all highlight or a block cursor (no prefix).
fn edit_spans(ev: &EditView) -> Vec<ratatui::text::Span<'static>> {
    use ratatui::text::Span;
    if ev.select_all {
        // a block cursor for the empty case keeps the line from being pure
        // whitespace (which Wrap{trim:false} mis-wraps into a phantom 2nd row).
        if ev.buffer.is_empty() {
            return vec![Span::styled("█".to_string(), theme::hint())];
        }
        return vec![Span::styled(ev.buffer.to_string(), theme::selected())];
    }
    let chars: Vec<char> = ev.buffer.chars().collect();
    let cur = ev.cursor.min(chars.len());
    let before: String = chars[..cur].iter().collect();
    let after: String = chars[cur.min(chars.len())..].iter().skip(1).collect();
    // cursor: reverse the char under it; at end-of-buffer use a block glyph (not a
    // space) so an empty edit line isn't all-whitespace and won't mis-wrap.
    let at = match chars.get(cur) {
        Some(c) => Span::styled(c.to_string(), theme::selected()),
        None => Span::styled("█".to_string(), theme::hint()),
    };
    vec![Span::raw(before), at, Span::raw(after)]
}


/// One edit line for an item: prefix + the edited buffer.
fn edit_line(prefix: &str, ev: &EditView) -> ratatui::text::Line<'static> {
    use ratatui::text::{Line, Span};
    let mut spans = vec![Span::raw(prefix.to_string())];
    spans.extend(edit_spans(ev));
    Line::from(spans)
}

pub fn render_surface(frame: &mut Frame, area: Rect, surface: &Surface, it: &Interact) {
    // Outer block carries the surface title; root node renders inside.
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {} ", surface.title));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    render_node(frame, inner, &surface.root, it);
}

fn render_node(frame: &mut Frame, area: Rect, node: &UiNode, it: &Interact) {
    match node {
        UiNode::Panel { title, layout, children, .. } => {
            let block = Block::default().borders(Borders::ALL);
            let block = match title {
                Some(t) => block.title(format!(" {t} ")),
                None => block,
            };
            let inner = block.inner(area);
            frame.render_widget(block, area);
            render_children(frame, inner, *layout, children, it);
        }
        UiNode::Text { text, .. } => {
            frame.render_widget(Paragraph::new(text.as_str()).wrap(Wrap { trim: false }), area);
        }
        UiNode::Metric { label, value, .. } => {
            let line = ratatui::text::Line::from(vec![
                ratatui::text::Span::raw(format!("{label}: ")),
                ratatui::text::Span::styled(value.clone(), theme::strong()),
            ]);
            frame.render_widget(Paragraph::new(line), area);
        }
        UiNode::Table { columns, rows, .. } => {
            let header = Row::new(
                columns
                    .iter()
                    .map(|c| Cell::from(c.clone()))
                    .collect::<Vec<_>>(),
            )
            .style(theme::strong());
            let widths = vec![Constraint::Ratio(1, columns.len().max(1) as u32); columns.len()];
            let body = rows
                .iter()
                .map(|r| Row::new(r.iter().map(|c| Cell::from(c.clone())).collect::<Vec<_>>()))
                .collect::<Vec<_>>();
            frame.render_widget(Table::new(body, widths).header(header), area);
        }
        UiNode::Log { lines, .. } => {
            // Show the most recent lines that fit; newest at the bottom.
            let take = area.height as usize;
            let start = lines.len().saturating_sub(take);
            let text = lines[start..].join("\n");
            frame.render_widget(Paragraph::new(text), area);
        }
        UiNode::Progress { label, value, .. } => {
            frame.render_widget(
                Gauge::default()
                    .ratio(value.clamp(0.0, 1.0))
                    .label(format!("{label} {:.0}%", value * 100.0)),
                area,
            );
        }
        UiNode::Board { columns, style, .. } => {
            render_board(frame, area, columns, *style, it);
        }
    }
}

/// Width-aware board laid out as an aligned grid: pillars fill `k` lanes
/// row-by-row, and every pillar in the same grid-row starts on the same line
/// (the row is as tall as its tallest pillar). The last cell is the add-pillar
/// button. Footer shows the selected item, the add buffer, or a delete prompt.
/// The item's marker glyph + style for this frame, from one presentation state:
/// `highlighted` (working set, blink on-phase) → arrow; otherwise the status
/// glyph or a plain bullet. Marker and style flip together — the single source
/// of the bullet↔arrow swap, so cursor-on-top behaves the same.
/// Shorten a path/URL for display: `$HOME`→`~`, then left-ellipsis to keep the
/// filename (the useful end) when it's still too long.
fn short_path(p: &str, max: usize) -> String {
    let p = match std::env::var("HOME") {
        Ok(h) if !h.is_empty() && p.starts_with(&h) => format!("~{}", &p[h.len()..]),
        _ => p.to_string(),
    };
    let n = p.chars().count();
    if n <= max {
        return p;
    }
    let tail: String = p.chars().skip(n - max.saturating_sub(1)).collect();
    format!("…{tail}")
}

/// Compact has-detail marks for a board element: `*` note, `@` attachment.
fn detail_marks(item: &arte_core::protocol::Item) -> String {
    let mut s = String::new();
    if !item.note.is_empty() {
        s.push('*');
    }
    if !item.attachments.is_empty() {
        s.push('@');
    }
    s
}

/// Inspector overlay: the selected element's links, note, and attachments — the
/// human-oversight view of what an agent attached. Read-only; edits go via the agent.
fn render_inspector(
    frame: &mut Frame,
    item: &arte_core::protocol::Item,
    picker: Option<(Vec<String>, usize)>,
    note_edit: Option<EditView>,
    attach_edit: Option<EditView>,
    cat_edit: Option<EditView>,
    cat_picker: Option<(Vec<String>, usize)>,
    detail_edit: Option<EditView>,
    comment_edit: Option<EditView>,
) {
    use ratatui::text::{Line, Span};
    use ratatui::widgets::Clear;
    let area = frame.area();
    let w = 64u16.min(area.width);
    let avail = (w as usize).saturating_sub(2 + 2 + 14 + 2); // borders + right pad + col + "@ "
    // title line = the item's text ("details"); becomes an editor when editing it
    let title_line = if let Some(ev) = &detail_edit {
        let mut spans = vec![Span::raw("  ".to_string())];
        spans.extend(edit_spans(ev));
        Line::from(spans)
    } else {
        Line::from(Span::styled(format!("  {}", item.text), theme::strong()))
    };
    let mut lines = vec![title_line, Line::from("")];
    let field = |k: &str, v: String| {
        Line::from(vec![Span::styled(format!("  {k:<12}"), theme::header()), Span::raw(v)])
    };
    let dash = "—".to_string();
    if let Some((cands, cur)) = &picker {
        // pick an existing parent title to link to: [x] = already linked, ▸ = cursor
        lines.push(Line::from(Span::styled("  linked to — pick:", theme::header())));
        for (i, t) in cands.iter().enumerate() {
            let on = item.serves.iter().any(|s| s == t);
            let row = format!("  {} [{}] {t}", if i == *cur { "▸" } else { " " }, if on { "x" } else { " " });
            lines.push(Line::from(Span::styled(row, if i == *cur { theme::selected() } else { theme::body() })));
        }
        if cands.is_empty() {
            lines.push(Line::from(Span::styled("    (no parent layer)", theme::hint())));
        }
    } else {
        let linked = if item.serves.is_empty() { dash.clone() } else { item.serves.join(", ") };
        lines.push(field("linked to", linked));
        // category — combobox when editing: field + a list of existing values to pick
        if let Some(ev) = &cat_edit {
            let mut spans = vec![Span::styled(format!("  {:<12}", "category"), theme::header())];
            spans.extend(edit_spans(ev));
            lines.push(Line::from(spans));
            if let Some((cands, pick)) = &cat_picker {
                for (i, c) in cands.iter().enumerate() {
                    let on = i == *pick;
                    let row = format!("    {} {c}", if on { "▸" } else { " " });
                    lines.push(Line::from(Span::styled(row, if on { theme::selected() } else { theme::body() })));
                }
                lines.push(Line::from(Span::styled("    ↑↓ pick · type a new one", theme::hint())));
            }
        } else {
            lines.push(field("category", item.category.clone().unwrap_or_else(|| dash.clone())));
        }
        lines.push(field("status", status_label(item.status).into()));
        // comment (result remark) — editable
        if let Some(ev) = &comment_edit {
            let mut spans = vec![Span::styled(format!("  {:<12}", "comment"), theme::header())];
            spans.extend(edit_spans(ev));
            lines.push(Line::from(spans));
        } else {
            lines.push(field("comment", item.comment.clone().unwrap_or_else(|| dash.clone())));
        }
        if let Some(s) = &item.sha {
            lines.push(field("sha", s.chars().take(arte_core::SHA_DISPLAY_LEN).collect()));
        }
        if item.derived {
            lines.push(field("derived", "yes (parentless by design)".into()));
        }
        if let Some(t) = item.modified {
            lines.push(field("modified", arte_core::agent::rel_age(t)));
        }
        // note + attachments are lists: first value on the field row, continuations
        // aligned under the value column (col 13 = "  " + {:<11}). Borderless grid.
        let vindent = " ".repeat(14);
        // note
        if item.note.is_empty() && note_edit.is_none() {
            lines.push(field("note", dash.clone()));
        } else {
            for (i, n) in item.note.iter().enumerate() {
                if i == 0 {
                    lines.push(field("note", n.clone()));
                } else {
                    lines.push(Line::from(Span::raw(format!("{vindent}{n}"))));
                }
            }
            if let Some(ev) = &note_edit {
                let mut spans = if item.note.is_empty() {
                    vec![Span::styled(format!("  {:<12}", "note"), theme::header())]
                } else {
                    vec![Span::raw(vindent.clone())]
                };
                spans.extend(edit_spans(ev));
                lines.push(Line::from(spans));
            }
        }
        // attachments (each value prefixed "@ ")
        if item.attachments.is_empty() && attach_edit.is_none() {
            lines.push(field("attachments", dash));
        } else {
            for (i, a) in item.attachments.iter().enumerate() {
                let v = format!("@ {}", short_path(a, avail));
                if i == 0 {
                    lines.push(field("attachments", v));
                } else {
                    lines.push(Line::from(Span::raw(format!("{vindent}{v}"))));
                }
            }
            if let Some(ev) = &attach_edit {
                let mut spans = if item.attachments.is_empty() {
                    vec![Span::styled(format!("  {:<12}", "attachments"), theme::header()), Span::raw("@ ".to_string())]
                } else {
                    vec![Span::raw(format!("{vindent}@ "))]
                };
                spans.extend(edit_spans(ev));
                lines.push(Line::from(spans));
            }
        }
    }
    lines.push(Line::from(""));
    let hint = if picker.is_some() {
        "  j/k move · Space toggle · Enter/Esc done"
    } else if note_edit.is_some() || attach_edit.is_some() {
        "  type · Enter save · Esc cancel"
    } else {
        "  e detail · s status · l link · c cat · m comment · n note · a/b attach · d derived · o/Esc close"
    };
    lines.push(Line::from(Span::styled(hint, theme::hint())));

    // Size to the WRAPPED row count (a long details/note line wraps to several
    // visual rows) so nothing below it gets clipped out of the box.
    let inner_w = (w as usize).saturating_sub(4).max(1); // borders + right pad
    let content_rows: usize = lines.iter().map(|l| l.width().max(1).div_ceil(inner_w)).sum();
    let h = (content_rows as u16 + 2).min(area.height);
    // Anchor the top near a fixed spot so adding note/attachment lines grows the
    // box DOWNWARD instead of recentering (which makes it jiggle while typing).
    let y = (area.height / 2).saturating_sub(8).min(area.height.saturating_sub(h));
    let rect = Rect {
        x: (area.width.saturating_sub(w)) / 2,
        y,
        width: w,
        height: h,
    };
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            // right padding == the 2-space left indent → equal L/R margins
            .block(Block::default().borders(Borders::ALL).title(" Inspector ").padding(Padding::new(0, 2, 0, 0))),
        rect,
    );
}

fn item_marker(item: &arte_core::protocol::Item, highlighted: bool) -> (&'static str, ratatui::style::Style) {
    use arte_core::protocol::Status;
    if highlighted {
        return ("⇒ ", theme::working());
    }
    match item.status {
        Some(Status::Ok) => ("✓ ", theme::ok()),
        Some(Status::Fail) => ("✗ ", theme::fail()),
        Some(Status::Pending) => ("◷ ", theme::pending()),
        Some(Status::Justified) => ("⊘ ", theme::hint()),
        None => ("• ", theme::hint()),
    }
}

/// " (serves: a, b)" for the footer when an item is linked, else empty.
fn serves_hint(item: &arte_core::protocol::Item) -> String {
    if item.serves.is_empty() {
        String::new()
    } else {
        format!("  (serves: {})", item.serves.join(", "))
    }
}

fn render_board(frame: &mut Frame, area: Rect, columns: &[BoardColumn], style: BoardStyle, it: &Interact) {
    if area.width == 0 {
        return;
    }
    let (body, footer) = {
        let c = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(0), Constraint::Length(1)])
            .split(area);
        (c[1], c[2])
    };
    let mut hits = it.hits.borrow_mut();
    let mut detail = None;
    // One renderer per board style — pick the component that fits the data.
    match style {
        BoardStyle::List => render_list_board(frame, body, columns, it, &mut hits, &mut detail),
        BoardStyle::Cards => render_cards_board(frame, body, columns, it, &mut hits, &mut detail),
        BoardStyle::Sheet => render_sheet_board(frame, body, columns, it, &mut hits, &mut detail),
        // render_app routes a Panel-styled page to render_panel before this point.
        BoardStyle::Panel => {} // routed at the page level by render_panel; nothing to draw here
    }
    drop(hits);
    frame.render_widget(Paragraph::new(board_footer(it, detail)), footer);
}

/// `list` board: pillars as a masonry grid of flat columns (intent.map).
/// Excel-like grid (BoardStyle::Sheet): a row per item — id · name · status ·
/// comment — and the selected row expands to its description (note) + links
/// (attachments). Used by validation.rep. Single grid; reuses the board cursor.
/// Vertical scrollbar on `area`, drawn only when content overflows the view —
/// the one place every board's "there's more" affordance lives.
fn vscrollbar(frame: &mut Frame, area: Rect, total: usize, view: usize, pos: usize) {
    if total <= view {
        return;
    }
    let mut sb = ScrollbarState::new(total).position(pos);
    frame.render_stateful_widget(
        Scrollbar::new(ScrollbarOrientation::VerticalRight).begin_symbol(None).end_symbol(None),
        area,
        &mut sb,
    );
}

/// Keep the board scroll STABLE: only move it when `focus` would fall outside the
/// window `heights[scroll..]` that fits `body_h`. So moving onto an already-visible
/// row/section (or a pinned button) doesn't jump the view.
fn stable_scroll(scroll: usize, focus: usize, heights: &[u16], body_h: u16) -> usize {
    let mut s = scroll.min(heights.len().saturating_sub(1));
    if focus < s {
        return focus; // cursor above the window → scroll up to it
    }
    while s < focus {
        let used: u16 = heights[s..=focus].iter().sum();
        if used <= body_h {
            break; // focus already fits from s → leave the view put
        }
        s += 1; // cursor below the window → advance the minimum
    }
    s
}

/// Single source of truth for status display (UI labels, not the wire words).
fn status_label(st: Option<arte_core::protocol::Status>) -> &'static str {
    use arte_core::protocol::Status;
    match st {
        Some(Status::Ok) => "OK",
        Some(Status::Fail) => "KO",
        Some(Status::Pending) => "Pending",
        Some(Status::Justified) => "Justified",
        None => "—",
    }
}
fn status_style(st: Option<arte_core::protocol::Status>) -> ratatui::style::Style {
    use arte_core::protocol::Status;
    match st {
        Some(Status::Ok) => theme::ok(),
        Some(Status::Fail) => theme::fail(),
        Some(Status::Pending) => theme::pending(),
        _ => theme::hint(), // Justified / none
    }
}

fn render_sheet_board(
    frame: &mut Frame,
    body: Rect,
    columns: &[BoardColumn],
    it: &Interact,
    hits: &mut Vec<arte_core::state::HitRegion>,
    detail: &mut Option<String>,
) {
    use ratatui::text::{Line, Span};
    it.lanes.set(1); // one grid, not lanes

    // --- coverage dashboard: high-level intent/impl/control/test stats (x/total %)
    let mut sl: Vec<Line> = Vec::new();
    if let Some(au) = &it.audit {
        let t = au.total;
        let pct = |n: usize| if t == 0 { 0 } else { n * 100 / t };
        let ctrl = au.layers.get(1).map(|(_, n)| *n).unwrap_or(0); // control = derived reqs
        sl.push(Line::from(vec![
            Span::styled(format!(" {t} intents"), theme::strong()),
            Span::styled(format!("   impl {}/{t} ({}%)", au.implemented, pct(au.implemented)), theme::hint()),
            Span::styled(format!("   control {ctrl}/{t} ({}%)", pct(ctrl)), theme::hint()),
            Span::styled(format!("   tests {}/{t} ({}%)", au.verified, pct(au.verified)), theme::hint()),
        ]));
        sl.push(Line::from(vec![
            Span::styled(
                format!(" covered {}/{t} ({}%)", au.covered, pct(au.covered)),
                if t > 0 && au.covered == t { theme::ok() } else { theme::strong() },
            ),
            Span::styled(format!("   validated {}/{t} ({}%)", au.validated, pct(au.validated)), if t > 0 && au.validated == t { theme::ok() } else { theme::hint() }),
        ]));
        sl.push(Line::from(vec![
            Span::styled(format!(" uncovered {}", au.gaps.len()), if au.gaps.is_empty() { theme::hint() } else { theme::pending() }),
            Span::styled(format!("   orphaned {}", au.orphans.len()), if au.orphans.is_empty() { theme::hint() } else { theme::fail() }),
            Span::styled(format!("   derived {}", au.derived), theme::hint()),
            Span::styled(format!("   justified {}", au.justified), theme::hint()),
        ]));
    }
    if let Some(f) = &it.sheet_filter {
        sl.push(Line::from(Span::styled(format!(" ▸ filter: area {f}   g change · Esc clear"), theme::header())));
    }
    let split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(sl.len() as u16 + 2), Constraint::Min(0)])
        .split(body);
    let (cov_area, body) = (split[0], split[1]);
    frame.render_widget(
        Paragraph::new(sl).block(Block::default().borders(Borders::ALL).title(" coverage ").border_style(theme::header())),
        cov_area,
    );

    // test cases live in a muted block below the coverage dashboard
    let gsplit = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(3)])
        .split(body);
    let (cases_outer, btn_area) = (gsplit[0], gsplit[1]);
    let cases_block = Block::default().borders(Borders::ALL).title(" test cases ").border_style(theme::hint());
    let rows_area = cases_block.inner(cases_outer);
    frame.render_widget(cases_block, cases_outer);

    let w = rows_area.width as usize;
    let sep = " │ ";
    // column widths: id + status fixed, name/comment share the rest.
    let id_w = 12usize.min(w / 4);
    let st_w = 10usize;
    let sha_w = 9usize; // short commit sha
    let rest = w.saturating_sub(id_w + st_w + sha_w + sep.len() * 4);
    let name_w = (rest * 6 / 10).max(8);
    let cmt_w = rest.saturating_sub(name_w);
    let pad = |s: &str, wd: usize| {
        let t = trunc(s, wd);
        let gap = wd.saturating_sub(t.chars().count());
        format!("{t}{}", " ".repeat(gap))
    };
    let center = |s: &str, wd: usize| {
        let n = s.chars().count().min(wd);
        let l = (wd - n) / 2;
        format!("{}{}{}", " ".repeat(l), trunc(s, wd), " ".repeat(wd - n - l))
    };

    // flatten items to a single ordered list (sheet = one grid) + the selected
    // position — the windowing key, so this scales to hundreds of cases.
    let mut flat: Vec<(usize, usize)> = Vec::new();
    let mut sel_pos: Option<usize> = None;
    let mut base = 0usize;
    let area = it.sheet_filter.as_deref();
    for (ci, col) in columns.iter().enumerate() {
        for (ii, item) in col.items.iter().enumerate() {
            let show = area.map(|a| item.id.as_deref().and_then(arte_core::agent::area_of) == Some(a)).unwrap_or(true);
            if !show {
                continue;
            }
            if it.selected == Some(base + ii) {
                sel_pos = Some(flat.len());
            }
            flat.push((ci, ii));
        }
        base += col.items.len() + 1;
    }
    let total = flat.len();


    let header = Line::from(Span::styled(
        format!("{}{sep}{}{sep}{}{sep}{}{sep}{}", pad("ID", id_w), pad("NAME", name_w), center("STATUS", st_w), pad("SHA", sha_w), pad("COMMENT", cmt_w)),
        theme::strong(),
    ));
    let rule = Line::from(Span::styled("─".repeat(rows_area.width as usize), theme::hint()));
    // the selected row expands (note + attachments) — budget those out of the view
    let exp = sel_pos
        .map(|p| {
            let (ci, ii) = flat[p];
            let s = &columns[ci].items[ii];
            s.note.len() + s.attachments.len()
        })
        .unwrap_or(0);
    let view = (rows_area.height as usize).saturating_sub(2 + exp).max(1); // minus header + rule + expansion
    let start = match sel_pos {
        Some(p) if p >= view => p + 1 - view,
        _ => 0,
    };

    let mut lines: Vec<Line> = vec![header, rule];
    for p in start..(start + view).min(total) {
        let (ci, ii) = flat[p];
        let item = &columns[ci].items[ii];
        let selected = sel_pos == Some(p);
        let working = it.blink && it.blinking.contains(&(ci, ii)); // working_cells is 0-based, like list/cards
        let mut style = if selected { theme::selected() } else { theme::body() };
        if working {
            style = style.patch(theme::working());
        }
        let id = item.id.clone().unwrap_or_else(|| "—".into());
        let cmt = item.comment.clone().unwrap_or_default();
        hits.push(arte_core::state::HitRegion {
            x: rows_area.x,
            y: rows_area.y + lines.len() as u16,
            w: rows_area.width,
            hit: arte_core::state::Hit::Cell { col: ci, row: ii + 1 },
        });
        let sha = item.sha.as_deref().map(|s| s.chars().take(arte_core::SHA_DISPLAY_LEN).collect::<String>()).unwrap_or_else(|| "—".into());
        // status = a colour-filled chip (whole cell), so it reads at a glance
        let chip = match item.status {
            // colour-filled chip for a real status; "—" follows the ROW style so it
            // highlights with the cursor/working pulse like the other cells.
            Some(_) => Span::styled(center(status_label(item.status), st_w), status_style(item.status).add_modifier(ratatui::style::Modifier::REVERSED)),
            None => Span::styled(center("—", st_w), style),
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{}{sep}{}{sep}", pad(&id, id_w), pad(&item.text, name_w)), style),
            chip,
            Span::styled(format!("{sep}{}{sep}{}", pad(&sha, sha_w), pad(&cmt, cmt_w)), style),
        ]));
        if selected {
            for n in &item.note {
                lines.push(Line::from(Span::styled(format!("        ↳ {n}"), theme::hint())));
            }
            for a in &item.attachments {
                lines.push(Line::from(Span::styled(format!("        ↳ @ {a}"), theme::body())));
            }
            if !item.serves.is_empty() {
                *detail = Some(format!("tests: {}", item.serves.join(", ")));
            }
        }
    }
    frame.render_widget(Paragraph::new(lines), rows_area);
    // scrollbar when the case list overflows the viewport
    vscrollbar(frame, rows_area.inner(ratatui::layout::Margin { vertical: 1, horizontal: 0 }), total, view, sel_pos.unwrap_or(0));

    // pinned "+ add case" footer button (the last column's add slot)
    let add_sel = it.selected == Some(base.saturating_sub(1)) || it.add_focus;
    let brect = Rect { x: btn_area.x, y: btn_area.y, width: 18u16.min(btn_area.width), height: 3 };
    let bstyle = if add_sel { theme::selected() } else { theme::header() };
    let border = if add_sel { bstyle } else { theme::hint() };
    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(border);
    let inner = block.inner(brect);
    frame.render_widget(block, brect);
    if inner.width > 0 && inner.height > 0 {
        let iw = inner.width as usize; // pad to full width so the whole box highlights
        let text = trunc("+ add case", iw);
        let tw = text.chars().count();
        let lead = iw.saturating_sub(tw) / 2;
        let bar = format!("{}{}{}", " ".repeat(lead), text, " ".repeat(iw.saturating_sub(lead + tw)));
        frame.render_widget(Paragraph::new(Span::styled(bar, bstyle)), inner);
    }
    let lc = columns.len().saturating_sub(1);
    let arow = columns.get(lc).map(|c| c.items.len() + 1).unwrap_or(1);
    for dy in 0..brect.height {
        hits.push(arte_core::state::HitRegion { x: brect.x, y: brect.y + dy, w: brect.width, hit: arte_core::state::Hit::Cell { col: lc, row: arow } });
    }
}

fn render_list_board(
    frame: &mut Frame,
    body: Rect,
    columns: &[BoardColumn],
    it: &Interact,
    hits: &mut Vec<arte_core::state::HitRegion>,
    detail: &mut Option<String>,
) {
    let n = columns.len();
    let cells = n + 1; // pillars + the add-pillar slot (the next pillar to create)
    let k = (body.width as usize / 46).clamp(1, cells);
    it.lanes.set(k);
    let rows = cells.div_ceil(k);
    let cell_h = |pi: usize| if pi < n { columns[pi].items.len() + 3 } else { 3 };
    let mut row_h = vec![0u16; rows];
    for i in 0..cells {
        row_h[i / k] = row_h[i / k].max(cell_h(i) as u16 + 1);
    }
    let slot_base = |pi: usize| columns[..pi].iter().map(|c| c.items.len() + 1).sum::<usize>();

    // which masonry row holds the cursor → scroll whole rows so it stays in view.
    // the add-pillar slot isn't an item, so detect it via add_focus (else it never
    // gets pulled into view when selected).
    let focus_pi = if it.add_focus {
        n
    } else {
        it.header_focus
            .or_else(|| it.selected.map(|s| (0..n).find(|&pi| s >= slot_base(pi) && s <= slot_base(pi) + columns[pi].items.len()).unwrap_or(n)))
            .unwrap_or(0)
    };
    let focus_row = focus_pi / k;

    // right gutter for the board-level scrollbar; keep the scroll STABLE so moving
    // onto an already-visible row (incl. the add-pillar slot) doesn't jump the view.
    let track = Rect { x: body.x + body.width.saturating_sub(1), y: body.y, width: 1, height: body.height };
    let body = Rect { width: body.width.saturating_sub(1), ..body };
    let first_row = stable_scroll(it.board_scroll.get(), focus_row, &row_h, body.height);
    it.board_scroll.set(first_row);

    let mut y = body.y;
    let mut n_rendered = 0usize;
    for r in first_row..rows {
        if y >= body.y + body.height {
            break;
        }
        n_rendered += 1;
        let rh = row_h[r].min(body.y + body.height - y);
        let row_rect = Rect { x: body.x, y, width: body.width, height: rh };
        let lane_rects = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(vec![Constraint::Ratio(1, k as u32); k])
            .spacing(2)
            .split(row_rect);
        for li in 0..k {
            let pi = r * k + li;
            if pi > n {
                break;
            }
            let cell = lane_rects[li];
            if cell.height == 0 {
                continue;
            }
            if pi == n {
                // the slot is a 3-tall box; the row may be taller (gap) — don't stretch it
                let slot = Rect { height: 3.min(cell.height), ..cell };
                render_add_button(frame, slot, "add pillar", it, hits); // the next-pillar slot
                continue;
            }
            let col = &columns[pi];
            let items = col.items.len();
            let base = slot_base(pi);
            let sel_row = if it.header_focus == Some(pi) {
                Some(0)
            } else {
                it.selected.filter(|s| *s >= base && *s <= base + items).map(|s| s - base + 1)
            };
            let (lines, d, hit_rows) = column_lines(it, col, pi, sel_row, cell.width as usize);
            if d.is_some() {
                *detail = d;
            }
            // header + rule stay PINNED; only the items below scroll, so the column
            // name never scrolls off and a scrollbar can signal "more".
            let hdr_n = 2usize.min(lines.len());
            let hdr_area = Rect { height: hdr_n as u16, ..cell };
            frame.render_widget(Paragraph::new(lines[..hdr_n].to_vec()), hdr_area);
            let items_area = Rect {
                y: cell.y + hdr_n as u16,
                height: cell.height.saturating_sub(hdr_n as u16),
                ..cell
            };
            let view = items_area.height as usize;
            let cur_il = sel_row
                .and_then(|r| hit_rows.iter().find(|(_, row)| *row == r))
                .map(|(l, _)| (*l as usize).saturating_sub(hdr_n))
                .unwrap_or(0);
            let off = (cur_il + 1).saturating_sub(view);
            frame.render_widget(Paragraph::new(lines[hdr_n..].to_vec()).scroll((off as u16, 0)), items_area);
            for (line, row) in hit_rows {
                let l = line as usize;
                if l < hdr_n {
                    hits.push(arte_core::state::HitRegion { x: cell.x, y: cell.y + l as u16, w: cell.width, hit: arte_core::state::Hit::Cell { col: pi, row } });
                } else {
                    let il = l - hdr_n;
                    if il >= off && (il - off) < view {
                        hits.push(arte_core::state::HitRegion { x: items_area.x, y: items_area.y + (il - off) as u16, w: items_area.width, hit: arte_core::state::Hit::Cell { col: pi, row } });
                    }
                }
            }
        }
        y += rh + 1; // 1-row gap between masonry rows
    }
    vscrollbar(frame, track, rows, n_rendered, focus_row); // board-level: more rows?
}

/// `cards` board: the page frame is the big card; each column is a SECTION card
/// (a layer) holding small component boxes that flow left→right, stacked top↔down.
fn render_cards_board(
    frame: &mut Frame,
    body: Rect,
    columns: &[BoardColumn],
    it: &Interact,
    hits: &mut Vec<arte_core::state::HitRegion>,
    detail: &mut Option<String>,
) {
    const BOX_W: u16 = 24;
    let inner_w = body.width.saturating_sub(2);
    let per_row = (inner_w / (BOX_W + 1)).max(1) as usize;
    it.lanes.set(per_row); // boxes-per-row → nav does 2D over the wrapped grid
    let n = columns.len();
    // Section height = its boxes (components + "+") wrapped into rows × 3 + borders.
    let sec_h = |ci: usize| {
        let boxes = columns[ci].items.len() + 1;
        (boxes.div_ceil(per_row).max(1) * 3 + 2) as u16
    };
    // which section holds the cursor (or the header / the add-card button at n)
    let base_of = |ci: usize| columns[..ci].iter().map(|c| c.items.len() + 1).sum::<usize>();
    let focus = if it.add_focus {
        n // the add-card slot isn't an item; target it so it scrolls into view
    } else {
        it.header_focus
            .or_else(|| it.selected.map(|s| (0..n).find(|&ci| s >= base_of(ci) && s <= base_of(ci) + columns[ci].items.len()).unwrap_or(n)))
            .unwrap_or(0)
    };
    // right gutter for the board-level scrollbar; keep scroll STABLE so moving onto
    // an already-visible section (incl. the add-card slot at n) doesn't jump the view.
    let track = Rect { x: body.x + body.width.saturating_sub(1), y: body.y, width: 1, height: body.height };
    let body = Rect { width: body.width.saturating_sub(1), ..body };
    let spacing = 1u16;
    // the add-card slot (index n) is a row in the flow too, so it scrolls into view
    let mut heights: Vec<u16> = (0..n).map(|ci| sec_h(ci) + spacing).collect();
    heights.push(3 + spacing);
    let first = stable_scroll(it.board_scroll.get(), focus, &heights, body.height);
    it.board_scroll.set(first);
    let mut y = body.y;
    let mut n_rendered = 0usize;
    for ci in first..=n {
        if y >= body.y + body.height {
            break;
        }
        n_rendered += 1;
        if ci == n {
            let h = 3u16.min(body.y + body.height - y);
            render_add_button(frame, Rect { x: body.x, y, width: body.width, height: h }, "add card", it, hits);
            break;
        }
        let h = sec_h(ci).min(body.y + body.height - y);
        let rect = Rect { x: body.x, y, width: body.width, height: h };
        let items = columns[ci].items.len();
        let base = base_of(ci);
        let sel_row = if it.header_focus == Some(ci) {
            Some(0)
        } else {
            it.selected.filter(|s| *s >= base && *s <= base + items).map(|s| s - base + 1)
        };
        render_card(frame, rect, &columns[ci], ci, sel_row, per_row, BOX_W, it, hits, detail);
        y += h + spacing;
    }
    vscrollbar(frame, track, n + 1, n_rendered, focus); // board-level: more sections?
}

/// One layer = a titled section card holding its component boxes.
#[allow(clippy::too_many_arguments)]
fn render_card(
    frame: &mut Frame,
    rect: Rect,
    col: &BoardColumn,
    ci: usize,
    sel_row: Option<usize>,
    per_row: usize,
    box_w: u16,
    it: &Interact,
    hits: &mut Vec<arte_core::state::HitRegion>,
    detail: &mut Option<String>,
) {
    use ratatui::text::Span;
    let name = col.header.split_once(" — ").map(|(n, _)| n.to_string()).unwrap_or_else(|| col.header.clone());
    let on_header = sel_row == Some(0);
    let border = if on_header { theme::header() } else { theme::hint() };
    let title = Span::styled(format!(" {name} "), if on_header { theme::selected() } else { theme::header() });
    // Square = structural container (one level up from the rounded content chips),
    // so the two border levels read as nesting, not noise.
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(border)
        .title(title);
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    if on_header {
        *detail = Some(format!("layer \"{}\"", col.header));
    }
    // the section title row is the header's hit target
    hits.push(arte_core::state::HitRegion {
        x: rect.x,
        y: rect.y,
        w: rect.width,
        hit: arte_core::state::Hit::Cell { col: ci, row: 0 },
    });
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let items = col.items.len();
    // window box-rows so the cursor stays visible in a tall section (100+ boxes)
    let vis_rows = (inner.height / 3).max(1) as usize;
    let cur_brow = match sel_row {
        Some(s) if s > 0 => (s - 1) / per_row,
        _ => 0,
    };
    let brow_off = cur_brow.saturating_sub(vis_rows - 1);
    for j in 0..=items {
        let r_abs = j / per_row;
        if r_abs < brow_off {
            continue; // scrolled above the section viewport
        }
        let r = (r_abs - brow_off) as u16;
        let c = (j % per_row) as u16;
        let bx = inner.x + c * (box_w + 1);
        let by = inner.y + r * 3;
        if by + 3 > inner.y + inner.height || bx + 4 > inner.x + inner.width {
            continue; // doesn't fit
        }
        let bw = box_w.min(inner.x + inner.width - bx);
        let brect = Rect { x: bx, y: by, width: bw, height: 3 };
        if j < items {
            render_component_box(frame, brect, &col.items[j], ci, j, sel_row == Some(j + 1), &name, it, hits, detail);
        } else {
            render_add_component(frame, brect, ci, items, sel_row == Some(items + 1), &name, it, hits, detail);
        }
    }
}

/// One component = a small box inside its section card. Border/label tint by
/// status; selected/working highlight; marker follows `item_marker`.
#[allow(clippy::too_many_arguments)]
fn render_component_box(
    frame: &mut Frame,
    rect: Rect,
    item: &arte_core::protocol::Item,
    ci: usize,
    j: usize,
    selected: bool,
    section: &str,
    it: &Interact,
    hits: &mut Vec<arte_core::state::HitRegion>,
    detail: &mut Option<String>,
) {
    use ratatui::text::{Line, Span};
    let highlighted = it.blink && it.blinking.contains(&(ci, j));
    let editing = selected && it.edit.is_some();
    let (mark, mstyle) = item_marker(item, highlighted);
    // base by selection/status, then the working pulse PATCHES on top — so a
    // selected box still blinks (same mechanic as the list/panel).
    let (mut border, mut label) = if editing || selected {
        (theme::selected(), theme::selected())
    } else if item.status.is_some() {
        (mstyle, theme::body())
    } else {
        (theme::hint(), theme::body())
    };
    if highlighted {
        border = border.patch(theme::working());
        label = label.patch(theme::working());
    }
    // Blinking on-phase gets a thick border (bold glyphs don't thicken box chars);
    // text is bold via theme::working().
    let btype = if highlighted { BorderType::Thick } else { BorderType::Rounded };
    let block = Block::default().borders(Borders::ALL).border_type(btype).border_style(border);
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    if inner.width > 0 && inner.height > 0 {
        let line = if editing {
            Line::from(edit_spans(it.edit.as_ref().unwrap()))
        } else {
            // pad to full inner width so the interior highlight matches the border;
            // has-detail marks (* note, @ attachment) sit at the right edge.
            let iw = inner.width as usize;
            let marks = detail_marks(item);
            let body = trunc(&format!("{mark}{}", item.text), iw.saturating_sub(marks.chars().count()));
            let pad = iw.saturating_sub(body.chars().count() + marks.chars().count());
            Line::from(Span::styled(format!("{body}{}{marks}", " ".repeat(pad)), label))
        };
        frame.render_widget(Paragraph::new(line), inner);
    }
    for dy in 0..rect.height {
        hits.push(arte_core::state::HitRegion {
            x: rect.x,
            y: rect.y + dy,
            w: rect.width,
            hit: arte_core::state::Hit::Cell { col: ci, row: j + 1 },
        });
    }
    if selected {
        *detail = Some(format!("{section}: {}{}", item.text, serves_hint(item)));
    }
}

/// The "+" add-component box at the end of a section.
#[allow(clippy::too_many_arguments)]
fn render_add_component(
    frame: &mut Frame,
    rect: Rect,
    ci: usize,
    items: usize,
    selected: bool,
    section: &str,
    it: &Interact,
    hits: &mut Vec<arte_core::state::HitRegion>,
    detail: &mut Option<String>,
) {
    use ratatui::text::{Line, Span};
    let editing = selected && it.edit.is_some();
    let style = if selected { theme::selected() } else { theme::hint() };
    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(style);
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    if inner.width > 0 && inner.height > 0 {
        if editing {
            frame.render_widget(Paragraph::new(Line::from(edit_spans(it.edit.as_ref().unwrap()))), inner);
            *detail = Some(format!("add to {section} — Enter to save, Esc to cancel"));
        } else {
            // full-width centered "+" so the whole box highlights with the border
            let iw = inner.width as usize;
            let lead = iw / 2;
            let bar = format!("{}+{}", " ".repeat(lead), " ".repeat(iw.saturating_sub(lead + 1)));
            frame.render_widget(Paragraph::new(Span::styled(bar, style)), inner);
            if selected {
                *detail = Some(format!("add to {section} — Enter to type"));
            }
        }
    }
    for dy in 0..rect.height {
        hits.push(arte_core::state::HitRegion {
            x: rect.x,
            y: rect.y + dy,
            w: rect.width,
            hit: arte_core::state::Hit::Cell { col: ci, row: items + 1 },
        });
    }
}

/// The add-pillar/add-card button cell: bordered, rounded, accent (reverse when
/// focused). `label` names the unit it adds ("add pillar" / "add card").
fn render_add_button(
    frame: &mut Frame,
    cell: Rect,
    label: &str,
    it: &Interact,
    hits: &mut Vec<arte_core::state::HitRegion>,
) {
    use ratatui::text::Span;
    // Focused = the WHOLE box highlights: border + a full-width interior share one
    // style, so the rounded border supplies the corners and nothing's half-styled.
    let style = if it.add_focus { theme::selected() } else { theme::header() };
    let border = if it.add_focus { style } else { theme::hint() };
    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(border);
    let inner = block.inner(cell);
    frame.render_widget(block, cell);
    if inner.height > 0 && inner.width > 0 {
        let inner_w = inner.width as usize;
        let text = trunc(&format!("+ {label}"), inner_w);
        let tw = text.chars().count();
        let lead = inner_w.saturating_sub(tw) / 2;
        let bar = format!("{}{}{}", " ".repeat(lead), text, " ".repeat(inner_w.saturating_sub(lead + tw)));
        frame.render_widget(Paragraph::new(Span::styled(bar, style)), inner);
    }
    for dy in 0..cell.height {
        hits.push(arte_core::state::HitRegion {
            x: cell.x,
            y: cell.y + dy,
            w: cell.width,
            hit: arte_core::state::Hit::Button(arte_core::state::Button::AddPillar),
        });
    }
}

/// A column's rendered lines + footer detail + the line offset(s) each cursor row
/// occupies (for the hit-map). Shared header/rule/"+"; items render as flat lines.
/// Returns `(lines, detail, hit_rows)` where
/// `hit_rows` is `(line_offset, cursor_row)` — a card contributes 3 rows.
fn column_lines(
    it: &Interact,
    col: &BoardColumn,
    pi: usize,
    sel_row: Option<usize>,
    w: usize,
) -> (Vec<ratatui::text::Line<'static>>, Option<String>, Vec<(u16, usize)>) {
    use ratatui::text::{Line, Span};
    let hl = theme::selected();
    let head = theme::header();
    let muted = theme::hint();
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut hits: Vec<(u16, usize)> = Vec::new();
    let mut detail = None;
    let (name, prompt) = match col.header.split_once(" — ") {
        Some((nm, p)) => (nm.to_string(), Some(p.to_string())),
        None => (col.header.clone(), None),
    };

    // row 0 — header (layer name in cards mode)
    hits.push((lines.len() as u16, 0));
    if sel_row == Some(0) && it.edit.is_some() {
        let ev = it.edit.as_ref().unwrap();
        let mut spans = Vec::new();
        if it.editing_prompt {
            if !it.header_other.is_empty() {
                spans.push(Span::styled(format!("{} — ", it.header_other), muted));
            }
            spans.extend(edit_spans(ev));
        } else {
            spans.extend(edit_spans(ev));
            if !it.header_other.is_empty() {
                spans.push(Span::styled(format!(" — {}", it.header_other), muted));
            }
        }
        lines.push(Line::from(spans));
    } else if sel_row == Some(0) {
        lines.push(Line::from(Span::styled(trunc(&col.header, w), hl)));
        detail = Some(format!("column \"{}\"", col.header));
    } else {
        let mut hspans = vec![Span::styled(trunc(&name, w), head)];
        if let Some(p) = &prompt {
            let rem = w.saturating_sub(name.chars().count() + 2);
            if rem > 4 {
                hspans.push(Span::styled(format!("  {}", trunc(p, rem)), muted));
            }
        }
        lines.push(Line::from(hspans));
    }

    // rule separating the header from the items
    lines.push(Line::from(Span::styled("─".repeat(w), muted)));

    for (j, item) in col.items.iter().enumerate() {
        let row = j + 1;
        let text = &item.text;
        let highlighted = it.blink && it.blinking.contains(&(pi, j));
        let (mark, mark_style) = item_marker(item, highlighted);
        let editing = sel_row == Some(row) && it.edit.is_some();
        let start = lines.len() as u16;

        if sel_row == Some(row) {
            detail = Some(if editing {
                format!("edit {name} — Enter to save, Esc to cancel")
            } else {
                format!("{name}: {text}{}", serves_hint(item))
            });
        }

        hits.push((start, row));
        if editing {
            lines.push(edit_line(mark, it.edit.as_ref().unwrap()));
        } else if sel_row == Some(row) {
            let st = if highlighted { hl.patch(theme::working()) } else { hl };
            let label = format!("{mark}{}", trunc(text, w.saturating_sub(3)));
            lines.push(Line::from(Span::styled(format!("{label:<w$}"), st)));
        } else if highlighted {
            let label = format!("{mark}{}", trunc(text, w.saturating_sub(3)));
            lines.push(Line::from(Span::styled(format!("{label:<w$}"), mark_style)));
        } else {
            let body = if item.status.is_some() { mark_style } else { theme::body() };
            let marks = detail_marks(item);
            let mut spans = vec![
                Span::styled(mark, mark_style),
                Span::styled(trunc(text, w.saturating_sub(3 + marks.len())), body),
            ];
            if !marks.is_empty() {
                spans.push(Span::styled(format!(" {marks}"), theme::hint()));
            }
            lines.push(Line::from(spans));
        }
    }

    // row items+1 — the "+"
    let arow = col.items.len() + 1;
    hits.push((lines.len() as u16, arow));
    if sel_row == Some(arow) && it.edit.is_some() {
        lines.push(edit_line("＋ ", it.edit.as_ref().unwrap()));
        detail = Some(format!("add to {name} — Enter to save, Esc to cancel"));
    } else if sel_row == Some(arow) {
        lines.push(Line::from(Span::styled(format!("{:<w$}", "＋ add element"), hl)));
        detail = Some(format!("add to {name} — Enter to type"));
    } else {
        lines.push(Line::from(Span::styled("＋ add element".to_string(), muted)));
    }

    (lines, detail, hits)
}

/// Footer: header context > selected detail. (Delete uses the centered overlay,
/// shared by every page, so the footer doesn't carry a separate delete prompt.)
fn board_footer(it: &Interact, detail: Option<String>) -> ratatui::text::Line<'static> {
    use ratatui::text::{Line, Span};
    let head = theme::header();
    if it.header_focus.is_some() {
        let hint = if it.edit.is_some() {
            let part = if it.editing_prompt { "prompt" } else { "name" };
            format!("editing header {part} — Tab switch · Enter save · Esc cancel")
        } else {
            "header — h/l move · Enter rename · j/Esc back to items".to_string()
        };
        Line::from(vec![Span::styled("▸ ", head), Span::raw(hint)])
    } else if let Some(d) = detail {
        Line::from(vec![Span::styled("▸ ", head), Span::raw(d)])
    } else {
        Line::from("")
    }
}

use arte_core::agent::trunc; // shared with the agent digest — one implementation

fn render_children(frame: &mut Frame, area: Rect, layout: LayoutKind, children: &[UiNode], it: &Interact) {
    if children.is_empty() {
        return;
    }
    let direction = match layout {
        LayoutKind::Vertical => Direction::Vertical,
        LayoutKind::Horizontal => Direction::Horizontal,
    };
    let constraints =
        vec![Constraint::Ratio(1, children.len() as u32); children.len()];
    let chunks = Layout::default()
        .direction(direction)
        .constraints(constraints)
        .split(area);
    for (child, chunk) in children.iter().zip(chunks.iter()) {
        render_node(frame, *chunk, child, it);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arte_core::protocol::{Item, Status};

    #[test]
    fn short_path_keeps_the_filename() {
        let p = "/a/very/long/path/to/some/deep/build/artifact/file.txt";
        let s = short_path(p, 20);
        assert!(s.chars().count() <= 20, "fits: {s:?}");
        assert!(s.ends_with("file.txt"), "keeps filename: {s:?}");
        assert!(s.starts_with('…'), "left-ellipsis: {s:?}");
        assert_eq!(short_path("/x/y.txt", 40), "/x/y.txt"); // short stays whole
    }

    // The marker is one function of `highlighted`: arrow when highlighted,
    // bullet/status otherwise — same whether or not the cursor is on top.
    #[test]
    fn marker_tracks_highlight_state() {
        let plain = Item::new("x");
        assert_eq!(item_marker(&plain, false).0, "• "); // normal
        assert_eq!(item_marker(&plain, true).0, "⇒ "); // highlighted (blink on-phase)

        // a linked item is no different at rest — link is not the marker
        let linked = Item { text: "y".into(), serves: vec!["A".into()], ..Default::default() };
        assert_eq!(item_marker(&linked, false).0, "• ");
        assert_eq!(item_marker(&linked, true).0, "⇒ ");

        // status shows its glyph when NOT highlighted; arrow wins on the on-phase
        let ok = Item { text: "z".into(), status: Some(Status::Ok), ..Default::default() };
        assert_eq!(item_marker(&ok, false).0, "✓ ");
        assert_eq!(item_marker(&ok, true).0, "⇒ ");
    }

    fn bare_interact<'a>(
        lanes: &'a std::cell::Cell<usize>,
        scroll: &'a std::cell::Cell<usize>,
        hits: &'a std::cell::RefCell<Vec<arte_core::state::HitRegion>>,
        blink: &'a std::collections::HashSet<(usize, usize)>,
    ) -> Interact<'a> {
        Interact {
            selected: None,
            lanes,
            board_scroll: scroll,
            edit: None,
            add_focus: false,
            blinking: blink,
            blink: false,
            header_focus: None,
            header_other: "",
            editing_prompt: false,
            hits,
            audit: None,
            sheet_filter: None,
        }
    }

    // list column = one line per item; one hit line per cursor row.
    #[test]
    fn list_column_one_line_per_item() {
        use arte_core::protocol::BoardColumn;
        let lanes = std::cell::Cell::new(1);
        let scroll = std::cell::Cell::new(0);
        let hv = std::cell::RefCell::new(Vec::new());
        let bs = std::collections::HashSet::new();
        let it = bare_interact(&lanes, &scroll, &hv, &bs);
        let col = BoardColumn { header: "WHO".into(), key: None, items: vec![Item::new("A"), Item::new("B")] };
        let (lines, _d, hits) = column_lines(&it, &col, 0, None, 30);
        assert_eq!(lines.len(), 5); // header + rule + 2 items + "+"
        assert_eq!(hits.iter().filter(|(_, r)| *r == 1).count(), 1);
    }

    // A `cards` board renders the page → section cards (titled) → component boxes.
    #[test]
    fn cards_board_draws_section_cards() {
        use arte_core::protocol::{BoardColumn, BoardStyle, Item, UiNode, UiMessage};
        use ratatui::{backend::TestBackend, Terminal};
        let mut app = AppState::default();
        app.apply(UiMessage::CreateSurface {
            id: "impl".into(),
            title: "impl.arch".into(),
            root: UiNode::Board {
                id: "m".into(),
                style: BoardStyle::Cards,
                columns: vec![BoardColumn {
                    header: "Frontend".into(),
                    key: Some("fe".into()),
                    items: vec![Item::new("App shell")],
                }],
            },
        })
        .unwrap();
        let mut term = Terminal::new(TestBackend::new(80, 24)).unwrap();
        term.draw(|f| render_app(f, &app)).unwrap();
        let dump: String =
            term.backend().buffer().content().iter().map(|c| c.symbol()).collect();
        assert!(dump.contains("Frontend"), "section card titled with the layer");
        assert!(dump.contains("App shell"), "component box content");
        assert!(dump.contains('╭'), "rounded card borders");
    }

    // Visuals are OBSERVABLE: render to a buffer and inspect cell styles, not a
    // screenshot. A selected + working control row reverses AND pulses yellow on
    // the on-phase, and is reverse-only off-phase (the pulse survives the cursor).
    #[test]
    fn panel_selected_working_row_pulses_observably() {
        use arte_core::protocol::{BoardColumn, BoardStyle, Item, UiMessage, UiNode};
        use ratatui::style::{Color, Modifier};
        use ratatui::{backend::TestBackend, Terminal};
        let mut app = AppState::default();
        app.apply(UiMessage::CreateSurface {
            id: "control".into(),
            title: "control.spec".into(),
            root: UiNode::Board {
                id: "c".into(),
                style: BoardStyle::Panel,
                columns: vec![BoardColumn {
                    header: "R".into(),
                    key: None,
                    items: vec![Item { text: "rule".into(), serves: vec!["Comp".into()], ..Default::default() }],
                }],
            },
        })
        .unwrap();
        app.ui.working = std::iter::once("Comp".to_string()).collect(); // agent works the component
        app.ui.panel_sel = 0; // cursor on the (only) control row
        let reversed_yellow_cell = |app: &AppState| {
            let mut term = Terminal::new(TestBackend::new(90, 24)).unwrap();
            term.draw(|f| render_app(f, app)).unwrap();
            term.backend()
                .buffer()
                .content()
                .iter()
                .any(|c| c.modifier.contains(Modifier::REVERSED) && c.fg == Color::Yellow)
        };
        app.ui.blink.set(true);
        assert!(reversed_yellow_cell(&app), "on-phase: selected row reversed AND working-yellow");
        app.ui.blink.set(false);
        assert!(!reversed_yellow_cell(&app), "off-phase: selected row reversed, not yellow");
    }

    // ---- viewport navigability: cursor stays visible at 100+ items / many sections ----
    fn build_board(style: &str, cols: &[usize]) -> AppState {
        use arte_core::agent::dispatch;
        let mut a = AppState::default();
        let head = if style.is_empty() { "addpage p \"P\"".to_string() } else { format!("addpage p \"P\" {style}") };
        dispatch(&mut a, &head).unwrap();
        for (ci, &cnt) in cols.iter().enumerate() {
            dispatch(&mut a, &format!("addcol p/c{ci} \"H{ci}\"")).unwrap();
            for j in 0..cnt {
                dispatch(&mut a, &format!("add p/c{ci} \"c{ci}i{j:03}\"")).unwrap();
            }
        }
        dispatch(&mut a, "page p").unwrap();
        a
    }
    fn rendered(app: &AppState) -> String {
        use ratatui::{backend::TestBackend, Terminal};
        let mut term = Terminal::new(TestBackend::new(120, 24)).unwrap();
        term.draw(|f| render_app(f, app)).unwrap();
        term.backend().buffer().content().iter().map(|c| c.symbol().to_string()).collect()
    }

    // every style: navigate the cursor to the last item of a 100-row section and to
    // the last of many 25-row sections — it must still be on screen (viewport scrolls).


    #[test]
    fn list_viewport_follows_cursor() {
        use arte_core::agent::dispatch;
        let mut a = build_board("", &[100]); // one tall column
        dispatch(&mut a, "sel p/c0/99").unwrap();
        let buf = rendered(&a);
        assert!(buf.contains("c0i099"), "list: last row of a 100-item column visible");
        assert!(!buf.contains("c0i000"), "list: scrolled past the top");

        let mut a = build_board("", &[25, 25, 25, 25, 25, 25]); // many sections
        dispatch(&mut a, "sel p/c5/24").unwrap();
        assert!(rendered(&a).contains("c5i024"), "list: last item of last section visible");
    }
    #[test]
    fn cards_viewport_follows_cursor() {
        use arte_core::agent::dispatch;
        let mut a = build_board("cards", &[100]); // one tall section
        dispatch(&mut a, "sel p/c0/99").unwrap();
        let buf = rendered(&a);
        assert!(buf.contains("c0i099"), "cards: last box of a 100-box section visible");
        assert!(!buf.contains("c0i000"), "cards: scrolled past the top");

        let mut a = build_board("cards", &[25, 25, 25, 25, 25, 25]); // many sections
        dispatch(&mut a, "sel p/c5/24").unwrap();
        assert!(rendered(&a).contains("c5i024"), "cards: last box of last section visible");
    }
    // edge case: cursor on the add-slot (not an item) must still scroll into view.
    #[test]
    fn add_slot_scrolls_into_view_when_selected() {
        use arte_core::state::Cursor;
        let mut a = build_board("", &[25, 25, 25, 25, 25, 25]);
        a.ui.cursor = Cursor { col: 6, row: 0 }; // on the add-pillar slot (col == n_cols)
        assert!(rendered(&a).contains("add pillar"), "list: add slot pulled into view when selected");

        let mut a = build_board("cards", &[25, 25, 25, 25, 25, 25]);
        a.ui.cursor = Cursor { col: 6, row: 0 };
        assert!(rendered(&a).contains("add card"), "cards: add slot pulled into view when selected");
    }

    #[test]
    fn sheet_viewport_follows_cursor() {
        use arte_core::agent::dispatch;
        let mut a = build_board("sheet", &[100]);
        dispatch(&mut a, "sel p/c0/99").unwrap();
        assert!(rendered(&a).contains("c0i099"), "sheet: last case visible");
    }
    #[test]
    fn panel_viewport_follows_cursor() {
        let mut a = build_board("panel", &[100]);
        a.ui.panel_sel = 99; // cursor on the last control
        assert!(rendered(&a).contains("c0i099"), "panel: last control visible");
    }
}
