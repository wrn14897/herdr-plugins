//! Fuzzy matching over workspace rows with nucleo (fzf-compatible scoring).

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

use crate::model::Row;

#[derive(Debug, Clone)]
pub struct Hit {
    pub row: usize,
    /// Matched char positions within the label.
    pub label: Vec<usize>,
    /// Matched char positions within the displayed cwd.
    pub cwd: Vec<usize>,
}

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
                .map(|row| Hit {
                    row,
                    label: Vec::new(),
                    cwd: Vec::new(),
                })
                .collect();
        }
        let pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);

        let mut scored: Vec<(u32, Hit)> = rows
            .iter()
            .enumerate()
            .filter_map(|(row, r)| {
                // Cheap rejection first: anything matching a field also matches the
                // full haystack, so most non-matching rows cost a single score call.
                pattern.score(Utf32Str::new(&r.haystack, &mut self.buf), &mut self.matcher)?;
                // Prefer matches inside the label: they are what users type for.
                if let Some(score) = self.indices_of(&pattern, &r.label) {
                    let label = self.indices.iter().map(|&i| i as usize).collect();
                    return Some((
                        score.saturating_mul(2),
                        Hit {
                            row,
                            label,
                            cwd: Vec::new(),
                        },
                    ));
                }
                let score = self.indices_of(&pattern, &r.haystack)?;

                let label_len = r.label.chars().count();
                let cwd_start = label_len + 1;
                let cwd_end = cwd_start + r.cwd_display.chars().count();
                let mut hit = Hit {
                    row,
                    label: Vec::new(),
                    cwd: Vec::new(),
                };
                for &i in &self.indices {
                    let i = i as usize;
                    if i < label_len {
                        hit.label.push(i);
                    } else if (cwd_start..cwd_end).contains(&i) {
                        hit.cwd.push(i - cwd_start);
                    }
                }
                Some((score, hit))
            })
            .collect();

        // Stable sort keeps the natural (number) order among equal scores.
        scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
        scored.into_iter().map(|(_, hit)| hit).collect()
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(label: &str, cwd: &str) -> Row {
        row_with(label, cwd, None)
    }

    fn row_with(label: &str, cwd: &str, branch: Option<&str>) -> Row {
        let mut row = Row {
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
            haystack: String::new(),
        };
        row.refresh_haystack(branch);
        row
    }

    #[test]
    fn empty_query_keeps_order() {
        let rows = [row("a", "/x"), row("b", "/y")];
        let hits = Searcher::new().search("", &rows);
        assert_eq!(hits.iter().map(|h| h.row).collect::<Vec<_>>(), [0, 1]);
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
        assert_eq!(hits[0].label, [0, 1, 2]);

        let hits = s.search("srv", &rows);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].row, 2);
        assert_eq!(hits[0].cwd, [2, 3, 4]);
    }

    #[test]
    fn branch_is_searchable() {
        let rows = [row("a", "/x"), row_with("b", "/y", Some("feature/login"))];
        let hits = Searcher::new().search("feature", &rows);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].row, 1);
    }
}
