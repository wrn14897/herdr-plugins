//! Background indexing jobs that feed the content stores.

use std::sync::mpsc::Sender;

use crate::content::{Segment, Source};
use crate::herdr::Client;
use crate::pool::Pool;

/// Scrollback rows read per pane for screen search.
pub const SCREEN_LINES: u32 = 500;

#[derive(Debug)]
pub struct Update {
    pub workspace_id: String,
    pub key: String,
    pub segment: Segment,
}

#[derive(Debug, Clone)]
pub struct PaneJob {
    pub workspace_id: String,
    pub pane_id: String,
    pub pane_label: String,
}

pub fn screen_key(pane_id: &str) -> String {
    format!("screen:{pane_id}")
}

/// Reads a pane's recent scrollback and sends it as one document per line.
pub fn queue_screen(pool: &Pool, client: &Client, job: PaneJob, urgent: bool, tx: Sender<Update>) {
    let client = client.clone();
    pool.submit(urgent, move || {
        let _ = tx.send(read_screen(&client, &job));
    });
}

pub fn read_screen(client: &Client, job: &PaneJob) -> Update {
    let segment = client
        .read_recent(&job.pane_id, SCREEN_LINES)
        .map(|text| screen_segment(job, &text))
        .unwrap_or_default();
    Update {
        workspace_id: job.workspace_id.clone(),
        key: screen_key(&job.pane_id),
        segment,
    }
}

fn screen_segment(job: &PaneJob, text: &str) -> Segment {
    let source = Source::Screen {
        pane_id: job.pane_id.clone(),
        pane: job.pane_label.clone(),
    };
    text.lines()
        .map(str::trim_end)
        .filter(|line| !line.trim().is_empty())
        .map(|line| (source.clone(), line.to_owned()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_segment_skips_blank_lines() {
        let job = PaneJob {
            workspace_id: "w".into(),
            pane_id: "w:p1".into(),
            pane_label: "shell".into(),
        };
        let segment = screen_segment(&job, "$ ls   \n\n   \nsrc\n");
        let lines: Vec<_> = segment.iter().map(|(_, l)| l.as_str()).collect();
        assert_eq!(lines, ["$ ls", "src"]);
    }
}
