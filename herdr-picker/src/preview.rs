//! Pane screen previews fetched on a worker thread.
//!
//! The UI sends pane ids as the selection moves; the worker drains to the most
//! recent request before reading, so fast scrolling never queues stale reads.

use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;

use ansi_to_tui::IntoText;
use ratatui::text::Text;

use crate::herdr::Client;

#[derive(Debug)]
pub struct Screen {
    pub pane_id: String,
    pub text: Result<Text<'static>, String>,
}

pub fn spawn_worker(client: Client, results: Sender<Screen>) -> Sender<String> {
    let (tx, rx) = channel::<String>();
    thread::spawn(move || worker(&client, &rx, &results));
    tx
}

fn worker(client: &Client, rx: &Receiver<String>, results: &Sender<Screen>) {
    while let Ok(mut pane_id) = rx.recv() {
        while let Ok(newer) = rx.try_recv() {
            pane_id = newer;
        }
        let text = client
            .read_visible(&pane_id)
            .map_err(|e| e.to_string())
            .and_then(|raw| to_text(&raw).map_err(|e| e.to_string()));
        if results.send(Screen { pane_id, text }).is_err() {
            return;
        }
    }
}

/// Converts an ANSI screen to styled text, dropping trailing blank rows so the
/// interesting part (usually the prompt) can be bottom-aligned in the preview.
fn to_text(raw: &str) -> Result<Text<'static>, ansi_to_tui::Error> {
    let lines: Vec<&str> = raw.lines().collect();
    let end = lines
        .iter()
        .rposition(|l| !strip_ansi(l).trim().is_empty())
        .map_or(0, |i| i + 1);
    let mut text = lines[..end].join("\n").into_bytes().into_text()?;
    for line in &mut text.lines {
        // Trim padding spaces so lines don't carry invisible width.
        while line
            .spans
            .last()
            .is_some_and(|s| s.content.trim_end().is_empty())
        {
            line.spans.pop();
        }
        if let Some(last) = line.spans.last_mut() {
            let trimmed = last.content.trim_end().to_owned();
            last.content = trimmed.into();
        }
    }
    Ok(text)
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('[') => {
                for c in chars.by_ref() {
                    if ('\x40'..='\x7e').contains(&c) {
                        break;
                    }
                }
            }
            Some(']') => {
                while let Some(c) = chars.next() {
                    if c == '\x07' || (c == '\x1b' && chars.next_if_eq(&'\\').is_some()) {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_csi_and_osc() {
        assert_eq!(
            strip_ansi("\x1b[1;31mred\x1b[0m \x1b]0;title\x07x"),
            "red x"
        );
    }

    #[test]
    fn drops_trailing_blank_rows() {
        let text = to_text("\x1b[32m$ ls\x1b[0m   \nfoo\n\x1b[0m   \n\n").unwrap();
        assert_eq!(text.lines.len(), 2);
        assert_eq!(text.lines[0].to_string(), "$ ls");
    }
}
