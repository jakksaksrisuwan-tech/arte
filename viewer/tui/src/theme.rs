//! The palette, in one place. Nothing else hardcodes a color or style — the
//! renderer asks for styles by *role* (header, hint, content…), so swapping the
//! four primitives below restyles the entire UI. Color carries meaning here:
//! accent = what you navigate by, dim = scaffolding, fg = content.
//!
//! Terminal-theme compatible by design: the primitives are the 16 ANSI palette
//! *slots* (and `Reset` = the terminal's own fg/bg), NOT fixed RGB. The terminal
//! owns the actual colors, so the UI inherits the user's theme automatically —
//! light/dark, Solarized, Gruvbox, a live theme switch — with zero detection on
//! our side. Rule: never use `Color::Rgb`/`Color::Indexed(>15)` here; that pins
//! a color and breaks the moment the user retheme their terminal.

use ratatui::style::{Color, Modifier, Style};

// --- primitives: ANSI slots only (the terminal maps these to its theme) ---
const ACCENT: Color = Color::Cyan; // ANSI 6 — categories, markers
const FG: Color = Color::Reset; // terminal default fg (follows light/dark)
const DIM: Color = Color::DarkGray; // ANSI 8 (bright black) — scaffolding
const ALERT: Color = Color::Red; // ANSI 1 — destructive prompts
const OK: Color = Color::Green; // ANSI 2 — status: ok
const WARN: Color = Color::Yellow; // ANSI 3 — status: pending

// --- semantic roles (the only thing the renderer references) ---
/// Category headers and accent markers.
pub fn header() -> Style {
    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
}
/// Supporting/scaffolding text — rules, bullets, guiding prompts, affordances.
pub fn hint() -> Style {
    Style::default().fg(DIM)
}
/// Primary content (board items, body text).
pub fn body() -> Style {
    Style::default().fg(FG)
}
/// Plain emphasis without accent (metric values, table headers).
pub fn strong() -> Style {
    Style::default().add_modifier(Modifier::BOLD)
}
/// The active selection / highlight.
pub fn selected() -> Style {
    Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
}
/// Destructive confirmation.
pub fn alert() -> Style {
    Style::default()
        .fg(ALERT)
        .add_modifier(Modifier::BOLD | Modifier::REVERSED)
}
/// The agent's working set — pulses for attention. Warning-yellow + bold so it
/// reads as "look here", distinct in shape from the reverse-bar cursor.
pub fn working() -> Style {
    Style::default().fg(WARN).add_modifier(Modifier::BOLD)
}
/// Producer-set item status (ok / ko / pending).
pub fn ok() -> Style {
    Style::default().fg(OK)
}
pub fn fail() -> Style {
    Style::default().fg(ALERT)
}
pub fn pending() -> Style {
    Style::default().fg(WARN)
}
