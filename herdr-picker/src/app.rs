//! Picker state and input handling.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::DefaultTerminal;
use ratatui::text::Text;
use ratatui::widgets::ListState;

use crate::git::{self, GitInfo};
use crate::herdr::Client;
use crate::model::{self, Row};
use crate::pool::Pool;
use crate::preview::{self, Screen};
use crate::search::{Hit, Searcher};
use crate::ui;

/// How often the selected pane's screen is re-read while the picker is idle.
const PREVIEW_REFRESH: Duration = Duration::from_millis(400);
const TICK: Duration = Duration::from_millis(40);
/// Background workers shared by git, screen, and transcript lookups.
const WORKERS: usize = 4;

#[derive(Debug)]
pub struct App {
    client: Client,
    pub rows: Vec<Row>,
    pub hits: Vec<Hit>,
    pub list: ListState,
    pub list_offset: usize,
    pub query: String,
    pub git: HashMap<String, Option<GitInfo>>,
    pub screens: HashMap<String, Result<Text<'static>, String>>,
    pub error: Option<String>,
    searcher: Searcher,
    pool: Pool,
    git_tx: Sender<(String, Option<GitInfo>)>,
    git_rx: Receiver<(String, Option<GitInfo>)>,
    preview_tx: Sender<String>,
    screen_rx: Receiver<Screen>,
    last_request: Option<(String, Instant)>,
    quit: bool,
}

impl App {
    pub fn new(client: Client) -> Result<Self> {
        let (git_tx, git_rx) = channel();
        let (screen_tx, screen_rx) = channel();
        let preview_tx = preview::spawn_worker(client.clone(), screen_tx);
        let mut app = Self {
            client,
            rows: Vec::new(),
            hits: Vec::new(),
            list: ListState::default(),
            list_offset: 0,
            query: String::new(),
            git: HashMap::new(),
            screens: HashMap::new(),
            error: None,
            searcher: Searcher::new(),
            pool: Pool::new(WORKERS),
            git_tx,
            git_rx,
            preview_tx,
            screen_rx,
            last_request: None,
            quit: false,
        };
        app.reload()?;
        Ok(app)
    }

    pub fn run(mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        while !self.quit {
            self.drain_background();
            self.request_preview();
            terminal.draw(|frame| ui::draw(frame, &mut self))?;
            if event::poll(TICK)? {
                match event::read()? {
                    Event::Key(key) if key.kind != KeyEventKind::Release => self.on_key(key),
                    Event::Paste(text) => {
                        self.query.push_str(&text.replace(['\n', '\r'], " "));
                        self.refilter();
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }

    fn reload(&mut self) -> Result<()> {
        let snapshot = self.client.snapshot()?;
        self.rows = model::build_rows(&snapshot);
        let mut queued = HashSet::new();
        for row in &mut self.rows {
            match self.git.get(&row.cwd) {
                Some(info) => row.set_branch(info.as_ref().map(|g| g.branch.as_str())),
                None if !row.cwd.is_empty() && queued.insert(row.cwd.clone()) => {
                    git::queue_lookup(&self.pool, row.cwd.clone(), self.git_tx.clone());
                }
                None => {}
            }
        }
        self.refilter_keep();
        Ok(())
    }

    /// Re-runs the search after the query changed: selection jumps to the best match.
    fn refilter(&mut self) {
        self.search(false);
    }

    /// Re-runs the search after background data changed: selection is preserved.
    fn refilter_keep(&mut self) {
        self.search(true);
    }

    fn search(&mut self, keep_selection: bool) {
        let selected_id = keep_selection
            .then(|| self.selected().map(|r| r.workspace_id.clone()))
            .flatten();
        self.hits = self.searcher.search(&self.query, &self.rows);
        let keep = selected_id.and_then(|id| {
            self.hits
                .iter()
                .position(|h| self.rows[h.row].workspace_id == id)
        });
        self.list.select(if self.hits.is_empty() {
            None
        } else {
            Some(keep.unwrap_or(0))
        });
    }

    pub fn selected(&self) -> Option<&Row> {
        let hit = self.hits.get(self.list.selected()?)?;
        self.rows.get(hit.row)
    }

    fn drain_background(&mut self) {
        let mut git_changed = false;
        while let Ok((dir, info)) = self.git_rx.try_recv() {
            let branch = info.as_ref().map(|g| g.branch.as_str());
            for row in self.rows.iter_mut().filter(|r| r.cwd == dir) {
                row.set_branch(branch);
            }
            self.git.insert(dir, info);
            git_changed = true;
        }
        if git_changed && !self.query.is_empty() {
            self.refilter_keep();
        }
        while let Ok(screen) = self.screen_rx.try_recv() {
            self.screens.insert(screen.pane_id, screen.text);
        }
    }

    fn request_preview(&mut self) {
        let Some(pane) = self.selected().and_then(|r| r.active_pane_id.clone()) else {
            return;
        };
        let stale = match &self.last_request {
            Some((last, at)) => *last != pane || at.elapsed() >= PREVIEW_REFRESH,
            None => true,
        };
        if stale && self.preview_tx.send(pane.clone()).is_ok() {
            self.last_request = Some((pane, Instant::now()));
        }
    }

    fn on_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('c') if ctrl => self.quit = true,
            KeyCode::Esc => {
                if self.query.is_empty() {
                    self.quit = true;
                } else {
                    self.query.clear();
                    self.refilter();
                }
            }
            KeyCode::Enter => self.accept(),
            KeyCode::Up | KeyCode::BackTab => self.step(-1),
            KeyCode::Down | KeyCode::Tab => self.step(1),
            KeyCode::Char('p' | 'k') if ctrl => self.step(-1),
            KeyCode::Char('n' | 'j') if ctrl => self.step(1),
            KeyCode::PageUp => self.step(-10),
            KeyCode::PageDown => self.step(10),
            KeyCode::Char('r') if ctrl => {
                if let Err(e) = self.reload() {
                    self.error = Some(e.to_string());
                }
            }
            KeyCode::Char('u') if ctrl => {
                self.query.clear();
                self.refilter();
            }
            KeyCode::Char('w') if ctrl => {
                let trimmed = self.query.trim_end().len();
                let cut = self.query[..trimmed]
                    .rfind(char::is_whitespace)
                    .map_or(0, |i| i + 1);
                self.query.truncate(cut);
                self.refilter();
            }
            KeyCode::Backspace => {
                self.query.pop();
                self.refilter();
            }
            KeyCode::Char(c) if !ctrl && !key.modifiers.contains(KeyModifiers::ALT) => {
                self.query.push(c);
                self.refilter();
            }
            _ => {}
        }
    }

    fn step(&mut self, delta: isize) {
        let len = self.hits.len();
        if len == 0 {
            return;
        }
        let cur = self.list.selected().unwrap_or(0) as isize;
        let next = (cur + delta).rem_euclid(len as isize);
        // Wrap only on single steps; page jumps clamp at the ends.
        let next = if delta.abs() > 1 {
            (cur + delta).clamp(0, len as isize - 1)
        } else {
            next
        };
        self.list.select(Some(next as usize));
    }

    fn accept(&mut self) {
        let Some(id) = self.selected().map(|r| r.workspace_id.clone()) else {
            return;
        };
        match self.client.focus_workspace(&id) {
            Ok(()) => self.quit = true,
            Err(e) => self.error = Some(e.to_string()),
        }
    }
}
