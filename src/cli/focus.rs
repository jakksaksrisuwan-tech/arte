//! Focus / working-set signal: stored in `.truth/.focus`, one node id per line.
//! Drives the visualiser pulse and gates edits.

use std::fs;

use crate::*;

pub fn focus_path() -> String {
    format!("{}/.focus", truth_dir())
}

/// Reads `.truth/.focus` line by line. Missing file → empty (no focus set
/// yet, which is fine). Read errors other than NotFound are surfaced to
/// stderr — silently treating a permission-denied as "no focus" hides a
/// real problem from the agent reading its own working set.
pub fn read_focus() -> Vec<String> {
    match fs::read_to_string(focus_path()) {
        Ok(s) => s.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => { eprintln!("warn: could not read {}: {e}", focus_path()); Vec::new() }
    }
}

/// `arte working <id>...` declare focus · `arte working` show · `arte working clear`.
pub fn cmd_working() {
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
            match fs::remove_file(focus_path()) {
                Ok(_) => println!("focus cleared"),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => println!("focus cleared (was already empty)"),
                Err(e) => { eprintln!("could not clear {}: {e}", focus_path()); std::process::exit(1); }
            }
        }
        Some(first) => {
            let ids: Vec<String> = std::iter::once(first.to_string()).chain(a).collect();
            for i in &ids {
                if !node_exists(i) {
                    eprintln!("no node '{i}' — map it on the board first (arte add ...)");
                    std::process::exit(2);
                }
            }
            if let Err(e) = fs::write(focus_path(), ids.join("\n") + "\n") {
                eprintln!("could not write {}: {e}", focus_path());
                std::process::exit(1);
            }
            println!("▶ working on: {}  (pulsing in the visualiser)", ids.join(", "));
        }
    }
}