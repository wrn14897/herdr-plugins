//! herdr-picker: fuzzy-search open herdr workspaces with a live preview.

mod app;
#[cfg(test)]
mod bench_tests;
mod content;
mod git;
mod herdr;
mod index;
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
    herdr-picker --search QUERY
                          index everything, then print ranked matches (no UI)
    herdr-picker --help   show this message

KEYS:
    type to search · up/down, ctrl-p/n, ctrl-k/j move · enter focus
    esc clears the query, then closes · ctrl-r refresh · ctrl-u clear · ctrl-w delete word";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        None => run_picker(),
        Some("--list") => list(),
        Some("--search") => search(&args[1..].join(" ")),
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

fn search(query: &str) -> Result<()> {
    use std::collections::HashMap;

    let client = Client::from_env();
    let mut rows = model::build_rows(&client.snapshot()?);
    let mut stores: HashMap<String, content::Store> = HashMap::new();
    for row in &mut rows {
        for pane in row.indexed_panes() {
            let job = index::PaneJob {
                workspace_id: row.workspace_id.clone(),
                pane_id: pane.pane_id.clone(),
                pane_label: pane.short_name(),
            };
            let update = index::read_screen(&client, &job);
            stores
                .entry(update.workspace_id)
                .or_default()
                .set_segment(update.key, update.segment);
        }
    }

    let mut hits = search::Searcher::new().search(query, &rows);
    if let Some(content_query) = content::Query::parse(query) {
        search::merge_content(&mut hits, &rows, &content_query, |row| {
            stores.get(&row.workspace_id)
        });
    }
    for hit in hits {
        let row = &rows[hit.row];
        let detail = hit.detail.map_or(String::new(), |d| {
            format!("\t{}: {}", d.badge.unwrap_or_default(), d.text)
        });
        println!(
            "{}\t{}\t{}\t{} content hits{detail}",
            row.workspace_id,
            hit.kind.badge(),
            row.label,
            hit.content.len()
        );
    }
    Ok(())
}
