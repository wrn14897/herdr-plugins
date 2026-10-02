//! Full-text content search over what is inside workspaces: pane scrollback
//! and agent conversation history.
//!
//! Each workspace keeps its documents (screen lines, chat messages) in one
//! contiguous string so a query is a single regex scan, which is what makes
//! searching megabytes of text per keystroke feel instant.

use std::collections::BTreeMap;
use std::ops::Range;

use regex::{Regex, RegexBuilder};

/// Where a document came from. Ordered by how useful a hit is (chat first).
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)] // Chat sources are produced by the transcript readers.
pub enum Source {
    Chat {
        pane_id: String,
        agent: String,
        role: Role,
    },
    Screen {
        pane_id: String,
        pane: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum Role {
    User,
    Assistant,
}

impl Source {
    pub fn pane_id(&self) -> &str {
        match self {
            Self::Chat { pane_id, .. } | Self::Screen { pane_id, .. } => pane_id,
        }
    }

    pub fn is_chat(&self) -> bool {
        matches!(self, Self::Chat { .. })
    }

    /// Short label for list rows and the preview rule, e.g. `chat · you`.
    pub fn badge(&self) -> String {
        match self {
            Self::Chat {
                agent,
                role: Role::User,
                ..
            } => format!("chat · you → {agent}"),
            Self::Chat {
                agent,
                role: Role::Assistant,
                ..
            } => format!("chat · {agent}"),
            Self::Screen { pane, .. } => format!("screen · {pane}"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Doc {
    pub source: Source,
    pub range: Range<usize>,
}

/// One batch of documents from a single origin (one pane's screen, one agent
/// session), replaced wholesale when that origin is re-read.
pub type Segment = Vec<(Source, String)>;

/// All searchable documents of one workspace.
#[derive(Debug, Default)]
pub struct Store {
    segments: BTreeMap<String, Segment>,
    text: String,
    docs: Vec<Doc>,
}

impl Store {
    pub fn set_segment(&mut self, key: String, segment: Segment) {
        if segment.is_empty() {
            self.segments.remove(&key);
        } else {
            self.segments.insert(key, segment);
        }
        self.rebuild();
    }

    fn rebuild(&mut self) {
        self.text.clear();
        self.docs.clear();
        for (source, body) in self.segments.values().flatten() {
            let start = self.text.len();
            self.text.push_str(body);
            self.docs.push(Doc {
                source: source.clone(),
                range: start..self.text.len(),
            });
            // Separator that no term (which never contains whitespace) can span.
            self.text.push('\n');
        }
    }

    pub fn is_empty(&self) -> bool {
        self.docs.is_empty()
    }

    pub fn doc(&self, index: usize) -> &Doc {
        &self.docs[index]
    }

    pub fn doc_text(&self, index: usize) -> &str {
        &self.text[self.docs[index].range.clone()]
    }

    pub fn doc_count(&self) -> usize {
        self.docs.len()
    }

    fn doc_at(&self, pos: usize) -> Option<usize> {
        let i = self.docs.partition_point(|d| d.range.end < pos);
        self.docs
            .get(i)
            .filter(|d| d.range.contains(&pos))
            .map(|_| i)
    }
}

/// A content match: which document, and where in the store's text.
#[derive(Debug, Clone)]
pub struct ContentHit {
    pub doc: usize,
    pub range: Range<usize>,
}

/// Content queries need at least this many non-space characters; shorter
/// queries match nearly every line and only add noise.
pub const MIN_QUERY_CHARS: usize = 3;
const MAX_HITS_PER_WORKSPACE: usize = 500;
const SNIPPET_LEAD: usize = 12;

/// A parsed content query: all words must appear in the same document
/// (case-smart), `'exact phrase` matches literally, `/regex/` is a regex.
#[derive(Debug)]
pub struct Query {
    /// The first pattern drives the scan; the rest must also match the document.
    patterns: Vec<Regex>,
}

impl Query {
    pub fn parse(raw: &str) -> Option<Self> {
        let raw = raw.trim();
        if raw.chars().filter(|c| !c.is_whitespace()).count() < MIN_QUERY_CHARS {
            return None;
        }
        let case_insensitive = !raw.chars().any(char::is_uppercase);
        let build = |pattern: &str| {
            RegexBuilder::new(pattern)
                .case_insensitive(case_insensitive)
                .size_limit(1 << 20)
                .build()
                .ok()
        };

        let mut patterns = if let Some(re) = raw.strip_prefix('/').and_then(|r| r.strip_suffix('/'))
        {
            vec![build(re)?]
        } else if let Some(phrase) = raw.strip_prefix('\'') {
            vec![build(&regex::escape(phrase.trim()))?]
        } else {
            raw.split_whitespace()
                .map(|w| build(&regex::escape(w)))
                .collect::<Option<Vec<_>>>()?
        };
        // Scan with the longest (usually most selective) term.
        patterns.sort_by_key(|p| std::cmp::Reverse(p.as_str().len()));
        Some(Self { patterns })
    }

    /// Matches in `store`, best first: chat before screen, newest first.
    pub fn search(&self, store: &Store) -> Vec<ContentHit> {
        let (scan, rest) = self.patterns.split_first().expect("query has a pattern");
        let mut hits: Vec<ContentHit> = Vec::new();
        for m in scan.find_iter(&store.text) {
            if m.start() == m.end() {
                continue;
            }
            let Some(doc) = store.doc_at(m.start()) else {
                continue;
            };
            let range = &store.docs[doc].range;
            if m.end() > range.end || hits.last().is_some_and(|h| h.doc == doc) {
                continue;
            }
            let body = &store.text[range.clone()];
            if rest.iter().all(|p| p.is_match(body)) {
                hits.push(ContentHit {
                    doc,
                    range: m.range(),
                });
                if hits.len() >= MAX_HITS_PER_WORKSPACE {
                    break;
                }
            }
        }
        hits.sort_by_key(|h| {
            (
                !store.docs[h.doc].source.is_chat(),
                std::cmp::Reverse(h.doc),
            )
        });
        hits
    }

    /// All match ranges of every pattern inside `text`, sorted, for highlighting.
    pub fn highlight_ranges(&self, text: &str) -> Vec<Range<usize>> {
        let mut ranges: Vec<Range<usize>> = self
            .patterns
            .iter()
            .flat_map(|p| p.find_iter(text).map(|m| m.range()))
            .filter(|r| !r.is_empty())
            .collect();
        ranges.sort_by_key(|r| r.start);
        ranges
    }
}

/// A single-line excerpt around a hit, with highlight char positions.
pub fn snippet(
    store: &Store,
    hit: &ContentHit,
    query: &Query,
    width: usize,
) -> (String, Vec<usize>) {
    let doc = &store.docs[hit.doc].range;
    let text = &store.text;
    // The line containing the hit, within the document.
    let line_start = text[doc.start..hit.range.start]
        .rfind('\n')
        .map_or(doc.start, |i| doc.start + i + 1);
    let line_end = text[hit.range.end..doc.end]
        .find('\n')
        .map_or(doc.end, |i| hit.range.end + i);
    let line = &text[line_start..line_end];

    // Window of `width` chars with a little context before the hit, so the hit
    // stays visible even when the list column truncates the snippet.
    let hit_char = line[..hit.range.start - line_start].chars().count();
    let total = line.chars().count();
    let lead = SNIPPET_LEAD.min(width / 4);
    // Never shift the window back to fill it: the row is usually truncated
    // by the list width, so the hit must stay near the start.
    let first = if hit_char <= 2 * lead {
        0
    } else {
        hit_char - lead
    };
    let excerpt: String = line.chars().skip(first).take(width).collect();
    let excerpt = excerpt.trim();

    let mut out = String::new();
    let mut offset = 0;
    if first > 0 {
        out.push('…');
        offset = 1;
    }
    out.push_str(excerpt);
    if first + width < total {
        out.push('…');
    }

    let mut indices = Vec::new();
    for range in query.highlight_ranges(excerpt) {
        let start = excerpt[..range.start].chars().count();
        let len = excerpt[range].chars().count();
        indices.extend((start..start + len).map(|i| i + offset));
    }
    (out, indices)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(pane: &str) -> Source {
        Source::Screen {
            pane_id: pane.into(),
            pane: pane.into(),
        }
    }

    fn chat(role: Role) -> Source {
        Source::Chat {
            pane_id: "p1".into(),
            agent: "claude".into(),
            role,
        }
    }

    fn store() -> Store {
        let mut store = Store::default();
        store.set_segment(
            "screen:p1".into(),
            vec![
                (screen("p1"), "$ cargo test".into()),
                (screen("p1"), "error: login failed for user".into()),
                (screen("p1"), "warning: unused import".into()),
            ],
        );
        store.set_segment(
            "chat:p1".into(),
            vec![
                (
                    chat(Role::User),
                    "Please fix the login bug in auth.rs".into(),
                ),
                (
                    chat(Role::Assistant),
                    "I fixed the Login flow.\nThe bug was a missing await.".into(),
                ),
            ],
        );
        store
    }

    fn texts(store: &Store, hits: &[ContentHit]) -> Vec<String> {
        hits.iter()
            .map(|h| store.text[h.range.clone()].to_owned())
            .collect()
    }

    #[test]
    fn short_queries_are_ignored() {
        assert!(Query::parse("ab").is_none());
        assert!(Query::parse(" a b ").is_none());
        assert!(Query::parse("abc").is_some());
    }

    #[test]
    fn smart_case_terms_chat_first_newest_first() {
        let store = store();
        let hits = Query::parse("login").unwrap().search(&store);
        // Two chat hits (newest first), then the screen hit.
        assert_eq!(texts(&store, &hits), ["Login", "login", "login"]);
        assert!(store.doc(hits[0].doc).source.is_chat());
        assert!(!store.doc(hits[2].doc).source.is_chat());

        let hits = Query::parse("Login").unwrap().search(&store);
        assert_eq!(hits.len(), 1, "uppercase makes the query case-sensitive");
    }

    #[test]
    fn all_terms_must_be_in_the_same_document() {
        let store = store();
        let hits = Query::parse("login await").unwrap().search(&store);
        assert_eq!(hits.len(), 1);
        assert_eq!(store.doc(hits[0].doc).source, chat(Role::Assistant));
        assert!(
            Query::parse("cargo import")
                .unwrap()
                .search(&store)
                .is_empty()
        );
    }

    #[test]
    fn phrase_and_regex_modes() {
        let store = store();
        assert_eq!(Query::parse("'login bug").unwrap().search(&store).len(), 1);
        let hits = Query::parse("/warn(ing)?:/").unwrap().search(&store);
        assert_eq!(texts(&store, &hits), ["warning:"]);
        assert!(
            Query::parse("/(/").is_none(),
            "invalid regex is not a query"
        );
    }

    #[test]
    fn segments_are_replaced() {
        let mut store = store();
        store.set_segment(
            "screen:p1".into(),
            vec![(screen("p1"), "fresh output".into())],
        );
        assert!(Query::parse("cargo").unwrap().search(&store).is_empty());
        assert_eq!(Query::parse("fresh").unwrap().search(&store).len(), 1);
        store.set_segment("chat:p1".into(), Vec::new());
        assert_eq!(store.doc_count(), 1);
    }

    #[test]
    fn snippet_windows_and_highlights() {
        let mut store = Store::default();
        let long = format!("{} needle {}", "x".repeat(100), "y".repeat(100));
        store.set_segment("s".into(), vec![(screen("p"), long)]);
        let query = Query::parse("needle").unwrap();
        let hits = query.search(&store);
        let (text, indices) = snippet(&store, &hits[0], &query, 40);
        assert!(text.starts_with('…') && text.ends_with('…'));
        let highlighted: String = indices
            .iter()
            .map(|&i| text.chars().nth(i).unwrap())
            .collect();
        assert_eq!(highlighted, "needle");
    }

    #[test]
    fn snippet_keeps_late_hits_near_the_start() {
        let mut store = Store::default();
        let line = format!("{} needle tail", "x".repeat(60));
        store.set_segment("s".into(), vec![(screen("p"), line)]);
        let query = Query::parse("needle").unwrap();
        let hits = query.search(&store);
        let (text, indices) = snippet(&store, &hits[0], &query, 120);
        assert!(text.starts_with('…'));
        assert!(indices[0] <= 16, "hit at {} in {text:?}", indices[0]);
    }

    #[test]
    fn snippet_uses_the_matching_line_of_a_message() {
        let store = store();
        let query = Query::parse("await").unwrap();
        let hits = query.search(&store);
        let (text, _) = snippet(&store, &hits[0], &query, 80);
        assert_eq!(text, "The bug was a missing await.");
    }
}
