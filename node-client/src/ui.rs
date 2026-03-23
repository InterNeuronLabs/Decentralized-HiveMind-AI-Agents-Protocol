// node-client/src/ui.rs — Aesthetic terminal output helpers

use colored::Colorize;
use indicatif::{ProgressBar, ProgressStyle};
use std::time::Duration;

// ─── Symbols ─────────────────────────────────────────────────────────────────
pub const OK: &str = "✓";
pub const ERR: &str = "✗";
pub const ACT: &str = "⚡";
pub const DOT: &str = "◆";
pub const ARR: &str = "›";
pub const SPIN_CHARS: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

// ─── Banner ──────────────────────────────────────────────────────────────────
pub fn print_banner(version: &str) {
    println!();
    println!(
        "  {}  {}  {}",
        "⚡".yellow(),
        "OS-PROJECT  NODE CLIENT".bright_white().bold(),
        format!("v{version}").dimmed()
    );
    println!("  {}", "─".repeat(42).bright_blue());
    println!();
}

// ─── Section header ──────────────────────────────────────────────────────────
pub fn section(title: &str) {
    println!("  {} {}", DOT.bright_blue(), title.bright_white().bold());
    println!("  {}", "─".repeat(38).dimmed());
}

// ─── Field row ───────────────────────────────────────────────────────────────
pub fn field(label: &str, value: &str) {
    println!(
        "  {:<12}  {}  {}",
        label.dimmed(),
        ARR.bright_blue(),
        value.bright_white()
    );
}

pub fn field_colored(label: &str, colored_value: &str) {
    println!(
        "  {:<12}  {}  {}",
        label.dimmed(),
        ARR.bright_blue(),
        colored_value
    );
}

// ─── Status messages ─────────────────────────────────────────────────────────
pub fn success(msg: &str) {
    println!("  {} {}", OK.bright_green().bold(), msg.bright_white());
}

pub fn failure(msg: &str) {
    eprintln!("  {} {}", ERR.bright_red().bold(), msg.bright_white());
}

pub fn action(msg: &str) {
    println!("  {} {}", ACT.yellow(), msg.bright_white());
}

pub fn hint(msg: &str) {
    println!("  {}", msg.dimmed());
}

pub fn gap() {
    println!();
}

// ─── Status value colorizer ──────────────────────────────────────────────────
pub fn status_colored(s: &str) -> String {
    match s.to_lowercase().as_str() {
        "complete" => format!("{} complete", OK.bright_green()),
        "failed" => format!("{} failed", ERR.bright_red()),
        "expired" => format!("{} expired", ERR.yellow()),
        "running" => "⟳ running".yellow().to_string(),
        "pending" => "○ pending".dimmed().to_string(),
        _ => s.normal().to_string(),
    }
}

// ─── Table ───────────────────────────────────────────────────────────────────
/// Print a styled table.
/// `cols` is a slice of (header_label, column_width) pairs.
/// `rows` is a slice of row vectors; column 1 (index 1) gets status-colorized.
pub fn table(cols: &[(&str, usize)], rows: &[Vec<String>]) {
    let header: Vec<String> = cols.iter().map(|(h, w)| format!("{h:<w$}")).collect();
    println!("  {}", header.join("   ").bright_white().bold());

    let sep: Vec<String> = cols.iter().map(|(_, w)| "─".repeat(*w)).collect();
    println!("  {}", sep.join("   ").dimmed());

    for row in rows {
        let cells: Vec<String> = row
            .iter()
            .zip(cols.iter())
            .enumerate()
            .map(|(i, (cell, (_, w)))| {
                if i == 1 {
                    // Status column — colorize, then pad by raw visible length
                    let colored = status_colored(cell);
                    let pad = w.saturating_sub(cell.len());
                    format!("{colored}{}", " ".repeat(pad))
                } else {
                    format!("{cell:<w$}")
                }
            })
            .collect();
        println!("  {}", cells.join("   "));
    }
    println!();
}

// ─── Spinner ─────────────────────────────────────────────────────────────────
pub fn spinner(msg: &str) -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::with_template("  {spinner:.cyan}  {msg}")
            .unwrap()
            .tick_strings(SPIN_CHARS),
    );
    pb.set_message(msg.to_owned());
    pb.enable_steady_tick(Duration::from_millis(80));
    pb
}

// ─── Worker-loop task events ──────────────────────────────────────────────────
pub fn task_start(task_id: &str, role: &str) {
    let now = chrono::Local::now().format("%H:%M:%S");
    let short = &task_id[..task_id.len().min(8)];
    println!(
        "  {}  {}  {:<12}  {}",
        format!("[{now}]").dimmed(),
        ACT.yellow(),
        role.bright_cyan(),
        format!("task {short}…").dimmed(),
    );
}

pub fn task_done(task_id: &str) {
    let now = chrono::Local::now().format("%H:%M:%S");
    let short = &task_id[..task_id.len().min(8)];
    println!(
        "  {}  {}  {}",
        format!("[{now}]").dimmed(),
        OK.bright_green(),
        format!("result submitted  task {short}…").dimmed(),
    );
}

pub fn task_err(task_id: &str, err: &str) {
    let now = chrono::Local::now().format("%H:%M:%S");
    let short = &task_id[..task_id.len().min(8)];
    println!(
        "  {}  {}  {}  {}",
        format!("[{now}]").dimmed(),
        ERR.bright_red(),
        format!("task {short}…").dimmed(),
        err.bright_red(),
    );
}
