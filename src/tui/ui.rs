//! Drawing the pane.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, Paragraph};
use ratatui::Frame;

use super::app::{App, Focus, Mode, HELP};

/// Below this width the list and the note are shown one at a time.
pub const NARROW: u16 = 80;

pub fn draw(frame: &mut Frame, app: &mut App) {
    let [header, body, status] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    draw_header(frame, app, header);

    app.narrow = body.width < NARROW;
    let (list_area, detail_area) = if app.narrow {
        match app.focus {
            Focus::List => (body, Rect::default()),
            Focus::Detail => (Rect::default(), body),
        }
    } else {
        let list_width = (body.width / 3).clamp(20, 40);
        let [l, d] =
            Layout::horizontal([Constraint::Length(list_width), Constraint::Min(1)]).areas(body);
        (l, d)
    };
    app.list_area = list_area;
    app.detail_area = detail_area;
    if list_area.width > 0 {
        draw_list(frame, app, list_area);
    }
    if detail_area.width > 0 {
        draw_detail(frame, app, detail_area);
    }

    if app.query_open {
        let line = Line::from(vec![
            Span::styled("/", Style::new().add_modifier(Modifier::BOLD)),
            Span::raw(app.query.clone()),
            Span::styled("▏", app.theme.dim()),
        ]);
        frame.render_widget(Paragraph::new(line), status);
        if app.help {
            draw_help(frame, app);
        }
        return;
    }
    let hint = app.status.clone().unwrap_or_else(|| {
        if app.narrow {
            "? keys  h l list/note  tab mode  q quit".into()
        } else {
            "? keys  tab mode  enter open  n link  [ back  q quit".into()
        }
    });
    frame.render_widget(Paragraph::new(Span::styled(hint, app.theme.dim())), status);

    if app.help {
        draw_help(frame, app);
    }
}

fn draw_header(frame: &mut Frame, app: &App, area: Rect) {
    let bold = Style::new().add_modifier(Modifier::BOLD);
    let dim = app.theme.dim();
    let width = usize::from(area.width);
    let send = if app.send_allow.is_empty() {
        "send off".to_string()
    } else {
        format!("send: {}", app.send_allow.join(" "))
    };
    let lead = |label: &str| {
        vec![
            Span::styled(" knapp ", bold.add_modifier(Modifier::REVERSED)),
            Span::raw(format!(" {label} ")),
            Span::styled("│ ", dim),
        ]
    };
    let current = bold.add_modifier(Modifier::UNDERLINED);
    let mut all = lead(&app.root_label);
    for m in Mode::ALL {
        all.push(Span::styled(
            m.name(),
            if m == app.mode { current } else { dim },
        ));
        all.push(Span::raw(" "));
    }
    all.push(Span::styled(format!("│ {send}"), dim));
    let short = |label: &str| {
        let mut spans = lead(label);
        spans.push(Span::styled("◂ ", dim));
        spans.push(Span::styled(app.mode.name(), current));
        spans.push(Span::styled(" ▸", dim));
        Line::from(spans)
    };
    // Every mode when it fits; else the current mode, with the root's last
    // path segment when the whole label does not fit either.
    let line = if Line::from(all.clone()).width() <= width {
        Line::from(all)
    } else if short(&app.root_label).width() <= width {
        short(&app.root_label)
    } else {
        let last = app.root_label.rsplit('/').next().unwrap_or(&app.root_label);
        short(last)
    };
    frame.render_widget(Paragraph::new(line), area);
}

fn draw_list(frame: &mut Frame, app: &mut App, area: Rect) {
    let rows = app.rows();
    let highlight = if app.focus == Focus::List {
        Style::new().add_modifier(Modifier::REVERSED)
    } else {
        Style::new().add_modifier(Modifier::BOLD)
    };
    let list = List::new(rows.iter().map(|r| r.line.clone())).highlight_style(highlight);
    let len = rows.len();
    if app.list.selected().is_some_and(|s| s >= len) {
        app.list.select(Some(len.saturating_sub(1)));
    }
    frame.render_stateful_widget(list, area, &mut app.list);
}

fn draw_detail(frame: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::new()
        .borders(Borders::LEFT)
        .border_style(app.theme.dim());
    frame.render_widget(block, area);
    let height = area.height.saturating_sub(1);
    let width = app.detail_width();
    let detail = app.detail(width, height);
    let content = Rect {
        x: area.x + 2,
        y: area.y + 1,
        width,
        height,
    };
    let title_style = if app.focus == Focus::Detail {
        Style::new().add_modifier(Modifier::BOLD)
    } else {
        app.theme.dim()
    };
    frame.render_widget(
        Paragraph::new(Span::styled(detail.title.clone(), title_style)),
        Rect {
            x: area.x + 2,
            y: area.y,
            width,
            height: 1,
        },
    );
    let visible: Vec<Line> = detail
        .lines
        .iter()
        .skip(app.scroll)
        .take(usize::from(height))
        .cloned()
        .collect();
    frame.render_widget(Paragraph::new(visible), content);

    if let Some(hit) = app.selected_hit.and_then(|i| detail.hits.get(i)) {
        let Some(row) = hit.line.checked_sub(app.scroll) else {
            return;
        };
        if row < usize::from(height) {
            let start = hit.cols.start.min(width);
            let end = hit.cols.end.min(width);
            let r = Rect {
                x: content.x + start,
                y: content.y + row as u16,
                width: end - start,
                height: 1,
            };
            frame
                .buffer_mut()
                .set_style(r, Style::new().add_modifier(Modifier::REVERSED));
        }
    }
}

fn draw_help(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let key_width = HELP.iter().map(|(k, _)| k.len()).max().unwrap_or(0);
    let width = (key_width + 50).min(usize::from(area.width)) as u16;
    let height = (HELP.len() + 2).min(usize::from(area.height)) as u16;
    let r = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    };
    let lines: Vec<Line> = HELP
        .iter()
        .map(|(k, what)| {
            Line::from(vec![
                Span::styled(
                    format!(" {k:<key_width$}  "),
                    Style::new().add_modifier(Modifier::BOLD),
                ),
                Span::raw(*what),
            ])
        })
        .collect();
    frame.render_widget(Clear, r);
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .title(" keys ")
                .border_style(app.theme.dim()),
        ),
        r,
    );
}
