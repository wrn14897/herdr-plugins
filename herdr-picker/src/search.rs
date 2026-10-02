//! Fuzzy matching over workspace metadata with nucleo (fzf-compatible scoring).
//!
//! Every row matches on one "kind" of thing, and kinds are ranked in tiers:
//! the workspace label first, then names inside it (tabs, panes, agents,
//! terminal titles), then location (path, repo, branch). Within a tier, the
//! fuzzy score decides, and ties keep the natural workspace order.

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

use std::collections::HashMap;

use crate::content::{self, ContentHit, Store};
use crate::model::{FieldKind, Row};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchKind {
    /// Empty query: every row, no highlighting.
    All,
    Label,
    Field(FieldKind),
    Path,
    Repo,
    Branch,
    /// Matched only inside agent conversation history.
    Chat,
    /// Matched only inside pane scrollback.
    Screen,
}

impl MatchKind {
    pub fn tier(self) -> u8 {
        match self {
            Self::All | Self::Label => 0,
            Self::Field(_) => 1,
            Self::Path | Self::Repo | Self::Branch => 2,
            Self::Chat => 3,
            Self::Screen => 4,
        }
    }

    pub fn is_content(self) -> bool {
        matches!(self, Self::Chat | Self::Screen)
    }

    pub fn badge(self) -> &'static str {
        match self {
            Self::All | Self::Label => "label",
            Self::Field(FieldKind::Tab) => "tab",
            Self::Field(FieldKind::Pane) => "pane",
            Self::Field(FieldKind::Agent) => "agent",
            Self::Field(FieldKind::Title) => "title",
            Self::Path => "path",
            Self::Repo => "repo",
            Self::Branch => "branch",
            Self::Chat => "chat",
            Self::Screen => "screen",
        }
    }
}

/// A match shown on a second line under the row, e.g. a matching tab name.
#[derive(Debug, Clone)]
pub struct Detail {
    /// Overrides the kind's badge, e.g. `chat · claude`.
    pub badge: Option<String>,
    pub text: String,
    /// Matched char positions within `text`.
    pub indices: Vec<usize>,
}

#[derive(Debug, Clone)]
pub struct Hit {
    pub row: usize,
    pub kind: MatchKind,
    pub score: u32,
    /// Matched char positions within the label.
    pub label: Vec<usize>,
    /// Matched char positions within the displayed cwd.
    pub cwd: Vec<usize>,
    pub detail: Option<Detail>,
    /// Content matches inside this workspace, best first.
    pub content: Vec<ContentHit>,
}

impl Hit {
    pub fn new(row: usize, kind: MatchKind, score: u32) -> Self {
        Self {
            row,
            kind,
            score,
            label: Vec::new(),
            cwd: Vec::new(),
            detail: None,
            content: Vec::new(),
        }
    }
}

/// Attaches content matches to metadata hits, adds rows that only match by
/// content, and re-ranks. `store_of` returns the content store for a row.
pub fn merge_content<'a>(
    hits: &mut Vec<Hit>,
    rows: &'a [Row],
    query: &content::Query,
    store_of: impl Fn(&'a Row) -> Option<&'a Store>,
) {
    let mut position: HashMap<usize, usize> =
        hits.iter().enumerate().map(|(i, h)| (h.row, i)).collect();
    for (i, row) in rows.iter().enumerate() {
        let Some(store) = store_of(row).filter(|s| !s.is_empty()) else {
            continue;
        };
        let found = query.search(store);
        let Some(best) = found.first() else { continue };
        if let Some(&at) = position.get(&i) {
            hits[at].content = found;
            continue;
        }
        let source = &store.doc(best.doc).source;
        let kind = if source.is_chat() {
            MatchKind::Chat
        } else {
            MatchKind::Screen
        };
        let (text, indices) = content::snippet(store, best, query, SNIPPET_CHARS);
        let mut hit = Hit::new(i, kind, u32::try_from(found.len()).unwrap_or(u32::MAX));
        hit.detail = Some(Detail {
            badge: Some(source.badge()),
            text,
            indices,
        });
        hit.content = found;
        position.insert(i, hits.len());
        hits.push(hit);
    }
    // Content matches in the current workspace (often your own session echoing
    // the query) rank after other workspaces, like the default list order.
    hits.sort_by_key(|h| {
        let focused = h.kind.is_content() && rows[h.row].focused;
        (h.kind.tier(), focused, std::cmp::Reverse(h.score))
    });
}

const SNIPPET_CHARS: usize = 120;

#[derive(Debug)]
pub struct Searcher {
    matcher: Matcher,
    buf: Vec<char>,
    indices: Vec<u32>,
}

impl Searcher {
    pub fn new() -> Self {
        Self {
            matcher: Matcher::new(Config::DEFAULT),
            buf: Vec::new(),
            indices: Vec::new(),
        }
    }

    /// Returns matching rows, best first. An empty query keeps the input order.
    pub fn search(&mut self, query: &str, rows: &[Row]) -> Vec<Hit> {
        if query.trim().is_empty() {
            return (0..rows.len())
                .map(|row| Hit::new(row, MatchKind::All, 0))
                .collect();
        }
        let pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);
        let mut hits: Vec<Hit> = rows
            .iter()
            .enumerate()
            .filter_map(|(i, row)| self.match_row(&pattern, i, row))
            .collect();
        // Stable sort keeps the natural (number) order among equal ranks.
        hits.sort_by_key(|h| (h.kind.tier(), std::cmp::Reverse(h.score)));
        hits
    }

    fn match_row(&mut self, pattern: &Pattern, i: usize, row: &Row) -> Option<Hit> {
        // Cheap rejection: anything matching one field also matches the joined
        // haystack, so most non-matching rows cost a single score call.
        let full_score = pattern.score(
            Utf32Str::new(&row.haystack, &mut self.buf),
            &mut self.matcher,
        )?;

        if let Some(score) = self.indices_of(pattern, &row.label) {
            let mut hit = Hit::new(i, MatchKind::Label, score);
            hit.label = self.positions();
            return Some(hit);
        }

        let mut best: Option<Hit> = None;
        for field in &row.fields {
            if let Some(score) = self.indices_of(pattern, &field.text)
                && best.as_ref().is_none_or(|b| score > b.score)
            {
                let mut hit = Hit::new(i, MatchKind::Field(field.kind), score);
                hit.detail = Some(Detail {
                    badge: None,
                    text: field.text.clone(),
                    indices: self.positions(),
                });
                best = Some(hit);
            }
        }
        if best.is_some() {
            return best;
        }

        if let Some(score) = self.indices_of(pattern, &row.cwd_display) {
            let mut hit = Hit::new(i, MatchKind::Path, score);
            hit.cwd = self.positions();
            return Some(hit);
        }
        for (kind, text) in [
            (MatchKind::Branch, &row.branch),
            (MatchKind::Repo, &row.repo_name),
        ] {
            if let Some(text) = text
                && let Some(score) = self.indices_of(pattern, text)
            {
                let mut hit = Hit::new(i, kind, score);
                hit.detail = Some(Detail {
                    badge: None,
                    text: text.clone(),
                    indices: self.positions(),
                });
                return Some(hit);
            }
        }

        // Matched only across fields (e.g. "api main" = label + branch):
        // highlight whatever landed on the label and path.
        self.indices_of(pattern, &row.haystack)?;
        let mut hit = Hit::new(i, MatchKind::Path, full_score);
        let label_len = row.label.chars().count();
        let cwd_start = label_len + 1;
        let cwd_end = cwd_start + row.cwd_display.chars().count();
        for pos in self.positions() {
            if pos < label_len {
                hit.label.push(pos);
            } else if (cwd_start..cwd_end).contains(&pos) {
                hit.cwd.push(pos - cwd_start);
            }
        }
        Some(hit)
    }

    /// Scores `haystack`, leaving sorted, deduplicated match positions in `self.indices`.
    fn indices_of(&mut self, pattern: &Pattern, haystack: &str) -> Option<u32> {
        self.indices.clear();
        let score = pattern.indices(
            Utf32Str::new(haystack, &mut self.buf),
            &mut self.matcher,
            &mut self.indices,
        )?;
        self.indices.sort_unstable();
        self.indices.dedup();
        Some(score)
    }

    fn positions(&self) -> Vec<usize> {
        self.indices.iter().map(|&i| i as usize).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Field;

    fn row(label: &str, cwd: &str) -> Row {
        Row {
            workspace_id: label.into(),
            number: 0,
            label: label.into(),
            status: String::new(),
            focused: false,
            cwd: cwd.into(),
            cwd_display: cwd.into(),
            repo_name: None,
            linked_worktree: false,
            active_pane_id: None,
            tabs: Vec::new(),
            branch: None,
            fields: Vec::new(),
            haystack: String::new(),
        }
        .finish()
    }

    impl Row {
        fn finish(mut self) -> Self {
            self.refresh_haystack();
            self
        }
        fn with_field(mut self, kind: FieldKind, text: &str) -> Self {
            self.fields.push(Field {
                kind,
                text: text.into(),
            });
            self.finish()
        }
    }

    fn rows_of(hits: &[Hit]) -> Vec<usize> {
        hits.iter().map(|h| h.row).collect()
    }

    #[test]
    fn empty_query_keeps_order() {
        let rows = [row("a", "/x"), row("b", "/y")];
        assert_eq!(rows_of(&Searcher::new().search("", &rows)), [0, 1]);
    }

    #[test]
    fn ranks_and_highlights() {
        let rows = [
            row("hyperdx-ee", "~/Codes/hyperdx"),
            row("dotfiles", "~/Codes/Dotfiles"),
            row("api", "~/srv"),
        ];
        let mut s = Searcher::new();
        let hits = s.search("dot", &rows);
        assert_eq!(hits[0].row, 1);
        assert_eq!(hits[0].kind, MatchKind::Label);
        assert_eq!(hits[0].label, [0, 1, 2]);

        let hits = s.search("srv", &rows);
        assert_eq!(rows_of(&hits), [2]);
        assert_eq!(hits[0].kind, MatchKind::Path);
        assert_eq!(hits[0].cwd, [2, 3, 4]);
    }

    #[test]
    fn label_outranks_fields_which_outrank_paths() {
        let rows = [
            row("zeta", "~/login"),
            row("alpha", "/x").with_field(FieldKind::Title, "fix login bug"),
            row("login-service", "/y"),
        ];
        let hits = Searcher::new().search("login", &rows);
        assert_eq!(rows_of(&hits), [2, 1, 0]);
        let field = &hits[1];
        assert_eq!(field.kind, MatchKind::Field(FieldKind::Title));
        let detail = field.detail.as_ref().unwrap();
        assert_eq!(detail.text, "fix login bug");
        assert_eq!(detail.indices, [4, 5, 6, 7, 8]);
    }

    #[test]
    fn branch_is_searchable() {
        let mut b = row("b", "/y");
        b.set_branch(Some("feature/login"));
        let rows = [row("a", "/x"), b];
        let hits = Searcher::new().search("feature", &rows);
        assert_eq!(rows_of(&hits), [1]);
        assert_eq!(hits[0].kind, MatchKind::Branch);
    }

    #[test]
    fn cross_field_queries_still_match() {
        let mut r = row("api", "/srv/api");
        r.set_branch(Some("main"));
        let hits = Searcher::new().search("api main", &[r]);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].label, [0, 1, 2]);
    }
}
