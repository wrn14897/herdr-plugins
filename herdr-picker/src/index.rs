//! Background indexing jobs that feed the content stores.
//!
//! Each job produces one [`Update`] that replaces a segment (one pane's screen,
//! or one agent session) in its workspace's store.

use std::collections::HashMap;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use crate::config::Config;
use crate::content::{Segment, Source};
use crate::herdr::Client;
use crate::model::{PaneRow, Row};
use crate::pool::Pool;
use crate::transcript::{self, Limits, Location, Request};

#[derive(Debug)]
pub struct Update {
    pub workspace_id: String,
    pub key: String,
    /// `None` means "unchanged since the last read": keep the existing segment.
    pub segment: Option<Segment>,
}

#[derive(Debug, Clone)]
pub enum Job {
    Screen {
        pane_id: String,
        pane: String,
        lines: u32,
    },
    Transcript {
        pane_id: String,
        agent: String,
        request: Request,
        limits: Limits,
    },
}

#[derive(Debug, Clone)]
pub struct Task {
    pub workspace_id: String,
    pub job: Job,
}

/// Last indexed version of each transcript, so unchanged sessions are skipped.
pub type Versions = Arc<Mutex<HashMap<String, (Location, u64)>>>;

/// All indexing tasks for one workspace under `config`.
pub fn tasks_for(row: &Row, config: &Config) -> Vec<Task> {
    if !config.content.enabled {
        return Vec::new();
    }
    let mut tasks = Vec::new();
    for pane in row.indexed_panes() {
        let task = |job| Task {
            workspace_id: row.workspace_id.clone(),
            job,
        };
        tasks.push(task(Job::Screen {
            pane_id: pane.pane_id.clone(),
            pane: pane.short_name(),
            lines: config.content.screen_lines,
        }));
        if let Some(request) = transcript_request(pane, row, config) {
            tasks.push(task(Job::Transcript {
                pane_id: pane.pane_id.clone(),
                agent: request.agent.clone(),
                request,
                limits: Limits {
                    messages: config.content.transcript_messages,
                    bytes: config.content.transcript_bytes,
                },
            }));
        }
    }
    tasks
}

fn transcript_request(pane: &PaneRow, row: &Row, config: &Config) -> Option<Request> {
    let agent = pane.agent.as_deref()?;
    if !transcript::supported(agent) || !config.indexes_transcripts_of(agent) {
        return None;
    }
    let cwd = if pane.cwd.is_empty() {
        row.cwd.clone()
    } else {
        pane.cwd.clone()
    };
    Some(Request {
        agent: agent.to_owned(),
        cwd,
        session: pane.session.clone(),
    })
}

pub fn queue(
    pool: &Pool,
    client: &Client,
    versions: &Versions,
    task: Task,
    urgent: bool,
    tx: Sender<Update>,
) {
    let client = client.clone();
    let versions = Arc::clone(versions);
    pool.submit(urgent, move || {
        let _ = tx.send(run(&client, &versions, &task));
    });
}

pub fn run(client: &Client, versions: &Versions, task: &Task) -> Update {
    let workspace_id = task.workspace_id.clone();
    match &task.job {
        Job::Screen {
            pane_id,
            pane,
            lines,
        } => {
            let segment = client
                .read_recent(pane_id, *lines)
                .map(|text| screen_segment(pane_id, pane, &text))
                .unwrap_or_default();
            Update {
                workspace_id,
                key: format!("screen:{pane_id}"),
                segment: Some(segment),
            }
        }
        Job::Transcript {
            pane_id,
            agent,
            request,
            limits,
        } => {
            let key = format!("chat:{pane_id}");
            let Some((location, version)) = transcript::locate(request) else {
                return Update {
                    workspace_id,
                    key,
                    segment: Some(Vec::new()),
                };
            };
            let current = (location.clone(), version);
            {
                let mut seen = versions
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if seen.get(&key) == Some(&current) {
                    return Update {
                        workspace_id,
                        key,
                        segment: None,
                    };
                }
                seen.insert(key.clone(), current);
            }
            let segment = transcript::read(&location, *limits)
                .into_iter()
                .map(|(role, text)| {
                    let source = Source::Chat {
                        pane_id: pane_id.clone(),
                        agent: agent.clone(),
                        role,
                    };
                    (source, text)
                })
                .collect();
            Update {
                workspace_id,
                key,
                segment: Some(segment),
            }
        }
    }
}

fn screen_segment(pane_id: &str, pane: &str, text: &str) -> Segment {
    let source = Source::Screen {
        pane_id: pane_id.to_owned(),
        pane: pane.to_owned(),
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
        let segment = screen_segment("w:p1", "shell", "$ ls   \n\n   \nsrc\n");
        let lines: Vec<_> = segment.iter().map(|(_, l)| l.as_str()).collect();
        assert_eq!(lines, ["$ ls", "src"]);
    }
}
