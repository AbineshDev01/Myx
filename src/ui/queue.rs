//! The Queue view.

use crate::*;

pub(crate) fn render_queue_view(f: &mut Frame, app: &App, theme: Theme, area: Rect) {
    let inner = area.inner(Margin::new(2, 1));
    if inner.height == 0 {
        return;
    }
    let max = inner.width as usize;
    let mut lines: Vec<Line> = Vec::new();

    // Context header — what's playing from.
    if !app.transport.source_name.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("PLAYING FROM  ", theme.muted()),
            Span::styled(
                truncate(&app.transport.source_name, max.saturating_sub(14)),
                Style::default()
                    .fg(theme.primary.into())
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        lines.push(Line::raw(""));
    }

    // Now playing — the current track, above the up-next list.
    if let Some(n) = app.playback.now.as_ref() {
        lines.push(Line::from(Span::styled("NOW PLAYING", theme.heading())));
        let is_liked = app.is_liked(&n.uri);
        let mut row_spans = vec![
            Span::styled("   ", theme.muted()),
            Span::styled(
                truncate(&n.title, max.saturating_sub(if is_liked { 5 } else { 3 })),
                Style::default()
                    .fg(theme.text.into())
                    .add_modifier(Modifier::BOLD),
            ),
        ];
        if is_liked {
            row_spans.push(Span::styled(
                " ♥",
                Style::default().fg(theme.accent.into()),
            ));
        }
        row_spans.push(Span::styled(format!("  {}", n.artist), theme.muted()));
        lines.push(Line::from(row_spans));
        lines.push(Line::raw(""));
    }

    lines.push(Line::from(Span::styled("UP NEXT", theme.heading())));
    lines.push(Line::raw(""));

    let used = lines.len();
    if app.transport.queue.is_empty() {
        lines.push(Line::from(Span::styled("queue is empty", theme.muted())));
    } else {
        for (i, q) in app
            .transport
            .queue
            .iter()
            .take(inner.height.saturating_sub(used as u16) as usize)
            .enumerate()
        {
            let is_liked = app
                .transport
                .queue_uris
                .get(i)
                .map(|u| app.is_liked(u))
                .unwrap_or(false);
            let mut row_spans = vec![
                Span::styled(format!("{:>2}  ", i + 1), theme.muted()),
                Span::styled(
                    truncate(q, max.saturating_sub(if is_liked { 6 } else { 4 })),
                    Style::default().fg(theme.text.into()),
                ),
            ];
            if is_liked {
                row_spans.push(Span::styled(
                    " ♥",
                    Style::default().fg(theme.accent.into()),
                ));
            }
            lines.push(Line::from(row_spans));
        }
    }
    f.render_widget(Paragraph::new(lines), inner);
}
