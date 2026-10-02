//! Picker state and input handling.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::DefaultTerminal;
use ratatui::text::Text;
use ratatui::widgets::ListState;

use crate::config::Config;
use crate::content::{self, ContentHit, Store};
use crate::git::{self, GitInfo};
use crate::herdr::Client;
use crate::index::{self, Update, Versions};
use crate::model::{self, Row};
use crate::pool::Pool;
use crate::preview::{self, Screen};
use crate::search::{self, Hit, Searcher};
use crate::ui;

/// How often the selected pane's screen is re-read while the picker is idle.
const PREVIEW_REFRESH: Duration = Duration::from_millis(400);
/// How often the selected workspace's searchable content is re-indexed.
const CONTENT_REFRESH: Duration = Duration::from_secs(2);
const TICK: Duration = Duration::from_millis(40);
/// Background workers shared by git, screen, and transcript lookups.
const WORKERS: usize = 4;

/// What the preview's lower half shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    /// The live screen of the workspace's active pane.
    Live,
    /// Content matches for the query, one at a time.
    Hits,
}

#[derive(Debug)]
pub struct App {
    client: Client,
    config: Config,
    own_pane: Option<String>,
    versions: Versions,
    pub rows: Vec<Row>,
    pub hits: Vec<Hit>,
    pub list: ListState,
    pub list_offset: usize,
    pub query: String,
    pub content_query: Option<content::Query>,
    pub git: HashMap<String, Option<GitInfo>>,
    pub screens: HashMap<String, Result<Text<'static>, String>>,
    pub stores: HashMap<String, Store>,
    pub error: Option<String>,
    /// Index of the content hit shown in the preview, within the selected row.
    pub hit_cursor: usize,
    view_override: Option<View>,
    pub index_pending: usize,
    pub index_total: usize,
    searcher: Searcher,
    pool: Pool,
    git_tx: Sender<(String, Option<GitInfo>)>,
    git_rx: Receiver<(String, Option<GitInfo>)>,
    index_tx: Sender<Update>,
    index_rx: Receiver<Update>,
    preview_tx: Sender<String>,
    screen_rx: Receiver<Screen>,
    last_preview: Option<(String, Instant)>,
    last_content_refresh: Option<(String, Instant)>,
    quit: bool,
}

impl App {
    pub fn new(client: Client, config: Config) -> Result<Self> {
        let (git_tx, git_rx) = channel();
        let (index_tx, index_rx) = channel();
        let (screen_tx, screen_rx) = channel();
        let preview_tx = preview::spawn_worker(client.clone(), screen_tx);
        let mut app = Self {
            client,
            config,
            own_pane: index::own_pane(),
            versions: Versions::default(),
            rows: Vec::new(),
            hits: Vec::new(),
            list: ListState::default(),
            list_offset: 0,
            query: String::new(),
            content_query: None,
            git: HashMap::new(),
            screens: HashMap::new(),
            stores: HashMap::new(),
            error: None,
            hit_cursor: 0,
            view_override: None,
            index_pending: 0,
            index_total: 0,
            searcher: Searcher::new(),
            pool: Pool::new(WORKERS),
            git_tx,
            git_rx,
            index_tx,
            index_rx,
            preview_tx,
            screen_rx,
            last_preview: None,
            last_content_refresh: None,
            quit: false,
        };
        app.reload()?;
        Ok(app)
    }

    pub fn run(mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        while !self.quit {
            self.drain_background();
            self.request_preview();
            self.refresh_selected_content();
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

        // Drop stores of closed workspaces, then (re)index everything.
        let live: HashSet<&str> = self.rows.iter().map(|r| r.workspace_id.as_str()).collect();
        self.stores.retain(|id, _| live.contains(id.as_str()));
        if let Ok(mut versions) = self.versions.lock() {
            versions.clear();
        }
        self.index_pending = 0;
        self.index_total = 0;
        for i in 0..self.rows.len() {
            self.queue_row_index(i, false);
        }

        self.refilter_keep();
        Ok(())
    }

    fn queue_row_index(&mut self, row: usize, urgent: bool) {
        for task in index::tasks_for(&self.rows[row], &self.config, self.own_pane.as_deref()) {
            self.index_pending += 1;
            self.index_total += 1;
            index::queue(
                &self.pool,
                &self.client,
                &self.versions,
                task,
                urgent,
                self.index_tx.clone(),
            );
        }
    }

    /// Re-runs the search after the query changed: selection jumps to the best match.
    fn refilter(&mut self) {
        self.content_query = content::Query::parse(&self.query);
        self.search(false);
    }

    /// Re-runs the search after background data changed: selection is preserved.
    fn refilter_keep(&mut self) {
        self.search(true);
    }

    fn search(&mut self, keep_selection: bool) {
        let previous = self.selected().map(|r| r.workspace_id.clone());
        let selected_id = if keep_selection {
            previous.clone()
        } else {
            None
        };

        self.hits = self.searcher.search(&self.query, &self.rows);
        if let Some(query) = &self.content_query {
            let stores = &self.stores;
            search::merge_content(&mut self.hits, &self.rows, query, |row| {
                stores.get(&row.workspace_id)
            });
        }

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
        if self.selected().map(|r| &r.workspace_id) != previous.as_ref() || !keep_selection {
            self.reset_hit_view();
        }
        let count = self.selected_hit().map_or(0, |h| h.content.len());
        self.hit_cursor = self.hit_cursor.min(count.saturating_sub(1));
    }

    fn reset_hit_view(&mut self) {
        self.hit_cursor = 0;
        self.view_override = None;
    }

    pub fn selected_hit(&self) -> Option<&Hit> {
        self.hits.get(self.list.selected()?)
    }

    pub fn selected(&self) -> Option<&Row> {
        self.rows.get(self.selected_hit()?.row)
    }

    /// The content hit currently shown in the preview, with its store and position.
    pub fn current_content(&self) -> Option<(&Store, &ContentHit, usize, usize)> {
        let hit = self.selected_hit()?;
        let store = self.stores.get(&self.rows[hit.row].workspace_id)?;
        let cursor = self.hit_cursor.min(hit.content.len().checked_sub(1)?);
        Some((store, &hit.content[cursor], cursor, hit.content.len()))
    }

    pub fn view(&self) -> View {
        if self.current_content().is_none() {
            return View::Live;
        }
        self.view_override.unwrap_or(
            if self.selected_hit().is_some_and(|h| h.kind.is_content()) {
                View::Hits
            } else {
                View::Live
            },
        )
    }

    fn drain_background(&mut self) {
        let mut changed = false;
        while let Ok((dir, info)) = self.git_rx.try_recv() {
            let branch = info.as_ref().map(|g| g.branch.as_str());
            for row in self.rows.iter_mut().filter(|r| r.cwd == dir) {
                row.set_branch(branch);
            }
            self.git.insert(dir, info);
            changed |= !self.query.is_empty();
        }
        while let Ok(update) = self.index_rx.try_recv() {
            self.index_pending = self.index_pending.saturating_sub(1);
            if let Some(segment) = update.segment {
                self.stores
                    .entry(update.workspace_id)
                    .or_default()
                    .set_segment(update.key, segment);
                changed |= self.content_query.is_some();
            }
        }
        if changed {
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
        let stale = match &self.last_preview {
            Some((last, at)) => *last != pane || at.elapsed() >= PREVIEW_REFRESH,
            None => true,
        };
        if stale && self.preview_tx.send(pane.clone()).is_ok() {
            self.last_preview = Some((pane, Instant::now()));
        }
    }

    /// Keeps the selected workspace's content index fresh while you look at it.
    fn refresh_selected_content(&mut self) {
        let Some(hit) = self.selected_hit() else {
            return;
        };
        let (row, id) = (hit.row, self.rows[hit.row].workspace_id.clone());
        let due = match &self.last_content_refresh {
            Some((last, at)) => *last != id || at.elapsed() >= CONTENT_REFRESH,
            None => true,
        };
        if due && self.index_pending == 0 {
            let first_visit = self
                .last_content_refresh
                .as_ref()
                .is_none_or(|(last, _)| *last != id);
            self.last_content_refresh = Some((id, Instant::now()));
            // The initial index pass already covers a freshly selected row.
            if !first_visit {
                self.queue_row_index(row, true);
            }
        }
    }

    fn on_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
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
            KeyCode::Down | KeyCode::Char('j') if alt => self.step_hit(1),
            KeyCode::Up | KeyCode::Char('k') if alt => self.step_hit(-1),
            KeyCode::Char('s') if ctrl => self.toggle_view(),
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
            KeyCode::Char(c) if !ctrl && !alt => {
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
        // Wrap only on single steps; page jumps clamp at the ends.
        let next = if delta.abs() > 1 {
            (cur + delta).clamp(0, len as isize - 1)
        } else {
            (cur + delta).rem_euclid(len as isize)
        };
        self.list.select(Some(next as usize));
        self.reset_hit_view();
    }

    fn step_hit(&mut self, delta: isize) {
        let count = self.selected_hit().map_or(0, |h| h.content.len());
        if count == 0 {
            return;
        }
        self.hit_cursor = (self.hit_cursor as isize + delta).rem_euclid(count as isize) as usize;
        self.view_override = Some(View::Hits);
    }

    fn toggle_view(&mut self) {
        if self.current_content().is_some() {
            self.view_override = Some(match self.view() {
                View::Live => View::Hits,
                View::Hits => View::Live,
            });
        }
    }

    fn accept(&mut self) {
        let Some(row) = self.selected() else {
            return;
        };
        // On a content hit inside an agent pane, land on that agent directly.
        let agent_pane = (self.view() == View::Hits)
            .then(|| self.current_content())
            .flatten()
            .map(|(store, hit, ..)| store.doc(hit.doc).source.pane_id().to_owned())
            .filter(|pane| {
                row.tabs
                    .iter()
                    .flat_map(|t| &t.panes)
                    .any(|p| &p.pane_id == pane && p.agent.is_some())
            });
        let id = row.workspace_id.clone();
        let result = match agent_pane {
            Some(pane) => self
                .client
                .focus_agent(&pane)
                .or_else(|_| self.client.focus_workspace(&id)),
            None => self.client.focus_workspace(&id),
        };
        match result {
            Ok(()) => self.quit = true,
            Err(e) => self.error = Some(e.to_string()),
        }
    }
}
