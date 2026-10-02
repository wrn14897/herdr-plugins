//! Snapshot types and the flattened workspace rows the picker displays.

use std::collections::HashMap;

use serde::Deserialize;

use crate::transcript::SessionRef;

#[derive(Debug, Default, Deserialize)]
pub struct Snapshot {
    #[serde(default)]
    pub focused_workspace_id: Option<String>,
    #[serde(default)]
    pub workspaces: Vec<WorkspaceInfo>,
    #[serde(default)]
    pub tabs: Vec<TabInfo>,
    #[serde(default)]
    pub panes: Vec<PaneInfo>,
    #[serde(default)]
    pub layouts: Vec<LayoutInfo>,
}

#[derive(Debug, Default, Deserialize)]
pub struct WorkspaceInfo {
    pub workspace_id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub number: u32,
    #[serde(default)]
    pub agent_status: String,
    #[serde(default)]
    pub focused: bool,
    #[serde(default)]
    pub active_tab_id: Option<String>,
    #[serde(default)]
    pub worktree: Option<Worktree>,
}

#[derive(Debug, Default, Clone, Deserialize)]
pub struct Worktree {
    #[serde(default)]
    pub repo_name: Option<String>,
    #[serde(default)]
    pub is_linked_worktree: bool,
}

#[derive(Debug, Default, Deserialize)]
pub struct TabInfo {
    pub tab_id: String,
    pub workspace_id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub number: u32,
    #[serde(default)]
    pub agent_status: String,
}

#[derive(Debug, Default, Deserialize)]
pub struct PaneInfo {
    pub pane_id: String,
    pub tab_id: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub agent_status: String,
    #[serde(default)]
    pub terminal_title_stripped: Option<String>,
    #[serde(default)]
    pub agent_session: Option<SessionRef>,
}

#[derive(Debug, Default, Deserialize)]
pub struct LayoutInfo {
    pub tab_id: String,
    #[serde(default)]
    pub focused_pane_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Row {
    pub workspace_id: String,
    pub number: u32,
    pub label: String,
    pub status: String,
    pub focused: bool,
    pub cwd: String,
    pub cwd_display: String,
    pub repo_name: Option<String>,
    pub linked_worktree: bool,
    pub active_pane_id: Option<String>,
    pub tabs: Vec<TabRow>,
    pub branch: Option<String>,
    /// Secondary searchable names: tabs, panes, agents, terminal titles.
    pub fields: Vec<Field>,
    /// Precomputed metadata search text; see [`Row::refresh_haystack`].
    pub haystack: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    Tab,
    Pane,
    Agent,
    Title,
}

#[derive(Debug, Clone)]
pub struct Field {
    pub kind: FieldKind,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct TabRow {
    pub label: String,
    pub status: String,
    pub active: bool,
    pub panes: Vec<PaneRow>,
}

#[derive(Debug, Clone)]
pub struct PaneRow {
    pub pane_id: String,
    pub label: String,
    pub agent: Option<String>,
    pub status: String,
    pub focused: bool,
    pub cwd: String,
    pub session: Option<SessionRef>,
}

impl PaneRow {
    /// A compact name for badges: the agent kind, or the pane label trimmed.
    pub fn short_name(&self) -> String {
        const MAX: usize = 18;
        if let Some(agent) = &self.agent {
            return agent.clone();
        }
        // Pane labels look like `name › terminal title`; the first part names it.
        let name = self.label.split(" › ").next().unwrap_or(&self.label).trim();
        if name.chars().count() > MAX {
            format!("{}…", name.chars().take(MAX - 1).collect::<String>())
        } else {
            name.to_owned()
        }
    }
}

/// Flattens a snapshot into one row per workspace, ordered by workspace number
/// with the currently focused workspace last, so the initial selection is the
/// most likely switch target. Linear in the size of the snapshot.
pub fn build_rows(snapshot: &Snapshot) -> Vec<Row> {
    let home = std::env::var("HOME").unwrap_or_default();
    let focused_ws = snapshot.focused_workspace_id.as_deref();

    let focused_pane_of_tab: HashMap<&str, &str> = snapshot
        .layouts
        .iter()
        .filter_map(|l| Some((l.tab_id.as_str(), l.focused_pane_id.as_deref()?)))
        .collect();
    let pane_by_id: HashMap<&str, &PaneInfo> = snapshot
        .panes
        .iter()
        .map(|p| (p.pane_id.as_str(), p))
        .collect();
    let mut panes_by_tab: HashMap<&str, Vec<&PaneInfo>> = HashMap::new();
    for pane in &snapshot.panes {
        panes_by_tab
            .entry(pane.tab_id.as_str())
            .or_default()
            .push(pane);
    }
    let mut tabs_by_ws: HashMap<&str, Vec<&TabInfo>> = HashMap::new();
    for tab in &snapshot.tabs {
        tabs_by_ws
            .entry(tab.workspace_id.as_str())
            .or_default()
            .push(tab);
    }

    let mut rows: Vec<Row> = snapshot
        .workspaces
        .iter()
        .map(|ws| {
            let mut tabs = tabs_by_ws
                .remove(ws.workspace_id.as_str())
                .unwrap_or_default();
            tabs.sort_by_key(|t| t.number);

            let active_tab = ws
                .active_tab_id
                .clone()
                .or_else(|| tabs.first().map(|t| t.tab_id.clone()));
            let tab_panes = |tab: &str| panes_by_tab.get(tab).map_or(&[][..], Vec::as_slice);
            let active_pane = active_tab.as_deref().and_then(|tab| {
                focused_pane_of_tab
                    .get(tab)
                    .and_then(|id| pane_by_id.get(id).copied())
                    .or_else(|| tab_panes(tab).first().copied())
            });
            let cwd = active_pane
                .or_else(|| {
                    tabs.iter()
                        .find_map(|t| tab_panes(&t.tab_id).first().copied())
                })
                .and_then(|p| p.cwd.clone())
                .unwrap_or_default();

            let tab_rows = tabs
                .iter()
                .map(|t| {
                    let focused_pane = focused_pane_of_tab.get(t.tab_id.as_str()).copied();
                    TabRow {
                        label: t.label.clone(),
                        status: t.agent_status.clone(),
                        active: active_tab.as_deref() == Some(t.tab_id.as_str()),
                        panes: tab_panes(&t.tab_id)
                            .iter()
                            .map(|p| PaneRow {
                                pane_id: p.pane_id.clone(),
                                label: p.label.clone().unwrap_or_else(|| p.pane_id.clone()),
                                agent: p.agent.clone(),
                                status: p.agent_status.clone(),
                                focused: focused_pane == Some(p.pane_id.as_str()),
                                cwd: p.cwd.clone().unwrap_or_default(),
                                session: p.agent_session.clone(),
                            })
                            .collect(),
                    }
                })
                .collect();

            let mut row = Row {
                workspace_id: ws.workspace_id.clone(),
                number: ws.number,
                label: ws.label.clone(),
                status: ws.agent_status.clone(),
                focused: ws.focused || focused_ws == Some(ws.workspace_id.as_str()),
                cwd_display: tildify(&cwd, &home),
                cwd,
                repo_name: ws.worktree.as_ref().and_then(|w| w.repo_name.clone()),
                linked_worktree: ws.worktree.as_ref().is_some_and(|w| w.is_linked_worktree),
                active_pane_id: active_pane.map(|p| p.pane_id.clone()),
                tabs: tab_rows,
                branch: None,
                fields: Vec::new(),
                haystack: String::new(),
            };
            row.fields = collect_fields(&row.label, &tabs, |tab| tab_panes(tab));
            row.refresh_haystack();
            row
        })
        .collect();

    rows.sort_by_key(|r| (r.focused, r.number));
    rows
}

/// Distinct, meaningful names inside a workspace. Shell window titles such as
/// `user@host:~/dir` only repeat the cwd, so they are skipped.
fn collect_fields<'a>(
    label: &str,
    tabs: &[&TabInfo],
    tab_panes: impl Fn(&str) -> &'a [&'a PaneInfo],
) -> Vec<Field> {
    let mut fields: Vec<Field> = Vec::new();
    let mut push = |kind, text: &str| {
        let text = text.trim();
        if !text.is_empty() && text != label && !fields.iter().any(|f| f.text == text) {
            fields.push(Field {
                kind,
                text: text.to_owned(),
            });
        }
    };
    for tab in tabs {
        push(FieldKind::Tab, &tab.label);
        for pane in tab_panes(&tab.tab_id) {
            if let Some(agent) = &pane.agent {
                push(FieldKind::Agent, agent);
            }
            if let Some(pane_label) = &pane.label {
                push(FieldKind::Pane, pane_label);
            }
            if let Some(title) = &pane.terminal_title_stripped
                && !is_shell_title(title)
            {
                push(FieldKind::Title, title);
            }
        }
    }
    fields
}

fn is_shell_title(title: &str) -> bool {
    title
        .split_once(':')
        .is_some_and(|(user_host, _)| user_host.contains('@') && !user_host.contains(' '))
}

impl Row {
    /// Panes worth indexing for content search: every agent pane, plus the
    /// pane you would land on.
    pub fn indexed_panes(&self) -> impl Iterator<Item = &PaneRow> {
        self.tabs.iter().flat_map(|t| &t.panes).filter(|p| {
            p.agent.is_some() || self.active_pane_id.as_deref() == Some(p.pane_id.as_str())
        })
    }

    pub fn set_branch(&mut self, branch: Option<&str>) {
        self.branch = branch.map(str::to_owned);
        self.refresh_haystack();
    }

    /// Rebuilds the cached fuzzy-search text. The label and displayed cwd come
    /// first so match positions map straight back onto the rendered row.
    pub fn refresh_haystack(&mut self) {
        let mut hay = format!("{} {}", self.label, self.cwd_display);
        for extra in [self.repo_name.as_deref(), self.branch.as_deref()]
            .into_iter()
            .flatten()
        {
            hay.push(' ');
            hay.push_str(extra);
        }
        for field in &self.fields {
            hay.push(' ');
            hay.push_str(&field.text);
        }
        self.haystack = hay;
    }
}

pub fn tildify(path: &str, home: &str) -> String {
    if home.is_empty() {
        return path.to_owned();
    }
    match path.strip_prefix(home) {
        Some("") => "~".to_owned(),
        Some(rest) if rest.starts_with('/') => format!("~{rest}"),
        _ => path.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"{
      "focused_workspace_id": "w2",
      "workspaces": [
        {"workspace_id":"w2","label":"api","number":2,"agent_status":"working","focused":true,"active_tab_id":"w2:t1"},
        {"workspace_id":"w1","label":"web","number":1,"agent_status":"idle","focused":false,"active_tab_id":"w1:t2",
         "worktree":{"repo_name":"web","is_linked_worktree":true}},
        {"workspace_id":"w3","label":"docs","number":3,"agent_status":"unknown","focused":false}
      ],
      "tabs": [
        {"tab_id":"w1:t1","workspace_id":"w1","label":"one","number":1,"agent_status":"unknown"},
        {"tab_id":"w1:t2","workspace_id":"w1","label":"two","number":2,"agent_status":"idle"},
        {"tab_id":"w2:t1","workspace_id":"w2","label":"main","number":1,"agent_status":"working"},
        {"tab_id":"w3:t1","workspace_id":"w3","label":"main","number":1,"agent_status":"unknown"}
      ],
      "panes": [
        {"pane_id":"w1:p1","tab_id":"w1:t1","workspace_id":"w1","cwd":"/home/u/web"},
        {"pane_id":"w1:p2","tab_id":"w1:t2","workspace_id":"w1","cwd":"/home/u/web/sub","agent":"claude","agent_status":"idle"},
        {"pane_id":"w1:p3","tab_id":"w1:t2","workspace_id":"w1","cwd":"/home/u/web"},
        {"pane_id":"w2:p1","tab_id":"w2:t1","workspace_id":"w2","cwd":"/srv/api"},
        {"pane_id":"w3:p1","tab_id":"w3:t1","workspace_id":"w3","cwd":"/home/u"}
      ],
      "layouts": [
        {"tab_id":"w1:t2","focused_pane_id":"w1:p3"},
        {"tab_id":"w2:t1","focused_pane_id":"w2:p1"}
      ]
    }"#;

    fn rows() -> Vec<Row> {
        let snapshot: Snapshot = serde_json::from_str(FIXTURE).unwrap();
        build_rows(&snapshot)
    }

    #[test]
    fn orders_by_number_with_focused_last() {
        let ids: Vec<_> = rows().into_iter().map(|r| r.workspace_id).collect();
        assert_eq!(ids, ["w1", "w3", "w2"]);
    }

    #[test]
    fn active_pane_comes_from_active_tab_layout() {
        let rows = rows();
        assert_eq!(rows[0].active_pane_id.as_deref(), Some("w1:p3"));
        assert_eq!(rows[0].cwd, "/home/u/web");
        assert!(rows[0].linked_worktree);
        // No active tab or layout: fall back to the first tab's first pane.
        assert_eq!(rows[1].active_pane_id.as_deref(), Some("w3:p1"));
    }

    #[test]
    fn tabs_are_sorted_and_marked() {
        let rows = rows();
        let tabs = &rows[0].tabs;
        assert_eq!(tabs.len(), 2);
        assert!(!tabs[0].active && tabs[1].active);
        assert_eq!(tabs[1].panes.len(), 2);
        assert_eq!(tabs[1].panes[0].agent.as_deref(), Some("claude"));
        assert!(tabs[1].panes[1].focused);
    }

    #[test]
    fn collects_distinct_fields_and_skips_shell_titles() {
        let snapshot: Snapshot = serde_json::from_str(
            r#"{
              "workspaces":[{"workspace_id":"w1","label":"api","number":1,"active_tab_id":"w1:t1"}],
              "tabs":[{"tab_id":"w1:t1","workspace_id":"w1","label":"api","number":1},
                      {"tab_id":"w1:t2","workspace_id":"w1","label":"logs","number":2}],
              "panes":[
                {"pane_id":"w1:p1","tab_id":"w1:t1","label":"opencode","agent":"opencode",
                 "terminal_title_stripped":"OC | fix login bug"},
                {"pane_id":"w1:p2","tab_id":"w1:t2","label":"logs",
                 "terminal_title_stripped":"u@host:~/api"}
              ]
            }"#,
        )
        .unwrap();
        let row = &build_rows(&snapshot)[0];
        let fields: Vec<_> = row
            .fields
            .iter()
            .map(|f| (f.kind, f.text.as_str()))
            .collect();
        assert_eq!(
            fields,
            [
                (FieldKind::Agent, "opencode"),
                (FieldKind::Title, "OC | fix login bug"),
                (FieldKind::Tab, "logs"),
            ]
        );
        assert!(row.haystack.contains("fix login bug"));
    }

    #[test]
    fn tildify_only_replaces_home_prefix() {
        assert_eq!(tildify("/home/u", "/home/u"), "~");
        assert_eq!(tildify("/home/u/x", "/home/u"), "~/x");
        assert_eq!(tildify("/home/user2", "/home/u"), "/home/user2");
    }
}
