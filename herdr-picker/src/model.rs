//! Snapshot types and the flattened workspace rows the picker displays.

use std::collections::HashMap;

use serde::Deserialize;

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
    pub workspace_id: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub agent_status: String,
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
    pub label: String,
    pub agent: Option<String>,
    pub status: String,
    pub focused: bool,
}

/// Flattens a snapshot into one row per workspace, ordered by workspace number
/// with the currently focused workspace last, so the initial selection is the
/// most likely switch target.
pub fn build_rows(snapshot: &Snapshot) -> Vec<Row> {
    let home = std::env::var("HOME").unwrap_or_default();
    let focused_tab_pane: HashMap<&str, &str> = snapshot
        .layouts
        .iter()
        .filter_map(|l| Some((l.tab_id.as_str(), l.focused_pane_id.as_deref()?)))
        .collect();
    let focused_ws = snapshot.focused_workspace_id.as_deref();

    let mut rows: Vec<Row> = snapshot
        .workspaces
        .iter()
        .map(|ws| {
            let mut tabs: Vec<&TabInfo> = snapshot
                .tabs
                .iter()
                .filter(|t| t.workspace_id == ws.workspace_id)
                .collect();
            tabs.sort_by_key(|t| t.number);

            let active_tab = ws
                .active_tab_id
                .clone()
                .or_else(|| tabs.first().map(|t| t.tab_id.clone()));
            let active_pane_id = active_tab.as_deref().and_then(|tab| {
                focused_tab_pane
                    .get(tab)
                    .map(|p| (*p).to_owned())
                    .or_else(|| {
                        snapshot
                            .panes
                            .iter()
                            .find(|p| p.tab_id == tab)
                            .map(|p| p.pane_id.clone())
                    })
            });

            let cwd = active_pane_id
                .as_deref()
                .and_then(|id| snapshot.panes.iter().find(|p| p.pane_id == id))
                .or_else(|| {
                    snapshot
                        .panes
                        .iter()
                        .find(|p| p.workspace_id == ws.workspace_id)
                })
                .and_then(|p| p.cwd.clone())
                .unwrap_or_default();

            let tab_rows = tabs
                .iter()
                .map(|t| {
                    let focused_pane = focused_tab_pane.get(t.tab_id.as_str()).copied();
                    TabRow {
                        label: t.label.clone(),
                        status: t.agent_status.clone(),
                        active: active_tab.as_deref() == Some(t.tab_id.as_str()),
                        panes: snapshot
                            .panes
                            .iter()
                            .filter(|p| p.tab_id == t.tab_id)
                            .map(|p| PaneRow {
                                label: p.label.clone().unwrap_or_else(|| p.pane_id.clone()),
                                agent: p.agent.clone(),
                                status: p.agent_status.clone(),
                                focused: focused_pane == Some(p.pane_id.as_str()),
                            })
                            .collect(),
                    }
                })
                .collect();

            Row {
                workspace_id: ws.workspace_id.clone(),
                number: ws.number,
                label: ws.label.clone(),
                status: ws.agent_status.clone(),
                focused: ws.focused || focused_ws == Some(ws.workspace_id.as_str()),
                cwd_display: tildify(&cwd, &home),
                cwd,
                repo_name: ws.worktree.as_ref().and_then(|w| w.repo_name.clone()),
                linked_worktree: ws.worktree.as_ref().is_some_and(|w| w.is_linked_worktree),
                active_pane_id,
                tabs: tab_rows,
            }
        })
        .collect();

    rows.sort_by_key(|r| (r.focused, r.number));
    rows
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
    fn tildify_only_replaces_home_prefix() {
        assert_eq!(tildify("/home/u", "/home/u"), "~");
        assert_eq!(tildify("/home/u/x", "/home/u"), "~/x");
        assert_eq!(tildify("/home/user2", "/home/u"), "/home/user2");
    }
}
