//! Rendering: search bar on top, workspace list on the left, preview on the right.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, List, ListItem, Paragraph};
use unicode_width::UnicodeWidthStr;

use crate::app::App;
use crate::model::Row;

const ACCENT: Color = Color::Cyan;
const MATCH: Style = Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD);
const DIM: Style = Style::new().add_modifier(Modifier::DIM);

pub fn draw(frame: &mut Frame, app: &mut App) {
    let [top, body] =
        Layout::vertical([Constraint::Length(3), Constraint::Min(0)]).areas(frame.area());
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(38), Constraint::Percentage(62)]).areas(body);

    draw_search(frame, app, top);
    draw_list(frame, app, left);
    draw_preview(frame, app, right);
}

fn block() -> Block<'static> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(DIM)
}

fn draw_search(frame: &mut Frame, app: &App, area: Rect) {
    let count = format!(" {}/{} ", app.hits.len(), app.rows.len());
    let hint = match &app.error {
        Some(err) => Line::from(format!(" {err} ")).red(),
        None => Line::from(" enter focus · esc clear/close · ctrl-r refresh ").style(DIM),
    };
    let block = block()
        .title(Line::from(" herdr picker ").fg(ACCENT).bold())
        .title(Line::from(count).style(DIM).right_aligned())
        .title_bottom(hint.right_aligned());

    let prompt = Span::styled("❯ ", Style::new().fg(ACCENT).bold());
    let input =
        Paragraph::new(Line::from(vec![prompt, Span::raw(app.query.as_str())])).block(block);
    frame.render_widget(input, area);

    let cursor_x = area.x + 1 + 2 + app.query.width() as u16;
    frame.set_cursor_position(Position::new(
        cursor_x.min(area.right().saturating_sub(2)),
        area.y + 1,
    ));
}

fn draw_list(frame: &mut Frame, app: &mut App, area: Rect) {
    let items: Vec<ListItem> = app
        .hits
        .iter()
        .map(|hit| {
            let row = &app.rows[hit.row];
            let mut spans = vec![
                Span::styled(format!("{:>2} ", row.number), DIM),
                status_dot(&row.status),
                Span::raw(" "),
            ];
            spans.extend(highlighted(&row.label, &hit.label, Style::new().bold()));
            if row.focused {
                spans.push(Span::styled(" (current)", DIM));
            }
            spans.push(Span::raw("  "));
            spans.extend(highlighted(&row.cwd_display, &hit.cwd, DIM));
            ListItem::new(Line::from(spans))
        })
        .collect();

    let empty = items.is_empty();
    let list = List::new(items)
        .block(block().title(Line::from(" workspaces ").style(DIM)))
        .highlight_style(Style::new().bg(Color::Indexed(237)))
        .highlight_symbol(Line::from("▌").fg(ACCENT));
    frame.render_stateful_widget(list, area, &mut app.list);

    if empty {
        let inner = area.inner(ratatui::layout::Margin::new(2, 1));
        frame.render_widget(Paragraph::new("no matching workspaces").style(DIM), inner);
    }
}

fn draw_preview(frame: &mut Frame, app: &App, area: Rect) {
    let Some(row) = app.selected() else {
        frame.render_widget(block(), area);
        return;
    };
    let block = block().title(Line::from(format!(" {} ", row.label)).fg(ACCENT).bold());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut header = header_lines(app, row);
    // Keep the tab tree from crowding out the screen on small popups.
    let max_tree = (inner.height as usize / 3).max(3);
    let tree = tree_lines(row);
    if tree.len() > max_tree {
        let hidden = tree.len() - max_tree + 1;
        header.extend(tree.into_iter().take(max_tree - 1));
        header.push(Line::styled(format!("  … {hidden} more"), DIM));
    } else {
        header.extend(tree);
    }
    let rule = format!(" screen {} ", row.active_pane_id.as_deref().unwrap_or("-"));
    let fill = (inner.width as usize).saturating_sub(rule.width() + 2);
    header.push(Line::styled(format!("──{rule}{}", "─".repeat(fill)), DIM));

    let header_height = (header.len() as u16).min(inner.height);
    let [head, screen] =
        Layout::vertical([Constraint::Length(header_height), Constraint::Min(0)]).areas(inner);
    frame.render_widget(Paragraph::new(Text::from(header)), head);

    let screen_text = match row.active_pane_id.as_ref().and_then(|p| app.screens.get(p)) {
        Some(Ok(text)) => {
            // Bottom-align: the newest output (and any prompt) lives at the end.
            let skip = text.lines.len().saturating_sub(screen.height as usize);
            Text::from(text.lines[skip..].to_vec())
        }
        Some(Err(err)) => Text::styled(format!("could not read pane: {err}"), Style::new().red()),
        None => Text::styled("loading…", DIM),
    };
    frame.render_widget(Paragraph::new(screen_text), screen);
}

fn header_lines(app: &App, row: &Row) -> Vec<Line<'static>> {
    let mut location = vec![Span::styled(row.cwd_display.clone(), DIM)];
    if let Some(Some(git)) = app.git.get(&row.cwd) {
        location.push(Span::raw("  "));
        location.push(Span::styled(
            format!("⎇ {}", git.branch),
            Style::new().fg(Color::Magenta),
        ));
        if git.dirty {
            location.push(Span::styled(" ✱", Style::new().fg(Color::Yellow)));
        }
    }
    if let Some(repo) = &row.repo_name {
        let kind = if row.linked_worktree {
            "worktree of "
        } else {
            "repo "
        };
        location.push(Span::styled(format!("  · {kind}{repo}"), DIM));
    }

    let mut status = vec![status_dot(&row.status), Span::raw(" ")];
    status.push(Span::styled(
        status_label(&row.status).to_owned(),
        status_style(&row.status),
    ));
    status.push(Span::styled(
        format!(
            "  · {} tab{}",
            row.tabs.len(),
            if row.tabs.len() == 1 { "" } else { "s" }
        ),
        DIM,
    ));

    vec![Line::from(location), Line::from(status), Line::raw("")]
}

fn tree_lines(row: &Row) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for tab in &row.tabs {
        let marker = if tab.active {
            Span::styled("▸ ", Style::new().fg(ACCENT))
        } else {
            Span::raw("  ")
        };
        let label = if tab.active {
            Span::styled(tab.label.clone(), Style::new().bold())
        } else {
            Span::raw(tab.label.clone())
        };
        let mut spans = vec![marker, label];
        if is_agent_status(&tab.status) {
            spans.push(Span::raw("  "));
            spans.push(status_dot(&tab.status));
        }
        lines.push(Line::from(spans));

        for pane in &tab.panes {
            let bullet = if pane.focused { "●" } else { "·" };
            let mut spans = vec![
                Span::styled(format!("    {bullet} "), DIM),
                Span::raw(pane.label.clone()),
            ];
            if let Some(agent) = &pane.agent {
                spans.push(Span::styled(
                    format!("  {agent}"),
                    Style::new().fg(Color::Blue),
                ));
                spans.push(Span::raw(" "));
                spans.push(Span::styled(
                    status_label(&pane.status).to_owned(),
                    status_style(&pane.status),
                ));
            }
            lines.push(Line::from(spans));
        }
    }
    lines
}

/// Splits `text` into spans, styling the chars at `hits` (sorted char indices) as matches.
fn highlighted(text: &str, hits: &[usize], base: Style) -> Vec<Span<'static>> {
    if hits.is_empty() {
        return vec![Span::styled(text.to_owned(), base)];
    }
    let mut spans = Vec::new();
    let mut buf = String::new();
    let mut buf_match = false;
    let mut next = hits.iter().peekable();
    for (i, c) in text.chars().enumerate() {
        let is_match = next.next_if(|&&h| h == i).is_some();
        if is_match != buf_match && !buf.is_empty() {
            spans.push(Span::styled(
                std::mem::take(&mut buf),
                if buf_match { base.patch(MATCH) } else { base },
            ));
        }
        buf_match = is_match;
        buf.push(c);
    }
    if !buf.is_empty() {
        spans.push(Span::styled(
            buf,
            if buf_match { base.patch(MATCH) } else { base },
        ));
    }
    spans
}

fn is_agent_status(status: &str) -> bool {
    matches!(status, "working" | "blocked" | "done" | "idle")
}

fn status_style(status: &str) -> Style {
    match status {
        "working" => Style::new().fg(Color::Yellow),
        "blocked" => Style::new().fg(Color::Red).bold(),
        "done" => Style::new().fg(Color::Green),
        _ => DIM,
    }
}

fn status_label(status: &str) -> &str {
    if is_agent_status(status) {
        status
    } else {
        "no agent"
    }
}

fn status_dot(status: &str) -> Span<'static> {
    match status {
        "working" | "blocked" | "done" => Span::styled("●", status_style(status)),
        "idle" => Span::styled("○", DIM),
        _ => Span::raw(" "),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlighted_groups_runs() {
        let spans = highlighted("abcde", &[1, 2, 4], Style::new());
        let parts: Vec<_> = spans.iter().map(|s| s.content.to_string()).collect();
        assert_eq!(parts, ["a", "bc", "d", "e"]);
        assert_eq!(spans[1].style, MATCH);
    }
}
