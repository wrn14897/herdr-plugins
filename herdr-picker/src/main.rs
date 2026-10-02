//! herdr-picker: fuzzy-search open herdr workspaces with a live preview.

mod app;
#[cfg(test)]
mod bench_tests;
mod git;
mod herdr;
mod model;
mod pool;
mod preview;
mod search;
mod ui;

use std::process::ExitCode;
use std::time::Duration;

use anyhow::Result;

use crate::app::App;
use crate::herdr::Client;

const USAGE: &str = "\
herdr-picker - fuzzy-search herdr workspaces with a live preview

USAGE:
    herdr-picker          open the interactive picker
    herdr-picker --list   print workspaces as TSV (id, number, label, status, cwd, active pane)
    herdr-picker --help   show this message

KEYS:
    type to search · up/down, ctrl-p/n, ctrl-k/j move · enter focus
    esc clears the query, then closes · ctrl-r refresh · ctrl-u clear · ctrl-w delete word";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        None => run_picker(),
        Some("--list") => list(),
        Some("-h" | "--help") => {
            println!("{USAGE}");
            Ok(())
        }
        Some(other) => {
            eprintln!("herdr-picker: unknown argument {other}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("herdr-picker: {err:#}");
            // Inside a plugin popup, the pane closes when we exit; hold the error on screen.
            if std::env::var_os("HERDR_PLUGIN_ENTRYPOINT_ID").is_some() {
                std::thread::sleep(Duration::from_secs(4));
            }
            ExitCode::FAILURE
        }
    }
}

fn run_picker() -> Result<()> {
    // Connect and snapshot before taking over the terminal so failures print plainly.
    let app = App::new(Client::from_env())?;
    let mut terminal = ratatui::init();
    let result = app.run(&mut terminal);
    ratatui::restore();
    result
}

fn list() -> Result<()> {
    let snapshot = Client::from_env().snapshot()?;
    for row in model::build_rows(&snapshot) {
        println!(
            "{}\t{}\t{}\t{}\t{}\t{}",
            row.workspace_id,
            row.number,
            row.label,
            row.status,
            row.cwd,
            row.active_pane_id.unwrap_or_default()
        );
    }
    Ok(())
}
