use crate::ui::{self, number, safe, text, App, Hit, HitKind, Menu, Modal};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    symbols::Marker,
    text::{Line, Span},
    widgets::{
        Axis, Block, Borders, Chart, Clear, Dataset, GraphType, List, ListItem, ListState,
        Paragraph, Row, Table, TableState, Wrap,
    },
    Frame,
};
use serde_json::Value;
use unicode_width::UnicodeWidthStr;
#[derive(Clone, Copy)]
struct Theme {
    bg: Color,
    fg: Color,
    muted: Color,
    accent: Color,
    border: Color,
    selected: Color,
}
fn theme(app: &App) -> Theme {
    if app.page().is_some_and(|p| p["theme"] == "light") {
        Theme {
            bg: Color::Rgb(246, 248, 250),
            fg: Color::Rgb(31, 41, 55),
            muted: Color::Rgb(96, 110, 128),
            accent: Color::Rgb(20, 101, 182),
            border: Color::Rgb(181, 192, 203),
            selected: Color::Rgb(222, 236, 249),
        }
    } else {
        Theme {
            bg: Color::Rgb(17, 23, 31),
            fg: Color::Rgb(218, 226, 236),
            muted: Color::Rgb(139, 156, 175),
            accent: Color::Rgb(102, 181, 250),
            border: Color::Rgb(52, 69, 89),
            selected: Color::Rgb(35, 59, 82),
        }
    }
}
pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let t = theme(app);
    app.hits.clear();
    f.render_widget(Block::new().style(Style::default().bg(t.bg).fg(t.fg)), area);
    if area.width < 40 || area.height < 12 {
        f.render_widget(
            Paragraph::new(
                "output-tui\nTerminal too small\nResize to 40 × 12 or larger\nq to detach",
            )
            .style(Style::default().fg(t.accent)),
            area,
        );
        return;
    }
    let mut page = app.page().cloned().unwrap_or_default();
    if let Some(p) = &app.pan {
        page["view_x"] = serde_json::json!(p.current.0);
        page["view_y"] = serde_json::json!(p.current.1);
    }
    let header = Rect::new(0, 0, area.width, 1);
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                " log-print / terminal ",
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ),
            Span::raw(safe(&text(&page, "title"))),
            Span::styled(
                format!(
                    "  ·  rev {}  ·  {}",
                    app.revision(),
                    if app.snapshot.connected {
                        "CONNECTED"
                    } else {
                        "DISCONNECTED"
                    }
                ),
                Style::default().fg(if app.snapshot.connected {
                    t.muted
                } else {
                    Color::Red
                }),
            ),
        ])),
        header,
    );
    let mut x = 1u16;
    for p in app.snapshot.state["pages"].as_array().into_iter().flatten() {
        let label = format!(" {} ", safe(&text(p, "name")));
        let w = (label.width() as u16 + 1).min(area.width.saturating_sub(x));
        if w == 0 {
            break;
        }
        let r = Rect::new(x, 1, w, 1);
        f.render_widget(
            Paragraph::new(label).style(
                Style::default()
                    .fg(if p["id"] == page["id"] {
                        t.accent
                    } else {
                        t.muted
                    })
                    .bg(if p["id"] == page["id"] {
                        t.selected
                    } else {
                        t.bg
                    }),
            ),
            r,
        );
        app.hits.push(Hit {
            area: r,
            kind: HitKind::Page(text(p, "id")),
        });
        x += w;
    }
    let footer = Rect::new(0, area.height - 2, area.width, 2);
    let status = if app.snapshot.notice.is_empty() {
        app.status.clone()
    } else {
        format!("{}  {}", app.status, app.snapshot.notice)
    };
    f.render_widget(
        Paragraph::new(vec![
            Line::styled(safe(&status), Style::default().fg(t.muted)),
            Line::styled(
                " Tab panels   p pages   s streams   e inspect   : command   ? help   q detach",
                Style::default().fg(t.accent),
            ),
        ]),
        footer,
    );
    let side = if page["sidebar_open"] == true && area.width >= 100 {
        24
    } else {
        0
    };
    let inspector = if page["inspector_open"] == true && area.width >= 140 {
        28
    } else {
        0
    };
    let body = Rect::new(side, 3, area.width - side - inspector, area.height - 5);
    app.body = body;
    if side > 0 {
        draw_sources(f, app, Rect::new(0, 3, side, area.height - 5), t);
    }
    if inspector > 0 {
        let r = Rect::new(area.width - inspector, 3, inspector, area.height - 5);
        let lines = app
            .menu_items(Menu::Panel)
            .into_iter()
            .map(|s| Line::raw(safe(&s)))
            .collect::<Vec<_>>();
        f.render_widget(
            Paragraph::new(lines)
                .block(
                    Block::bordered()
                        .title(" Panel · e to edit ")
                        .border_style(Style::default().fg(t.border)),
                )
                .wrap(Wrap { trim: false }),
            r,
        );
    }
    if page["show_grid"] == true {
        for y in (body.y..body.bottom()).step_by(2) {
            for x in (body.x..body.right()).step_by(4) {
                f.buffer_mut()[(x, y)].set_symbol("·").set_fg(t.border);
            }
        }
    }
    let mut panels = app.panels();
    if let Some(d) = &app.draft {
        if let Some(p) = panels.iter_mut().find(|p| p["id"] == d.panel["id"]) {
            *p = d.panel.clone();
        }
    }
    panels.sort_by_key(|p| p["z_index"].as_u64().unwrap_or(0));
    let visible = panels.iter().filter(|p| p["hidden"] != true).count();
    if visible == 0 {
        f.render_widget(Paragraph::new("Create your terminal workspace\n\nPress s to choose a stream, a for logs, c for a curve.\nUse m / r to place panels, or : for the full CLI.\nPages and display settings persist in the backend.\n\nHistory only covers the reported archive / Core ranges.").style(Style::default().fg(t.muted)).wrap(Wrap{trim:false}),Rect::new(body.x+2,body.y+2,body.width.saturating_sub(4),body.height.saturating_sub(4)));
    }
    for p in &panels {
        if p["hidden"] == true {
            continue;
        }
        if let Some(r) = panel_rect(p, &page, body) {
            draw_panel(f, app, p, r, t);
        }
    }
    if page["show_minimap"] == true && body.width >= 45 && body.height >= 12 {
        draw_minimap(f, &panels, &page, body, t);
    }
    if let Some(modal) = app.modal.clone() {
        draw_modal(f, app, modal, t);
    }
}
fn draw_sources(f: &mut Frame, app: &mut App, area: Rect, t: Theme) {
    let block = Block::new()
        .borders(Borders::RIGHT)
        .title(" STREAMS · s ")
        .border_style(Style::default().fg(t.border));
    let inner = block.inner(area);
    f.render_widget(block, area);
    for (i, s) in app.snapshot.state["streams"]
        .as_array()
        .into_iter()
        .flatten()
        .take(inner.height as usize / 3)
        .enumerate()
    {
        let y = inner.y + (i as u16) * 3 + 1;
        if y + 1 >= inner.bottom() {
            break;
        }
        let r = Rect::new(inner.x + 1, y, inner.width.saturating_sub(2), 2);
        let alias = s["alias"].as_str().unwrap_or("unnamed");
        f.render_widget(
            Paragraph::new(vec![
                Line::styled(safe(alias), Style::default().fg(t.fg)),
                Line::styled(safe(&text(s, "owner")), Style::default().fg(t.muted)),
            ]),
            r,
        );
        app.hits.push(Hit {
            area: r,
            kind: HitKind::Source(i),
        });
    }
}
pub fn panel_rect(p: &Value, page: &Value, body: Rect) -> Option<Rect> {
    let (x, y, w, h) = if page["layout_mode"] == "grid" {
        let col = f64::from(body.width) / 12.;
        (
            number(p, "x", 0.) * col,
            number(p, "y", 0.) * 3.,
            number(p, "w", 12.) * col,
            number(p, "h", 6.) * 3.,
        )
    } else {
        let z = number(page, "view_zoom", 1.);
        (
            (number(p, "left", 0.) * z + number(page, "view_x", 0.)) / 8.,
            (number(p, "top", 0.) * z + number(page, "view_y", 0.)) / 16.,
            number(p, "panel_width", 640.) * z / 8.,
            number(p, "panel_height", 320.) * z / 16.,
        )
    };
    let left = (x.round() as i64).max(0).min(i64::from(body.width));
    let top = (y.round() as i64).max(0).min(i64::from(body.height));
    let right = ((x + w).round() as i64).max(0).min(i64::from(body.width));
    let bottom = ((y + h).round() as i64).max(0).min(i64::from(body.height));
    if right - left < 8 || bottom - top < 4 {
        return None;
    }
    Some(Rect::new(
        body.x + left as u16,
        body.y + top as u16,
        (right - left) as u16,
        (bottom - top) as u16,
    ))
}
fn draw_panel(f: &mut Frame, app: &mut App, p: &Value, r: Rect, t: Theme) {
    let id = text(p, "id");
    let selected = app.panel_id().as_deref() == Some(&id);
    let data = app.panel_data(p);
    f.render_widget(Clear, r);
    f.render_widget(Block::new().style(Style::default().bg(t.bg).fg(t.fg)), r);
    let state = if p["paused"] == true {
        "PAUSED"
    } else if p["mode"] == "history" {
        "HISTORY"
    } else {
        "LIVE"
    };
    let title = format!(
        " {}  ·  {} {} ",
        safe(&text(p, "title")),
        state,
        if p["locked"] == true { "◆" } else { "" }
    );
    let block = Block::bordered()
        .title(title)
        .border_style(Style::default().fg(if selected { t.accent } else { t.border }));
    let inner = block.inner(r);
    f.render_widget(block, r);
    app.hits.push(Hit {
        area: r,
        kind: HitKind::Panel(id.clone()),
    });
    app.hits.push(Hit {
        area: Rect::new(r.x + 1, r.y, r.width - 2, 1),
        kind: HitKind::Header(id.clone()),
    });
    let footer = Rect::new(inner.x, inner.bottom() - 1, inner.width, 1);
    let content = Rect::new(inner.x, inner.y, inner.width, inner.height - 1);
    let message = if data["error"].is_string() {
        text(&data, "error")
    } else if data["waiting"] == true {
        "Waiting for source".into()
    } else if p["kind"] == "curve" && p["series"].as_array().is_none_or(Vec::is_empty) {
        "Press y, then a to define a series".into()
    } else {
        String::new()
    };
    if !message.is_empty() {
        f.render_widget(
            Paragraph::new(safe(&message))
                .style(Style::default().fg(t.muted))
                .wrap(Wrap { trim: false }),
            content,
        );
    } else if p["kind"] == "curve" {
        draw_curve(f, p, &data, content, t);
    } else {
        draw_log(f, app, p, &data, content, t, selected);
    }
    let note = if p["mode"] == "history" {
        format!(
            " {}  rows {} / {} · o coverage",
            data["status"]["state"].as_str().unwrap_or("query"),
            p["offset"].as_u64().unwrap_or(0),
            data["total"].as_u64().unwrap_or(0)
        )
    } else {
        format!(
            " {} · {} · {}",
            if p["follow"] == true {
                "following"
            } else {
                "manual"
            },
            if p["format"] == "hex" { "hex" } else { "text" },
            "live cache"
        )
    };
    f.render_widget(
        Paragraph::new(note).style(Style::default().fg(t.muted)),
        footer,
    );
    if p["locked"] != true {
        f.buffer_mut()[(r.right() - 1, r.bottom() - 1)]
            .set_symbol("◢")
            .set_fg(t.accent);
        app.hits.push(Hit {
            area: Rect::new(r.right() - 2, r.bottom() - 2, 2, 2),
            kind: HitKind::Resize(id),
        });
    }
}
fn draw_log(
    f: &mut Frame,
    app: &mut App,
    p: &Value,
    data: &Value,
    area: Rect,
    t: Theme,
    selected: bool,
) {
    if area.height < 2 {
        return;
    }
    let rows = ui::sorted_rows(data, p);
    let mut columns: Vec<(String, u16)> = if let Some(state) = p["column_state"].as_array() {
        state
            .iter()
            .filter(|c| c["hide"] != true)
            .map(|c| {
                (
                    text(c, "colId"),
                    (number(c, "width", 140.) / 8.).round().clamp(4., 500.) as u16,
                )
            })
            .collect()
    } else {
        p["columns"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| {
                v.as_str().map(|s| {
                    (
                        s.to_owned(),
                        match s {
                            "text" => 50,
                            "time" => 14,
                            "channel" => 8,
                            _ => 15,
                        },
                    )
                })
            })
            .collect()
    };
    if p["metadata"] == true {
        for k in [
            "seq",
            "offset",
            "epoch",
            "source_ts_ns",
            "source_seq",
            "upstream",
            "upstream_epochs",
        ] {
            if !columns.iter().any(|c| c.0 == k) {
                columns.push((k.into(), 12));
            }
        }
    }
    if columns.is_empty() {
        columns.push(("text".into(), 50));
    }
    let skip = if selected { app.horizontal as usize } else { 0 };
    let columns: Vec<_> = columns.into_iter().skip(skip).collect();
    let row_height = (number(p, "row_height", 28.) / 28.).round().clamp(1., 5.) as u16;
    let visible = (area.height.saturating_sub(1) / row_height).max(1) as usize;
    let selected_row = if p["follow"] == true {
        rows.len().saturating_sub(1)
    } else if selected {
        app.row.min(rows.len().saturating_sub(1))
    } else {
        0
    };
    let offset = if p["follow"] == true {
        rows.len().saturating_sub(visible)
    } else if selected {
        if selected_row < app.scroll {
            app.scroll = selected_row;
        }
        if selected_row >= app.scroll + visible {
            app.scroll = selected_row + 1 - visible;
        }
        app.scroll.min(rows.len().saturating_sub(1))
    } else {
        0
    };
    let rendered: Vec<_> = rows
        .iter()
        .skip(offset)
        .take(visible)
        .map(|row| {
            let cells: Vec<_> = columns
                .iter()
                .map(|(key, _)| {
                    let key = if key == "text" && p["format"] == "hex" {
                        "hex"
                    } else {
                        key.as_str()
                    };
                    safe(&if row[key].is_null() {
                        String::new()
                    } else {
                        ui::display(&row[key])
                    })
                })
                .collect();
            Row::new(cells)
                .height(row_height)
                .style(Style::default().fg(if row["gap"] == true {
                    Color::Yellow
                } else {
                    t.fg
                }))
        })
        .collect();
    let constraints: Vec<_> = columns
        .iter()
        .enumerate()
        .map(|(i, (_, w))| {
            if i == columns.len() - 1 {
                Constraint::Min((*w).min(20))
            } else {
                Constraint::Length(*w)
            }
        })
        .collect();
    let header = Row::new(columns.iter().map(|c| c.0.clone()))
        .style(Style::default().fg(t.muted))
        .height(1);
    let table = Table::new(rendered, constraints)
        .header(header)
        .column_spacing(1)
        .row_highlight_style(Style::default().bg(t.selected));
    let mut state = TableState::default().with_selected(if selected && !rows.is_empty() {
        Some(selected_row.saturating_sub(offset))
    } else {
        None
    });
    f.render_stateful_widget(table, area, &mut state);
    for i in 0..visible.min(rows.len().saturating_sub(offset)) {
        app.hits.push(Hit {
            area: Rect::new(
                area.x,
                area.y + 1 + (i as u16) * row_height,
                area.width,
                row_height,
            ),
            kind: HitKind::Row(text(p, "id"), offset + i),
        });
    }
}
fn color(s: &str, fallback: Color) -> Color {
    if s.len() == 7 && s.starts_with('#') {
        if let Ok(v) = u32::from_str_radix(&s[1..], 16) {
            return Color::Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8);
        }
    }
    fallback
}
// A null value or explicit gap splits the path; never draw across missing records.
pub fn segments(points: &[Value]) -> Vec<Vec<(f64, f64)>> {
    let mut segments = vec![];
    let mut path = vec![];
    for p in points {
        let x = p["time"]
            .as_f64()
            .or_else(|| p["time"].as_str().and_then(|s| s.parse().ok()));
        let y = p["value"].as_f64();
        match (p["gap"] == true, x, y) {
            (false, Some(x), Some(y)) => path.push((x, y)),
            _ if !path.is_empty() => segments.push(std::mem::take(&mut path)),
            _ => {}
        }
    }
    if !path.is_empty() {
        segments.push(path);
    }
    segments
}
fn draw_curve(f: &mut Frame, p: &Value, data: &Value, area: Rect, t: Theme) {
    if area.width < 12 || area.height < 4 {
        return;
    }
    type CurvePath = (String, Color, Vec<(f64, f64)>);
    let mut paths: Vec<CurvePath> = vec![];
    for s in p["series"].as_array().into_iter().flatten() {
        let name = text(s, "name");
        if p["legend_selected"][&name] == false {
            continue;
        }
        let points: Vec<_> = if let Some(points) = data["series"]
            .as_array()
            .and_then(|a| a.iter().find(|d| d["id"] == s["id"]))
            .and_then(|s| s["points"].as_array())
        {
            points.clone()
        } else {
            data["rows"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|r| r["series"] == s["id"] || r["series"] == s["name"])
                .cloned()
                .collect()
        };
        for (i, path) in segments(&points).into_iter().enumerate() {
            paths.push((
                if i == 0 && p["legend"] != false {
                    safe(&name)
                } else {
                    String::new()
                },
                color(s["color"].as_str().unwrap_or(""), t.accent),
                path,
            ));
        }
    }
    let xs: Vec<_> = paths
        .iter()
        .flat_map(|(_, _, v)| v.iter().map(|v| v.0))
        .collect();
    let ys: Vec<_> = paths
        .iter()
        .flat_map(|(_, _, v)| v.iter().map(|v| v.1))
        .collect();
    if xs.is_empty() {
        f.render_widget(
            Paragraph::new("No numeric samples in this range").style(Style::default().fg(t.muted)),
            area,
        );
        return;
    }
    let min = xs.iter().copied().fold(f64::INFINITY, f64::min);
    let max = xs
        .iter()
        .copied()
        .fold(f64::NEG_INFINITY, f64::max)
        .max(min + 1.);
    let xmin = p["time_from"]
        .as_f64()
        .or_else(|| p["time_from"].as_str().and_then(|s| s.parse().ok()))
        .map(|v| v / 1_000_000.)
        .unwrap_or(min);
    let xmax = p["time_end"]
        .as_f64()
        .or_else(|| p["time_end"].as_str().and_then(|s| s.parse().ok()))
        .map(|v| v / 1_000_000.)
        .unwrap_or(max)
        .max(xmin + 1.);
    let range = xmax - xmin;
    let x0 = xmin + range * number(p, "zoom_start", 0.) / 100.;
    let x1 = (xmin + range * number(p, "zoom_end", 100.) / 100.).max(x0 + 0.001);
    let ymin = p["y_min"]
        .as_f64()
        .unwrap_or_else(|| ys.iter().copied().fold(f64::INFINITY, f64::min));
    let ymax = p["y_max"]
        .as_f64()
        .unwrap_or_else(|| ys.iter().copied().fold(f64::NEG_INFINITY, f64::max))
        .max(ymin + 0.001);
    let datasets = paths
        .iter()
        .map(|(name, c, points)| {
            let d = Dataset::default()
                .marker(Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(*c))
                .data(points);
            if name.is_empty() {
                d
            } else {
                d.name(name.as_str())
            }
        })
        .collect();
    let chart = Chart::new(datasets)
        .x_axis(
            Axis::default()
                .style(Style::default().fg(t.muted))
                .bounds([x0, x1])
                .labels([format!("{x0:.0}"), format!("{x1:.0}")]),
        )
        .y_axis(
            Axis::default()
                .style(Style::default().fg(t.muted))
                .bounds([ymin, ymax])
                .labels([format!("{ymin:.2}"), format!("{ymax:.2}")]),
        );
    f.render_widget(chart, area);
}
fn draw_minimap(f: &mut Frame, panels: &[Value], _page: &Value, body: Rect, t: Theme) {
    let r = Rect::new(body.right() - 22, body.bottom() - 7, 22, 7);
    f.render_widget(Clear, r);
    let block = Block::bordered()
        .title(" Map ")
        .style(Style::default().bg(t.bg).fg(t.muted));
    let inner = block.inner(r);
    f.render_widget(block, r);
    let ps: Vec<_> = panels.iter().filter(|p| p["hidden"] != true).collect();
    let maxx = ps
        .iter()
        .map(|p| number(p, "left", 0.) + number(p, "panel_width", 640.))
        .fold(1., f64::max);
    let maxy = ps
        .iter()
        .map(|p| number(p, "top", 0.) + number(p, "panel_height", 320.))
        .fold(1., f64::max);
    for p in ps {
        let x = (number(p, "left", 0.) / maxx * f64::from(inner.width - 1))
            .clamp(0., f64::from(inner.width - 1)) as u16;
        let y = (number(p, "top", 0.) / maxy * f64::from(inner.height - 1))
            .clamp(0., f64::from(inner.height - 1)) as u16;
        f.buffer_mut()[(inner.x + x, inner.y + y)]
            .set_symbol("▪")
            .set_fg(t.accent);
    }
}
fn draw_modal(f: &mut Frame, app: &App, modal: Modal, t: Theme) {
    let area = f.area();
    let w = area.width.saturating_sub(6).min(104);
    let h = area.height.saturating_sub(4).min(30);
    let r = Rect::new((area.width - w) / 2, (area.height - h) / 2, w, h);
    f.render_widget(Clear, r);
    f.render_widget(Block::new().style(Style::default().bg(t.bg).fg(t.fg)), r);
    let (title, hint) = match &modal {
        Modal::Prompt(p) => (p.title.clone(), "Enter save · Esc cancel"),
        Modal::Menu { kind, .. } => (
            match kind {
                Menu::Pages => "Pages · n new / c clone / d delete",
                Menu::Streams => "Streams · Enter add / i inspect",
                Menu::Series => "Series · a add / e edit / d delete / Space legend",
                Menu::Panel => "Panel inspector",
                Menu::Page => "Page inspector",
            }
            .into(),
            "↑↓ select · Enter edit · Esc close",
        ),
        Modal::Info { title, .. } => (title.clone(), "↑↓ / PgUp / PgDn scroll · Esc close"),
        Modal::Confirm { title, .. } => (title.clone(), "Enter confirm · Esc cancel"),
        Modal::Result => ("Command result".into(), "Esc close"),
    };
    let block = Block::bordered()
        .title(format!(" {} ", safe(&title)))
        .title_bottom(hint)
        .border_style(Style::default().fg(t.accent));
    let inner = block.inner(r);
    f.render_widget(block, r);
    match modal {
        Modal::Prompt(p) => {
            let before: String = p.text.chars().take(p.cursor).collect();
            let cursor = before.width();
            let available = inner.width.saturating_sub(2) as usize;
            let skip = cursor.saturating_sub(available);
            let shown: String = p
                .text
                .chars()
                .scan(0, |width, c| {
                    let start = *width;
                    *width += c.to_string().width();
                    Some((start, c))
                })
                .filter(|(start, _)| *start >= skip)
                .map(|(_, c)| c)
                .collect();
            f.render_widget(
                Paragraph::new(vec![
                    Line::raw(""),
                    Line::raw(safe(&shown)),
                    Line::raw(""),
                    Line::styled(safe(&app.status), Style::default().fg(t.muted)),
                ])
                .wrap(Wrap { trim: false }),
                inner,
            );
            f.set_cursor_position((inner.x + (cursor - skip).min(available) as u16, inner.y + 1));
        }
        Modal::Menu { kind, index } => {
            let items: Vec<_> = app
                .menu_items(kind)
                .into_iter()
                .map(|s| ListItem::new(safe(&s)))
                .collect();
            let mut state = ListState::default().with_selected(Some(index));
            f.render_stateful_widget(
                List::new(items)
                    .highlight_style(Style::default().bg(t.selected).fg(t.accent))
                    .highlight_symbol("› "),
                inner,
                &mut state,
            );
        }
        Modal::Info { text, offset, .. } => f.render_widget(
            Paragraph::new(text.lines().map(|l| Line::raw(safe(l))).collect::<Vec<_>>())
                .scroll((offset, 0))
                .wrap(Wrap { trim: false }),
            inner,
        ),
        Modal::Confirm { .. } => f.render_widget(
            Paragraph::new("This removes the saved configuration.\nPress Enter to confirm."),
            inner,
        ),
        Modal::Result => f.render_widget(
            Paragraph::new(
                app.snapshot
                    .reply
                    .lines()
                    .map(|l| Line::raw(safe(l)))
                    .collect::<Vec<_>>(),
            )
            .wrap(Wrap { trim: false }),
            inner,
        ),
    }
}
pub fn plain(buffer: &Buffer) -> String {
    (buffer.area.y..buffer.area.bottom())
        .map(|y| {
            let mut line = String::new();
            let mut x = buffer.area.x;
            while x < buffer.area.right() {
                let symbol = buffer[(x, y)].symbol();
                line.push_str(symbol);
                x = x.saturating_add(symbol.width().max(1) as u16);
            }
            line.trim_end().to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};
    use serde_json::json;
    #[test]
    fn clips_negative_canvas_coordinates_and_retains_gaps() {
        let p = json!({"left":-80,"top":-16,"panel_width":640,"panel_height":320});
        let page = json!({"view_zoom":1,"view_x":0,"view_y":0});
        assert_eq!(
            panel_rect(&p, &page, Rect::new(4, 3, 100, 30)),
            Some(Rect::new(4, 3, 70, 19))
        );
        let points = vec![
            json!({"time":"1000","value":4}),
            json!({"time":"1001","value":null,"gap":true}),
            json!({"time":"1002","value":9}),
        ];
        assert_eq!(
            segments(&points),
            vec![vec![(1000., 4.)], vec![(1002., 9.)]]
        );
    }
    #[test]
    fn unicode_untrusted_logs_and_tiny_resizes_are_safe() {
        let mut app = App::new();
        app.snapshot.connected = true;
        app.snapshot.state = json!({"revision":3,"selected":"p","streams":[],"pages":[{"id":"p","name":"中文","title":"Monitor","theme":"dark","view_x":0,"view_y":0,"view_zoom":1,"active_panel":"log","panels":[{"id":"log","title":"Console","kind":"log","columns":["text"],"left":0,"top":0,"panel_width":1000,"panel_height":500}]}]});
        app.snapshot.panels.insert(
            "log".into(),
            json!({"rows":[{"text":"中文\u{1b}[2Jbad\u{7}"}]}),
        );
        for (w, h) in [(1, 1), (30, 10), (80, 24), (140, 36)] {
            let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
            terminal.draw(|f| draw(f, &mut app)).unwrap();
            let output = plain(terminal.backend().buffer());
            assert!(!output.contains('\u{1b}'));
            assert!(!output.contains('\u{7}'));
            if w == 140 {
                assert!(output.contains("中文\\u001b[2Jbad\\u0007"));
                assert!(output.lines().all(|line| line.width() <= w as usize));
            }
        }
    }
}
