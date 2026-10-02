//! Scale tests: a synthetic session with thousands of workspaces must stay fast.

use std::fmt::Write as _;
use std::time::{Duration, Instant};

use crate::model::{self, Snapshot};
use crate::search::Searcher;

/// Builds a snapshot with `n` workspaces, 3 tabs each, 2 panes per tab.
pub fn synthetic_snapshot(n: usize) -> Snapshot {
    let mut ws = String::new();
    let mut tabs = String::new();
    let mut panes = String::new();
    let mut layouts = String::new();
    for w in 0..n {
        let sep = if w == 0 { "" } else { "," };
        let _ = write!(
            ws,
            r#"{sep}{{"workspace_id":"w{w}","label":"project-{w}-service","number":{w},"agent_status":"idle","focused":{},"active_tab_id":"w{w}:t0"}}"#,
            w == n / 2
        );
        for t in 0..3 {
            let tsep = if w == 0 && t == 0 { "" } else { "," };
            let _ = write!(
                tabs,
                r#"{tsep}{{"tab_id":"w{w}:t{t}","workspace_id":"w{w}","label":"tab {t}","number":{t},"agent_status":"idle"}}"#
            );
            let _ = write!(
                layouts,
                r#"{tsep}{{"tab_id":"w{w}:t{t}","focused_pane_id":"w{w}:p{t}0"}}"#
            );
            for p in 0..2 {
                let psep = if w == 0 && t == 0 && p == 0 { "" } else { "," };
                let _ = write!(
                    panes,
                    r#"{psep}{{"pane_id":"w{w}:p{t}{p}","tab_id":"w{w}:t{t}","workspace_id":"w{w}","cwd":"/home/u/code/org-{}/project-{w}","agent":"claude","agent_status":"idle"}}"#,
                    w % 37
                );
            }
        }
    }
    let json = format!(
        r#"{{"focused_workspace_id":"w{}","workspaces":[{ws}],"tabs":[{tabs}],"panes":[{panes}],"layouts":[{layouts}]}}"#,
        n / 2
    );
    serde_json::from_str(&json).expect("synthetic snapshot parses")
}

/// Generous in debug builds; the release numbers are the real budget.
fn budget(release_ms: u64) -> Duration {
    Duration::from_millis(if cfg!(debug_assertions) {
        release_ms * 20
    } else {
        release_ms
    })
}

#[test]
fn five_thousand_workspaces_stay_fast() {
    let snapshot = synthetic_snapshot(5_000);

    let started = Instant::now();
    let rows = model::build_rows(&snapshot);
    let build = started.elapsed();
    assert_eq!(rows.len(), 5_000);
    assert_eq!(
        rows.last().unwrap().workspace_id,
        "w2500",
        "focused workspace sorts last"
    );

    let mut searcher = Searcher::new();
    let started = Instant::now();
    let mut total = 0;
    for query in ["p", "pr", "pro", "proj", "proj 42", "org-3 service", "zzz"] {
        total += searcher.search(query, &rows).len();
    }
    let per_query = started.elapsed() / 7;
    assert!(total > 0);

    eprintln!("5k workspaces: build_rows {build:?}, search {per_query:?}/query");
    assert!(build < budget(50), "build_rows took {build:?}");
    assert!(
        per_query < budget(50),
        "search took {per_query:?} per query"
    );
}
