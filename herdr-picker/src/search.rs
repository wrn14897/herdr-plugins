//! Fuzzy matching over workspace metadata with nucleo (fzf-compatible scoring).
//!
//! Every row matches on one "kind" of thing, and kinds are ranked in tiers:
//! the workspace label first, then names inside it (tabs, panes, agents,
//! terminal titles), then location (path, repo, branch). Within a tier, the
//! fuzzy score decides, and ties keep the natural workspace order.

use nucleo_matcher::pattern::{Atom, AtomKind, CaseMatching, Normalization, Pattern};
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

/// Fuzzy matches must score at least this percentage of a perfect
/// (contiguous) match of the same word; see `atom_indices`.
const MIN_QUALITY_PERCENT: u32 = 60;

/// Where in a row a query word matched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    Label,
    Field(usize),
    Path,
    Branch,
    Repo,
}

impl Target {
    fn tier(self) -> u8 {
        match self {
            Self::Label => 0,
            Self::Field(_) => 1,
            Self::Path | Self::Branch | Self::Repo => 2,
        }
    }
}

/// The last two segments of a displayed path (`repo/worktree`), and their
/// char offset. Fuzzy-matching whole paths lets long prefixes like
/// `~/.herdr/worktrees/` match almost anything.
fn path_tail(path: &str) -> (usize, &str) {
    let trimmed = path.trim_end_matches('/');
    let mut cuts = trimmed.rmatch_indices('/').map(|(i, _)| i);
    let start = match (cuts.next(), cuts.next()) {
        (Some(_), Some(second)) => second + 1,
        _ => 0,
    };
    (path[..start].chars().count(), &trimmed[start..])
}

/// Attaches content matches to metadata hits, adds rows that only match by
/// content, and re-ranks. `store_of` returns the content store for a row.
/// Rows in `excluded` (see [`Searcher::excluded_rows`]) are never added.
pub fn merge_content<'a>(
    hits: &mut Vec<Hit>,
    rows: &'a [Row],
    query: &content::Query,
    excluded: &[bool],
    store_of: impl Fn(&'a Row) -> Option<&'a Store>,
) {
    let mut position: HashMap<usize, usize> =
        hits.iter().enumerate().map(|(i, h)| (h.row, i)).collect();
    for (i, row) in rows.iter().enumerate() {
        if excluded.get(i).copied().unwrap_or(false) {
            continue;
        }
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

    /// Rows whose names contain a `!word` of `query`. Text matches must not
    /// bring these back.
    pub fn excluded_rows(&mut self, query: &str, rows: &[Row]) -> Vec<bool> {
        let pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);
        let negatives: Vec<&Atom> = pattern.atoms.iter().filter(|a| a.negative).collect();
        rows.iter()
            .map(|row| {
                let hay = Utf32Str::new(&row.haystack, &mut self.buf);
                negatives
                    .iter()
                    .any(|a| a.score(hay, &mut self.matcher).is_none())
            })
            .collect()
    }

    fn match_row(&mut self, pattern: &Pattern, i: usize, row: &Row) -> Option<Hit> {
        // Cheap rejection: a word matching some field also matches the joined
        // haystack, so most non-matching rows cost a single score call.
        pattern.score(
            Utf32Str::new(&row.haystack, &mut self.buf),
            &mut self.matcher,
        )?;

        // Every positive word must land inside one field with a good-quality
        // match. Words never match across field boundaries, which is what
        // made long rows match almost any string as a scattered subsequence.
        let (path_offset, path_tail) = path_tail(&row.cwd_display);
        let mut matches: Vec<(Target, u32, Vec<usize>)> = Vec::new();
        for atom in pattern.atoms.iter().filter(|a| !a.negative) {
            let mut best: Option<(Target, u32, Vec<usize>)> = None;
            let mut consider = |target: Target, text: &str, this: &mut Self| {
                let Some(score) = this.atom_indices(atom, text) else {
                    return;
                };
                let better = best.as_ref().is_none_or(|(t, s, _)| {
                    (target.tier(), std::cmp::Reverse(score)) < (t.tier(), std::cmp::Reverse(*s))
                });
                if better {
                    best = Some((target, score, this.positions()));
                }
            };
            consider(Target::Label, &row.label, self);
            for (f, field) in row.fields.iter().enumerate() {
                consider(Target::Field(f), &field.text, self);
            }
            consider(Target::Path, path_tail, self);
            if let Some(branch) = &row.branch {
                consider(Target::Branch, branch, self);
            }
            if let Some(repo) = &row.repo_name {
                consider(Target::Repo, repo, self);
            }
            matches.push(best?);
        }
        if matches.is_empty() {
            // Only negative words: the haystack check above already applied them.
            return Some(Hit::new(i, MatchKind::All, 0));
        }

        let kind_of = |target: Target| match target {
            Target::Label => MatchKind::Label,
            Target::Field(f) => MatchKind::Field(row.fields[f].kind),
            Target::Path => MatchKind::Path,
            Target::Branch => MatchKind::Branch,
            Target::Repo => MatchKind::Repo,
        };
        let primary = matches
            .iter()
            .map(|(t, ..)| *t)
            .min_by_key(|t| t.tier())
            .expect("non-empty");
        let score = matches.iter().map(|(_, s, _)| *s).sum();
        let mut hit = Hit::new(i, kind_of(primary), score);

        // Highlights: label and path in the row itself; the best other field
        // (all words that landed on it) on the detail line.
        let mut detail: Option<(Target, Vec<usize>)> = None;
        for (target, _, positions) in &matches {
            match target {
                Target::Label => hit.label.extend(positions),
                Target::Path => hit.cwd.extend(positions.iter().map(|p| p + path_offset)),
                other => match &mut detail {
                    Some((t, idx)) if t == other => idx.extend(positions),
                    None => detail = Some((*other, positions.clone())),
                    Some(_) => {}
                },
            }
        }
        hit.label.sort_unstable();
        hit.label.dedup();
        hit.cwd.sort_unstable();
        hit.cwd.dedup();
        if let Some((target, mut indices)) = detail {
            indices.sort_unstable();
            indices.dedup();
            let text = match target {
                Target::Field(f) => row.fields[f].text.clone(),
                Target::Branch => row.branch.clone().unwrap_or_default(),
                Target::Repo => row.repo_name.clone().unwrap_or_default(),
                Target::Label | Target::Path => unreachable!("highlighted in the row"),
            };
            // Show which kind of field matched, even when the row's primary
            // match is the label (e.g. "api main" = label + branch).
            let badge = (kind_of(target) != hit.kind).then(|| kind_of(target).badge().to_owned());
            hit.detail = Some(Detail {
                badge,
                text,
                indices,
            });
        }
        Some(hit)
    }

    /// Scores one word against `text`, rejecting scattered low-quality fuzzy
    /// matches. Leaves sorted, deduplicated positions in `self.indices`.
    fn atom_indices(&mut self, atom: &Atom, text: &str) -> Option<u32> {
        self.indices.clear();
        let score = atom.indices(
            Utf32Str::new(text, &mut self.buf),
            &mut self.matcher,
            &mut self.indices,
        )?;
        if atom.kind == AtomKind::Fuzzy {
            let perfect = atom
                .score(atom.needle_text(), &mut self.matcher)
                .unwrap_or(score);
            // Contiguous, prefix, and initials matches score >= ~0.68 of a
            // perfect match; scattered junk like "asd" in a long name <= ~0.53.
            if u32::from(score) * 100 < u32::from(perfect) * MIN_QUALITY_PERCENT {
                return None;
            }
        }
        self.indices.sort_unstable();
        self.indices.dedup();
        Some(u32::from(score))
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
        assert_eq!(hits[0].kind, MatchKind::Label);
        assert_eq!(hits[0].label, [0, 1, 2]);
        let detail = hits[0].detail.as_ref().unwrap();
        assert_eq!(
            (detail.badge.as_deref(), detail.text.as_str()),
            (Some("branch"), "main")
        );
    }

    /// Workspaces from a real session: long worktree paths and titles are
    /// exactly where scattered subsequence matches used to come from.
    fn real_rows() -> Vec<Row> {
        let mut rows = vec![
            row("[1] opencode", "~/Codes/opencode"),
            row("[2] hyperdx-ee", "~/Codes/hyperdx/hyperdx-ee"),
            row("[7] helm-charts", "~/Codes/hyperdx/helm-charts"),
            row("[9] Dotfiles", "~/Codes/Dotfiles"),
            row(
                "[3] warren-query-layer-optimizations-doc",
                "~/.herdr/worktrees/hyperdx-ee/warren-query-layer-optimizations-doc",
            )
            .with_field(
                FieldKind::Pane,
                "warren-query-layer-optimizations-doc › query-layer",
            ),
            row(
                "warren-revisit-claude-bot",
                "~/.herdr/worktrees/hyperdx-ee/warren-revisit-claude-bot",
            )
            .with_field(FieldKind::Agent, "opencode")
            .with_field(FieldKind::Title, "OC | PR #3513 claude bot resolution")
            .with_field(FieldKind::Pane, "opencode-gh"),
            row("herdr-plugins", "~/Codes/herdr-plugins").with_field(
                FieldKind::Title,
                "OC | herdr workspace search + preview plugin",
            ),
        ];
        rows[5].repo_name = Some("hyperdx-ee".into());
        rows[5].set_branch(Some("warren/opencode-gh-action-opus-skills"));
        rows[6].set_branch(Some("main"));
        rows
    }

    #[test]
    fn nonsense_matches_nothing() {
        let rows = real_rows();
        let mut s = Searcher::new();
        // ("eee" is not here: it is a fair match for "hyperdx-ee", as in fzf.)
        for query in ["asdasd", "qwzx", "asd", "ard", "zzz", "xkcd", "jjjj"] {
            let hits = s.search(query, &rows);
            assert!(
                hits.is_empty(),
                "{query:?} matched {:?}",
                hits.iter().map(|h| &rows[h.row].label).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn real_queries_still_find_their_workspace() {
        let rows = real_rows();
        let mut s = Searcher::new();
        let top = |s: &mut Searcher, q: &str| {
            s.search(q, &rows)
                .first()
                .map(|h| rows[h.row].label.clone())
        };
        for (query, expected) in [
            ("revis", "warren-revisit-claude-bot"),
            ("wrcb", "warren-revisit-claude-bot"),
            ("hyp", "[2] hyperdx-ee"),
            ("hdxee", "[2] hyperdx-ee"),
            ("hlmc", "[7] helm-charts"),
            ("dot", "[9] Dotfiles"),
            ("wqlod", "[3] warren-query-layer-optimizations-doc"),
            ("optdoc", "[3] warren-query-layer-optimizations-doc"),
            ("claude bot", "warren-revisit-claude-bot"),
            ("3513", "warren-revisit-claude-bot"),
            ("opus skills", "warren-revisit-claude-bot"),
            ("plugins main", "herdr-plugins"),
        ] {
            assert_eq!(
                top(&mut s, query).as_deref(),
                Some(expected),
                "query {query:?}"
            );
        }
    }

    #[test]
    fn path_matches_only_its_last_segments() {
        assert_eq!(
            path_tail("~/.herdr/worktrees/hyperdx-ee/wip"),
            (19, "hyperdx-ee/wip")
        );
        assert_eq!(path_tail("~/srv"), (0, "~/srv"));
        assert_eq!(path_tail("/"), (0, ""));
        // "worktrees" is in the path prefix, which is not matched.
        let rows = [row("x", "~/.herdr/worktrees/repo/wip")];
        assert!(Searcher::new().search("worktrees", &rows).is_empty());
        let hits = Searcher::new().search("wip", &rows);
        assert_eq!(hits[0].cwd, [24, 25, 26]);
    }

    #[test]
    fn negative_words_exclude_rows() {
        let rows = [row("api-server", "/a"), row("api-client", "/b")];
        let mut s = Searcher::new();
        let hits = s.search("api !client", &rows);
        assert_eq!(rows_of(&hits), [0]);
        assert_eq!(s.excluded_rows("api !client", &rows), [false, true]);
        assert_eq!(s.excluded_rows("api", &rows), [false, false]);
    }
}
