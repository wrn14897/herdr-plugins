//! Rendering: search bar on top, workspace list on the left, preview on the right.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, List, ListItem, ListState, Paragraph};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::{App, View};
use crate::model::Row;
use crate::search::Hit;

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
    let mut count = format!(" {}/{} ", app.hits.len(), app.rows.len());
    if app.index_pending > 0 {
        let done = app.index_total - app.index_pending;
        count = format!(" indexing {done}/{} ·{count}", app.index_total);
    }
    let hint = match &app.error {
        Some(err) => Line::from(format!(" {err} ")).red(),
        None if app.current_content().is_some() => {
            Line::from(" enter focus · alt-j/k next/prev hit · ctrl-s hits/screen · esc ")
                .style(DIM)
        }
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
    let block = block().title(Line::from(" workspaces ").style(DIM));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if app.hits.is_empty() {
        let msg = Rect {
            x: inner.x + 1,
            width: inner.width.saturating_sub(1),
            ..inner
        };
        frame.render_widget(Paragraph::new("no matching workspaces").style(DIM), msg);
        return;
    }

    // Only the visible window is materialized, so thousands of rows cost nothing per frame.
    let height = inner.height as usize;
    let selected = app.list.selected().unwrap_or(0);
    let heights = |i: usize| hit_height(&app.hits[i]);
    app.list_offset = window_start(app.list_offset, selected, app.hits.len(), height, heights);

    let mut used = 0;
    let items: Vec<ListItem> = app.hits[app.list_offset..]
        .iter()
        .take_while(|hit| {
            used += hit_height(hit);
            used <= height
        })
        .map(|hit| ListItem::new(hit_lines(app, hit)))
        .collect();

    let list = List::new(items)
        .highlight_style(Style::new().bg(Color::Indexed(237)))
        .highlight_symbol(Line::from("▌").fg(ACCENT));
    let mut state = ListState::default().with_selected(Some(selected - app.list_offset));
    frame.render_stateful_widget(list, inner, &mut state);
}

fn hit_height(hit: &Hit) -> usize {
    1 + usize::from(hit.detail.is_some())
}

fn hit_lines(app: &App, hit: &Hit) -> Vec<Line<'static>> {
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
    let mut lines = vec![Line::from(spans)];

    if let Some(detail) = &hit.detail {
        let badge = detail
            .badge
            .clone()
            .unwrap_or_else(|| hit.kind.badge().to_owned());
        let mut spans = vec![
            Span::styled("     ↳ ", DIM),
            Span::styled(format!("{badge}  "), Style::new().fg(Color::Blue)),
        ];
        if hit.kind.is_content() && hit.content.len() > 1 {
            spans.push(Span::styled(format!("+{} ", hit.content.len() - 1), DIM));
        }
        spans.extend(highlighted(&detail.text, &detail.indices, Style::new()));
        lines.push(Line::from(spans));
    }
    lines
}

/// First visible item index so `selected` stays on screen, scrolling as little
/// as possible and never leaving blank space below the last item.
fn window_start(
    offset: usize,
    selected: usize,
    len: usize,
    height: usize,
    item_height: impl Fn(usize) -> usize,
) -> usize {
    if len == 0 || height == 0 {
        return 0;
    }
    let selected = selected.min(len - 1);
    let mut start = offset.min(selected);
    let span = |from: usize, to: usize| (from..=to).map(&item_height).sum::<usize>();
    while start < selected && span(start, selected) > height {
        start += 1;
    }
    while start > 0 && span(start - 1, len - 1) <= height {
        start -= 1;
    }
    start
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
    let view = app.view();
    let rule = match (view, app.current_content()) {
        (View::Hits, Some((store, hit, cursor, total))) => {
            format!(
                " {} {}/{total} ",
                store.doc(hit.doc).source.badge(),
                cursor + 1
            )
        }
        (_, content) => {
            let more = content.map_or(String::new(), |(.., total)| {
                format!(
                    "· {total} hit{} (ctrl-s) ",
                    if total == 1 { "" } else { "s" }
                )
            });
            format!(
                " screen {} {more}",
                row.active_pane_id.as_deref().unwrap_or("-")
            )
        }
    };
    let fill = (inner.width as usize).saturating_sub(rule.width() + 2);
    let rule_style = if view == View::Hits {
        Style::new().fg(Color::Blue)
    } else {
        DIM
    };
    header.push(Line::styled(
        format!("──{rule}{}", "─".repeat(fill)),
        rule_style,
    ));

    let header_height = (header.len() as u16).min(inner.height);
    let [head, screen] =
        Layout::vertical([Constraint::Length(header_height), Constraint::Min(0)]).areas(inner);
    frame.render_widget(Paragraph::new(Text::from(header)), head);

    if view == View::Hits {
        frame.render_widget(
            Paragraph::new(hit_view(app, screen.width, screen.height)),
            screen,
        );
        return;
    }
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

/// The selected content hit in context: the whole chat message, or the
/// surrounding scrollback lines, wrapped and scrolled so the hit is in view.
fn hit_view(app: &App, width: u16, height: u16) -> Text<'static> {
    let (Some((store, hit, ..)), Some(query)) = (app.current_content(), &app.content_query) else {
        return Text::default();
    };
    let (width, height) = (usize::from(width).max(10), usize::from(height));
    let source = &store.doc(hit.doc).source;

    // Context: neighbouring screen lines of the same pane, or the single message.
    let (first, last) = if source.is_chat() {
        (hit.doc, hit.doc)
    } else {
        let same = |i: usize| store.doc(i).source == *source;
        let reach = height / 2;
        let mut first = hit.doc;
        while first > 0 && hit.doc - first < reach && same(first - 1) {
            first -= 1;
        }
        let mut last = hit.doc;
        while last + 1 < store.doc_count() && last - hit.doc < reach && same(last + 1) {
            last += 1;
        }
        (first, last)
    };
    let mut text = String::new();
    let mut focus = 0;
    for i in first..=last {
        if i == hit.doc {
            focus = text.len() + (hit.range.start - store.doc(i).range.start);
        }
        text.push_str(store.doc_text(i));
        text.push('\n');
    }

    let ranges = query.highlight_ranges(&text);
    let (lines, focus_line) = wrap_highlighted(&text, &ranges, focus, width);
    let start = focus_line
        .saturating_sub(height / 3)
        .min(lines.len().saturating_sub(height));
    Text::from(
        lines
            .into_iter()
            .skip(start)
            .take(height)
            .collect::<Vec<_>>(),
    )
}

/// Hard-wraps `text` to `width` columns, styling `ranges` (sorted byte ranges)
/// as matches and the range starting at `focus` as the current hit. Returns the
/// lines and the index of the line containing `focus`.
fn wrap_highlighted(
    text: &str,
    ranges: &[std::ops::Range<usize>],
    focus: usize,
    width: usize,
) -> (Vec<Line<'static>>, usize) {
    let current = MATCH.add_modifier(Modifier::REVERSED);
    let mut lines = Vec::new();
    let mut spans: Vec<Span<'static>> = Vec::new();
    let (mut run, mut run_style, mut col) = (String::new(), Style::new(), 0);
    let mut focus_line = 0;
    let mut next_range = 0;

    let flush_run = |spans: &mut Vec<Span<'static>>, run: &mut String, style: Style| {
        if !run.is_empty() {
            spans.push(Span::styled(std::mem::take(run), style));
        }
    };

    for (pos, ch) in text.char_indices() {
        while next_range < ranges.len() && ranges[next_range].end <= pos {
            next_range += 1;
        }
        let style = match ranges.get(next_range) {
            Some(r) if r.contains(&pos) => {
                let in_focus = ranges[next_range].start <= focus && focus < ranges[next_range].end;
                if in_focus { current } else { MATCH }
            }
            _ => Style::new(),
        };
        if pos == focus {
            focus_line = lines.len();
        }
        let ch_width = ch.width().unwrap_or(0);
        if ch == '\n' || col + ch_width > width {
            flush_run(&mut spans, &mut run, run_style);
            lines.push(Line::from(std::mem::take(&mut spans)));
            col = 0;
            if ch == '\n' {
                continue;
            }
            if pos == focus {
                focus_line = lines.len();
            }
        }
        if style != run_style {
            flush_run(&mut spans, &mut run, run_style);
            run_style = style;
        }
        run.push(ch);
        col += ch_width;
    }
    flush_run(&mut spans, &mut run, run_style);
    if !spans.is_empty() {
        lines.push(Line::from(spans));
    }
    (lines, focus_line)
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
    fn window_keeps_selection_visible() {
        let one = |_| 1;
        assert_eq!(window_start(0, 0, 100, 10, one), 0);
        assert_eq!(window_start(0, 15, 100, 10, one), 6);
        assert_eq!(window_start(6, 3, 100, 10, one), 3);
        // Near the end, the window is pulled back to fill the area.
        assert_eq!(window_start(95, 99, 100, 10, one), 90);
        // Fewer items than rows: always start at the top.
        assert_eq!(window_start(4, 2, 5, 10, one), 0);
        // Variable heights.
        assert_eq!(
            window_start(0, 4, 10, 6, |i| if i % 2 == 0 { 2 } else { 1 }),
            1
        );
    }

    #[test]
    fn wrap_highlighted_finds_focus_line() {
        let text = "alpha beta\ngamma delta epsilon";
        let ranges = [6..10, 17..22];
        let (lines, focus) = wrap_highlighted(text, &ranges, 17, 8);
        let rendered: Vec<String> = lines.iter().map(ToString::to_string).collect();
        assert_eq!(rendered, ["alpha be", "ta", "gamma de", "lta epsi", "lon"]);
        assert_eq!(focus, 2);
        // The focused match is styled differently from other matches.
        let style_of = |line: usize, text: &str| {
            lines[line]
                .spans
                .iter()
                .find(|s| s.content == text)
                .map(|s| s.style)
        };
        assert_eq!(
            style_of(2, "de"),
            Some(MATCH.add_modifier(Modifier::REVERSED))
        );
        assert_eq!(style_of(0, "be"), Some(MATCH));
    }

    #[test]
    fn highlighted_groups_runs() {
        let spans = highlighted("abcde", &[1, 2, 4], Style::new());
        let parts: Vec<_> = spans.iter().map(|s| s.content.to_string()).collect();
        assert_eq!(parts, ["a", "bc", "d", "e"]);
        assert_eq!(spans[1].style, MATCH);
    }
}
