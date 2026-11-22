//! Rendering for `terra --tui` (ratatui):
//! - Live curved heading-up Road View (±50 m / ±100 m) with real road edges, dashed centerline,
//!   draft slipstream wakes, attack alerts, and decluttered rider callouts at ~20 Hz.
//! - Filled grade-colored full-route elevation profile with all riders plotted along the course.

use ratatui::layout::{Alignment, Constraint, Flex, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::symbols::Marker;
use ratatui::text::{Line, Span};
use ratatui::widgets::canvas::{Canvas, Line as CLine, Points};
use ratatui::widgets::{Axis, Block, Chart, Clear, Dataset, GraphType, Paragraph, Row, Table, TableState, Wrap};
use ratatui::Frame;

use super::state::{App, Ego, Mode, RenderMode, Rider, RoadViewMode, RouteData, Screen};

const DIM: Color = Color::DarkGray;
const POWER: Color = Color::Yellow;
const HR: Color = Color::LightRed;
const CAD: Color = Color::LightCyan;
const SPEED: Color = Color::LightGreen;
const ELEV: Color = Color::LightMagenta;

/// Lateral offsets (m) relative to Biketerra's pathNode polyline (center of the right riding lane):
/// oncoming left lane spans [-12 m, -4 m], riding right lane spans [-4 m, +4 m], dashed road center at -4 m.
const ROAD_LEFT_OFF: f64 = -12.0;
const ROAD_CENTER_OFF: f64 = -4.0;
const ROAD_RIGHT_OFF: f64 = 4.0;
const ROAD_HALF_W: f64 = (ROAD_RIGHT_OFF - ROAD_LEFT_OFF) * 0.5; // 8.0 m (16 m total 2-lane width)

fn fmt(v: Option<f64>, dec: usize) -> String {
    v.map(|x| format!("{x:.dec$}")).unwrap_or_else(|| "—".into())
}

fn fmt_signed(v: Option<f64>, dec: usize) -> String {
    v.map(|x| format!("{x:+.dec$}")).unwrap_or_else(|| "—".into())
}

fn kmh(ms: Option<f64>) -> Option<f64> {
    ms.map(|v| v * 3.6)
}

fn km(m: Option<f64>) -> Option<f64> {
    m.map(|v| v / 1000.0)
}

/// Bright ridge / text color for a grade percentage.
fn grade_pct_color(pct: f64) -> Color {
    if pct >= 8.5 {
        Color::LightRed
    } else if pct >= 5.0 {
        Color::Rgb(255, 150, 60)
    } else if pct >= 2.0 {
        Color::Yellow
    } else if pct <= -2.0 {
        Color::LightCyan
    } else {
        Color::White
    }
}

/// Solid, clearly visible fill color underneath the elevation profile curve.
fn grade_fill_color(pct: f64) -> Color {
    if pct >= 8.5 {
        Color::Rgb(170, 46, 46)
    } else if pct >= 5.0 {
        Color::Rgb(175, 90, 36)
    } else if pct >= 2.0 {
        Color::Rgb(155, 130, 42)
    } else if pct <= -2.0 {
        Color::Rgb(50, 125, 150)
    } else {
        Color::Rgb(112, 120, 136)
    }
}

fn grade_color(g: Option<f64>) -> Color {
    g.map(|v| grade_pct_color(v * 100.0)).unwrap_or(Color::White)
}

fn rider_color(r: &Rider) -> Color {
    if r.attacking {
        Color::LightRed
    } else if let Some((cr, cg, cb)) = r.color {
        Color::Rgb(cr, cg, cb)
    } else {
        Color::Cyan
    }
}

fn short_name(name: &str, max_chars: usize) -> String {
    let first = name.split_whitespace().next().unwrap_or(name);
    let mut out: String = first.chars().take(max_chars).collect();
    if out.is_empty() {
        out = "?".into();
    }
    out
}

pub fn draw(f: &mut Frame, app: &App) {
    let [header, body, footer] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0), Constraint::Length(1)]).areas(f.area());

    draw_header(f, header, app);
    match app.mode() {
        Mode::Ride => draw_ride(f, body, app),
        Mode::Spectate => draw_spectate(f, body, app),
        Mode::Idle => draw_idle(f, body, app),
    }
    draw_footer(f, footer, app);

    if app.show_help {
        draw_help(f, f.area());
    }
}

// ---------------------------------------------------------------- chrome

fn draw_header(f: &mut Frame, area: Rect, app: &App) {
    let (label, color) = match app.mode() {
        Mode::Ride => (" RIDE ", Color::Green),
        Mode::Spectate => (" SPECTATE ", Color::Magenta),
        Mode::Idle => (" IDLE ", Color::DarkGray),
    };
    let mut spans = vec![
        Span::styled(" TERRA ", Style::new().bold().fg(Color::Black).bg(Color::White)),
        Span::styled(label, Style::new().bold().fg(Color::Black).bg(color)),
    ];
    if app.screen != Screen::Auto {
        spans.push(Span::styled(" manual ", Style::new().fg(Color::Black).bg(Color::Yellow)));
    }
    spans.push(Span::raw(" "));
    if app.connected {
        spans.push(Span::styled("● ", Style::new().fg(Color::Green)));
    } else {
        spans.push(Span::styled("○ ", Style::new().fg(Color::Red)));
    }

    let (gpu_label, gpu_col) = match (app.render_mode, app.effective_render.as_str()) {
        (RenderMode::Auto, "off") => ("3D:OFF(auto)", Color::Cyan),
        (RenderMode::Off, _) => ("3D:OFF", Color::Cyan),
        (RenderMode::OneFps, _) | (_, "1fps") => ("3D:1fps@0.1x", Color::Yellow),
        _ => ("3D:FULL", Color::DarkGray),
    };
    spans.push(Span::styled(gpu_label, Style::new().fg(gpu_col)));
    if app.canvas_w > 0 && app.canvas_h > 0 && app.effective_render != "full" {
        spans.push(Span::styled(format!(" {}×{}", app.canvas_w, app.canvas_h), Style::new().fg(DIM)));
    }
    spans.push(Span::styled(
        if app.window_hidden { " [hidden] " } else { " [win] " },
        Style::new().fg(DIM),
    ));

    if app.bt_app_ready {
        let hz = app.world_hz();
        spans.push(Span::styled(format!("btApp {hz:.0}Hz "), Style::new().fg(Color::Green)));
    }

    match app.frame_age_secs() {
        Some(a) if a < 5.0 => spans.push(Span::styled(format!("live {a:.1}s"), Style::new().fg(Color::Green))),
        Some(a) => spans.push(Span::styled(format!("stale {a:.0}s"), Style::new().fg(Color::Yellow))),
        None => spans.push(Span::styled("no frames", Style::new().fg(DIM))),
    }

    if let Some(rt) = &app.route {
        spans.push(Span::raw("  "));
        let rname = if rt.name.is_empty() { "Route" } else { rt.name.as_str() };
        spans.push(Span::styled(
            format!("{rname} · {:.1} km ↑{:.0}m", rt.total_len / 1000.0, rt.total_climb),
            Style::new().fg(Color::White),
        ));
    } else {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(app.path().to_string(), Style::new().fg(Color::Gray)));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_footer(f: &mut Frame, area: Rect, app: &App) {
    if let Some((msg, at)) = &app.toast {
        if at.elapsed().as_secs_f64() < 3.0 {
            f.render_widget(Paragraph::new(Span::styled(format!(" {msg}"), Style::new().fg(Color::Yellow))), area);
            return;
        }
    }
    let keys: &[(&str, &str)] = &[
        ("q", "quit"),
        ("1", "ride"),
        ("2", "spectate"),
        ("a", "auto"),
        ("↑↓", "rider"),
        ("↵", "focus"),
        ("m", "±25/50/100m/map"),
        ("g", "3D mode"),
        ("w", "browser"),
        ("?", "help"),
    ];
    let mut spans = vec![Span::raw(" ")];
    for (k, d) in keys {
        spans.push(Span::styled(*k, Style::new().bold().fg(Color::White)));
        spans.push(Span::styled(format!(" {d}  "), Style::new().fg(DIM)));
    }
    if let Some(v) = &app.backend_version {
        spans.push(Span::styled(format!("v{v}"), Style::new().fg(DIM)));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_help(f: &mut Frame, area: Rect) {
    let [area] = Layout::horizontal([Constraint::Length(64)]).flex(Flex::Center).areas(area);
    let [area] = Layout::vertical([Constraint::Length(20)]).flex(Flex::Center).areas(area);
    let rows = [
        ("q / Esc", "quit the TUI (Terra keeps running if --attach)"),
        ("1", "force the ride screen"),
        ("2", "force the spectate screen"),
        ("a / 0", "auto: follow the page (ride / spectate / idle)"),
        ("↑ ↓ / j k", "select a rider (PgUp/PgDn/Home/End)"),
        ("Enter", "switch Biketerra focal/spectated rider to selected"),
        ("m", "cycle road view: ±25 m → ±50 m → ±100 m → full map"),
        ("g", "cycle 3D canvas: auto → off → 1fps@0.1x → full"),
        ("w", "show / hide the Terra browser window (sim stays alive)"),
        ("r", "reload the page"),
        ("h", "open biketerra.com home page"),
        ("?", "this help"),
    ];
    let mut lines = vec![Line::raw("")];
    for (k, d) in rows {
        lines.push(Line::from(vec![
            Span::styled(format!("  {k:<11}"), Style::new().bold()),
            Span::raw(d),
        ]));
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        "  Road view updates at ~20 Hz; ⚡ flags attacking/surging riders.",
        Style::new().fg(DIM),
    ));
    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(lines).block(Block::bordered().title(" Terra TUI · keys ").border_style(Style::new().fg(Color::Cyan))),
        area,
    );
}

fn message(f: &mut Frame, area: Rect, title: &str, lines: Vec<Line>) {
    let block = Block::bordered().title(format!(" {title} ")).border_style(Style::new().fg(DIM));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let h = lines.len() as u16;
    let [mid] = Layout::vertical([Constraint::Length(h)]).flex(Flex::Center).areas(inner);
    f.render_widget(Paragraph::new(lines).alignment(Alignment::Center).wrap(Wrap { trim: true }), mid);
}

fn draw_idle(f: &mut Frame, area: Rect, app: &App) {
    let mut lines = vec![
        Line::styled("Not riding or spectating.", Style::new().bold()),
        Line::raw(""),
        Line::raw("Start or spectate a ride in the Terra window — then press w to hide it."),
        Line::raw("3D rendering shuts off automatically while hidden and the ride keeps running."),
        Line::raw(""),
        Line::styled("w  show/hide browser   ·   g  3D mode   ·   h  home   ·   1/2  force screen", Style::new().fg(DIM)),
    ];
    if !app.connected {
        lines.push(Line::raw(""));
        lines.push(Line::styled(app.status.clone(), Style::new().fg(Color::Yellow)));
        lines.push(Line::styled("Start Terra first (./tui.sh does both).", Style::new().fg(DIM)));
    }
    message(f, area, "Terra", lines);
}

// ---------------------------------------------------------------- widgets

/// Compact 4-line stat tile: title on border, bold value + unit, dim sub-line.
fn tile(f: &mut Frame, area: Rect, title: &str, value: String, unit: &str, sub: String, color: Color) {
    let block = Block::bordered()
        .title(Span::styled(format!(" {title} "), Style::new().fg(DIM)))
        .border_style(Style::new().fg(DIM));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let mut lines = Vec::new();
    if inner.height >= 3 {
        lines.push(Line::raw(""));
    }
    lines.push(Line::from(vec![
        Span::styled(value, Style::new().fg(color).add_modifier(Modifier::BOLD)),
        Span::styled(format!(" {unit}"), Style::new().fg(DIM)),
    ]));
    if inner.height >= 2 {
        lines.push(Line::styled(sub, Style::new().fg(Color::Gray)));
    }
    f.render_widget(Paragraph::new(lines).alignment(Alignment::Center), inner);
}

/// Line chart of `values` (oldest → newest), x = sample index.
fn series_chart(f: &mut Frame, area: Rect, title: String, values: &[f64], color: Color, floor_zero: bool) {
    let block = Block::bordered()
        .title(Span::styled(title, Style::new().fg(color)))
        .border_style(Style::new().fg(DIM));
    if values.len() < 2 {
        let inner = block.inner(area);
        f.render_widget(block, area);
        f.render_widget(Paragraph::new(Span::styled("collecting…", Style::new().fg(DIM))).alignment(Alignment::Center), inner);
        return;
    }
    let pts: Vec<(f64, f64)> = values.iter().enumerate().map(|(i, v)| (i as f64, *v)).collect();
    let max = values.iter().cloned().fold(f64::MIN, f64::max);
    let min = values.iter().cloned().fold(f64::MAX, f64::min);
    let (lo, hi) = if floor_zero {
        (0.0, (max * 1.1).max(10.0))
    } else {
        let pad = ((max - min) * 0.1).max(2.0);
        (min - pad, max + pad)
    };
    let ds = Dataset::default().marker(Marker::Braille).graph_type(GraphType::Line).style(Style::new().fg(color)).data(&pts);
    let chart = Chart::new(vec![ds])
        .block(block)
        .x_axis(Axis::default().bounds([0.0, (pts.len() - 1) as f64]))
        .y_axis(
            Axis::default()
                .bounds([lo, hi])
                .labels(vec![format!("{lo:.0}"), format!("{hi:.0}")])
                .style(Style::new().fg(DIM)),
        );
    f.render_widget(chart, area);
}

// ---------------------------------------------------------------- live curved road view

fn draw_road_or_map(f: &mut Frame, area: Rect, app: &App, highlight_id: Option<&str>) {
    match app.road_view {
        RoadViewMode::Road25 => draw_road_canvas(f, area, app, 25.0, highlight_id),
        RoadViewMode::Road50 => draw_road_canvas(f, area, app, 50.0, highlight_id),
        RoadViewMode::Road100 => draw_road_canvas(f, area, app, 100.0, highlight_id),
        RoadViewMode::FullMap => route_map_canvas(f, area, app, highlight_id),
    }
}

/// High-refresh heading-up curved road view showing `[-range_m, +range_m]` around the focal/ego rider.
fn draw_road_canvas(f: &mut Frame, area: Rect, app: &App, range_m: f64, highlight_id: Option<&str>) {
    if app.road.len() < 2 {
        let block = Block::bordered()
            .title(Span::styled(format!(" Road · ±{range_m:.0}m "), Style::new().fg(Color::Cyan)))
            .border_style(Style::new().fg(DIM));
        let inner = block.inner(area);
        f.render_widget(block, area);
        f.render_widget(
            Paragraph::new(Span::styled("waiting for btApp road geometry…", Style::new().fg(DIM)))
                .alignment(Alignment::Center),
            inner,
        );
        return;
    }

    let focal = app.focal_or_ego_rider();
    let focal_rx = focal.and_then(|r| r.rx).unwrap_or(0.0);
    let focal_rf = focal.and_then(|r| r.rf).unwrap_or(0.0);
    let draft_pct = focal
        .and_then(|r| r.draft_pct())
        .or_else(|| app.ego.as_ref().and_then(|e| e.draft).map(|d| ((1.0 - d) * 100.0).clamp(0.0, 100.0)))
        .unwrap_or(0.0);

    // Filter road ribbon slices within the visible range (with 12m margin so edges extend cleanly)
    let visible_road: Vec<_> = app
        .road
        .iter()
        .filter(|s| s.rel_dist >= -(range_m + 12.0) && s.rel_dist <= (range_m + 12.0))
        .copied()
        .collect();

    let inner_rows = area.height.saturating_sub(2).max(1) as usize;
    let inner_cols = area.width.saturating_sub(2).max(1) as usize;

    // Constant scale derived strictly from range_m and pane dimensions (never auto-zooms on curves!).
    // Terminal cells are ~2x taller than wide, so physical aspect = inner_cols / (inner_rows * 2).
    // 1:1 isotropic scale keeps the 16m 2-lane road a constant visual width through every corner,
    // centered on the road's visual midpoint (ROAD_CENTER_OFF = -4m).
    let aspect = (inner_cols as f64) / ((inner_rows as f64) * 2.0);
    let min_road_cols = if range_m <= 30.0 {
        12.0
    } else if range_m <= 60.0 {
        8.0
    } else {
        5.0
    };
    let max_half_x = (ROAD_HALF_W * (inner_cols as f64)) / min_road_cols;
    let half_x = (range_m * aspect).min(max_half_x).max(18.0);
    let x_lo = ROAD_CENTER_OFF - half_x;
    let x_hi = ROAD_CENTER_OFF + half_x;
    let x_bounds = [x_lo, x_hi];
    let y_lo = -range_m;
    let y_hi = range_m;
    let y_bounds = [y_lo, y_hi];
    let m_per_col = (2.0 * half_x) / (inner_cols as f64);

    // Collect riders visible in the road window
    let in_view: Vec<&Rider> = app
        .riders
        .iter()
        .filter(|r| {
            let (Some(rx), Some(rf)) = (r.rx, r.rf) else { return false };
            rf >= y_lo - 4.0 && rf <= y_hi + 4.0 && rx >= x_lo && rx <= x_hi
        })
        .collect();

    let attacks = in_view.iter().filter(|r| r.attacking && !r.is_ego).count();

    let mut title_spans = vec![
        Span::styled(format!(" Road ±{range_m:.0}m "), Style::new().bold().fg(Color::Cyan)),
        Span::styled(format!("· {} in view ", in_view.len()), Style::new().fg(Color::Gray)),
    ];
    if draft_pct >= 5.0 {
        title_spans.push(Span::styled(
            format!("· DRAFT {draft_pct:.0}% "),
            Style::new().bold().fg(Color::Black).bg(Color::Cyan),
        ));
    }
    if attacks > 0 {
        title_spans.push(Span::styled(
            format!(" ⚡{attacks} ATTACK "),
            Style::new().bold().fg(Color::White).bg(Color::Red),
        ));
    }
    let hz = app.world_hz();
    if hz >= 1.0 {
        title_spans.push(Span::styled(format!("· {hz:.0}Hz "), Style::new().fg(DIM)));
    }

    let canvas = Canvas::default()
        .block(Block::bordered().title(Line::from(title_spans)).border_style(Style::new().fg(DIM)))
        .marker(Marker::Braille)
        .x_bounds(x_bounds)
        .y_bounds(y_bounds)
        .paint(|ctx| {
            // 1. Distance tick lines & labels on the left margin
            let tick_step = if range_m <= 30.0 {
                10.0
            } else if range_m <= 60.0 {
                25.0
            } else {
                50.0
            };
            let max_k = ((range_m - 1.0) / tick_step).floor() as i32;
            for k in -max_k..=max_k {
                let m = (k as f64) * tick_step;
                // Subtle guide dots across the canvas at each distance mark
                let mut gx = x_lo + 4.0;
                let mut guide_pts = Vec::new();
                while gx < x_hi - 1.0 {
                    guide_pts.push((gx, m));
                    gx += half_x * 0.16;
                }
                ctx.draw(&Points { coords: &guide_pts, color: Color::Rgb(45, 50, 62) });
                let lbl = if m.abs() < 0.5 {
                    "0m".to_string()
                } else {
                    format!("{m:+.0}m")
                };
                ctx.print(x_lo + 0.4, m, Span::styled(lbl, Style::new().fg(Color::Rgb(90, 98, 115))));
            }

            // 2. Actual curved 2-lane road edges (-12m left, +4m right) + dashed centerline (-4m)
            for (i, w) in visible_road.windows(2).enumerate() {
                let a = w[0];
                let b = w[1];
                let avg_g = (a.grade + b.grade) * 0.5;
                let edge_col = if avg_g >= 7.5 {
                    Color::LightRed
                } else if avg_g >= 4.0 {
                    Color::Yellow
                } else {
                    Color::Rgb(165, 175, 190)
                };

                // Left road edge (outer edge of oncoming left lane)
                ctx.draw(&CLine {
                    x1: a.cx + a.nr * ROAD_LEFT_OFF,
                    y1: a.cf + a.nf * ROAD_LEFT_OFF,
                    x2: b.cx + b.nr * ROAD_LEFT_OFF,
                    y2: b.cf + b.nf * ROAD_LEFT_OFF,
                    color: edge_col,
                });
                // Right road edge (outer edge of riding right lane)
                ctx.draw(&CLine {
                    x1: a.cx + a.nr * ROAD_RIGHT_OFF,
                    y1: a.cf + a.nf * ROAD_RIGHT_OFF,
                    x2: b.cx + b.nr * ROAD_RIGHT_OFF,
                    y2: b.cf + b.nf * ROAD_RIGHT_OFF,
                    color: edge_col,
                });
                // Dashed road centerline dividing oncoming left lane and riding right lane
                if i % 2 == 0 {
                    ctx.draw(&CLine {
                        x1: a.cx + a.nr * ROAD_CENTER_OFF,
                        y1: a.cf + a.nf * ROAD_CENTER_OFF,
                        x2: b.cx + b.nr * ROAD_CENTER_OFF,
                        y2: b.cf + b.nf * ROAD_CENTER_OFF,
                        color: Color::Rgb(110, 118, 134),
                    });
                }
            }

            // 3. Draft slipstream wakes behind riders directly ahead of focal (within 1.5..15m)
            for r in &in_view {
                if r.is_focal || r.is_ego {
                    continue;
                }
                let (Some(rx), Some(rf)) = (r.rx, r.rf) else { continue };
                let ahead = rf - focal_rf;
                let lat_diff = (rx - focal_rx).abs();
                if ahead > 1.2 && ahead < 16.0 && lat_diff < 2.4 {
                    let wake_len = ahead.min(9.0);
                    let col = if draft_pct >= 10.0 { Color::Cyan } else { Color::Rgb(50, 110, 130) };
                    ctx.draw(&CLine {
                        x1: rx,
                        y1: rf - 0.8,
                        x2: rx,
                        y2: (rf - wake_len).max(focal_rf + 0.5),
                        color: col,
                    });
                }
            }

            // 4. Plot every rider on the road + prioritized non-overlapping labels
            // Sort label priority: attacking first, then selected, then closest to focal
            let mut label_order: Vec<&Rider> = in_view.iter().copied().collect();
            label_order.sort_by(|a, b| {
                let pa = if a.is_focal || a.is_ego {
                    0
                } else if a.attacking {
                    1
                } else if highlight_id.is_some_and(|id| id == a.id) {
                    2
                } else {
                    3
                };
                let pb = if b.is_focal || b.is_ego {
                    0
                } else if b.attacking {
                    1
                } else if highlight_id.is_some_and(|id| id == b.id) {
                    2
                } else {
                    3
                };
                pa.cmp(&pb).then_with(|| {
                    let da = a.rf.unwrap_or(999.0).abs();
                    let db = b.rf.unwrap_or(999.0).abs();
                    da.total_cmp(&db)
                })
            });

            // Draw rider dots first so markers/labels sit crisply on top
            for r in &in_view {
                let (Some(rx), Some(rf)) = (r.rx, r.rf) else { continue };
                if r.is_focal || r.is_ego || r.attacking || highlight_id.is_some_and(|id| id == r.id) {
                    continue;
                }
                let col = rider_color(r);
                ctx.draw(&Points { coords: &[(rx, rf)], color: col });
                ctx.print(rx, rf, Span::styled("●", Style::new().fg(col)));
            }

            // Track used vertical rows (left & right label columns) to keep labels uncluttered
            let mut used_right = vec![false; inner_rows + 2];
            let mut used_left = vec![false; inner_rows + 2];
            let row_of = |rf: f64| -> usize {
                let frac = ((y_hi - rf) / (y_hi - y_lo)).clamp(0.0, 1.0);
                (frac * inner_rows.saturating_sub(1) as f64) as usize
            };
            let road_cx_at = |rf: f64| -> f64 {
                visible_road
                    .iter()
                    .min_by(|a, b| (a.cf - rf).abs().total_cmp(&(b.cf - rf).abs()))
                    .map(|s| s.cx)
                    .unwrap_or(0.0)
            };

            for r in &label_order {
                let (Some(rx), Some(rf)) = (r.rx, r.rf) else { continue };
                let is_hi = highlight_id.is_some_and(|id| id == r.id);
                let is_me = r.is_ego || r.is_focal;

                // Prominent symbol for focal, selected, or attacking rider
                if is_me {
                    ctx.print(rx, rf, Span::styled("▲", Style::new().bold().fg(Color::LightGreen)));
                } else if is_hi {
                    ctx.print(rx, rf, Span::styled("◆", Style::new().bold().fg(Color::LightMagenta)));
                } else if r.attacking {
                    ctx.print(rx, rf, Span::styled("⚡", Style::new().bold().fg(Color::LightRed)));
                }

                // Compact callout label beside the rider (exact terminal row collision check)
                let row = row_of(rf).min(inner_rows);
                let rcx = road_cx_at(rf);
                let try_right = rx >= rcx + ROAD_CENTER_OFF;
                let place_right = if try_right && !used_right[row] {
                    used_right[row] = true;
                    true
                } else if !try_right && !used_left[row] {
                    used_left[row] = true;
                    false
                } else if !used_right[row] {
                    used_right[row] = true;
                    true
                } else if !used_left[row] {
                    used_left[row] = true;
                    false
                } else {
                    continue;
                };

                let pwr = r.power.map(|p| format!("{p:.0}W")).unwrap_or_default();
                let label_text = if is_me {
                    let tag = if r.is_ego { "YOU" } else { "FOCAL" };
                    if draft_pct >= 8.0 {
                        format!("{tag} {pwr} D{draft_pct:.0}%")
                    } else {
                        format!("{tag} {pwr}")
                    }
                } else if r.attacking {
                    let wkg = r.wkg.map(|w| format!("{w:.1}w/k")).unwrap_or(pwr);
                    format!("⚡{} {}", short_name(&r.name, 7), wkg)
                } else {
                    format!("{} {pwr}", short_name(&r.name, 8))
                };

                let style = if is_me {
                    Style::new().bold().fg(Color::LightGreen)
                } else if r.attacking {
                    Style::new().bold().fg(Color::LightRed)
                } else if is_hi {
                    Style::new().bold().fg(Color::LightMagenta)
                } else {
                    Style::new().fg(Color::Gray)
                };

                // Anchor label just outside the curved 2-lane road edge using actual cell width
                let label_w_m = (label_text.chars().count() as f64 + 1.0) * m_per_col;
                let lx = if place_right {
                    (rcx + ROAD_RIGHT_OFF + 1.5 * m_per_col)
                        .max(rx + 2.0 * m_per_col)
                        .min(x_hi - label_w_m)
                } else {
                    (rcx + ROAD_LEFT_OFF - label_w_m - 1.0 * m_per_col)
                        .min(rx - label_w_m - 1.5 * m_per_col)
                        .max(x_lo + 5.5 * m_per_col)
                };
                ctx.print(lx, rf, Span::styled(label_text, style));
            }
        });

    f.render_widget(canvas, area);
}

// ---------------------------------------------------------------- filled elevation profile

/// Full route elevation profile with solid grade-colored Braille fill underneath + all riders.
fn route_elevation_canvas(f: &mut Frame, area: Rect, app: &App, highlight_id: Option<&str>) {
    let Some(route) = &app.route else {
        fallback_elevation_chart(f, area, app);
        return;
    };

    let total_km = (route.total_len / 1000.0).max(0.1);
    // Use at least a 120m vertical scale so flat routes look flat instead of jagged peaks,
    // and keep a solid ~25% ground band below min_elev so flat sections always show visible fill.
    let raw_span = (route.max_elev - route.min_elev).max(0.0);
    let display_span = raw_span.max(120.0);
    let y_lo = route.min_elev - display_span * 0.25;
    let y_hi = y_lo + display_span * 1.40;

    let grade_str = app
        .ego
        .as_ref()
        .and_then(|e| e.grade)
        .map(|g| format!("{:.1}%", g * 100.0))
        .unwrap_or_else(|| "—".into());
    let title = format!(
        " Elevation · {:.1} km · {:.0}–{:.0}m · {} ",
        total_km, route.min_elev, route.max_elev, grade_str
    );

    // 2 Braille sub-columns per character cell → dense seamless area fill
    let sub_cols = (area.width.saturating_sub(2).max(4) as usize) * 2;

    let canvas = Canvas::default()
        .block(Block::bordered().title(Span::styled(title, Style::new().fg(ELEV))).border_style(Style::new().fg(DIM)))
        .marker(Marker::Braille)
        .x_bounds([0.0, total_km])
        .y_bounds([y_lo, y_hi])
        .paint(|ctx| {
            // 1. Solid grade-colored area fill underneath the elevation profile
            for c in 0..sub_cols {
                let frac = (c as f64 + 0.5) / sub_cols as f64;
                let d_m = frac * route.total_len;
                let (elev, grade) = route.sample_at(d_m);
                let x_km = d_m / 1000.0;
                ctx.draw(&CLine {
                    x1: x_km,
                    y1: y_lo,
                    x2: x_km,
                    y2: elev,
                    color: grade_fill_color(grade),
                });
            }

            // 2. Bright top ridge polyline for crisp contrast
            for w in route.points.windows(2) {
                let a = &w[0];
                let b = &w[1];
                let col = grade_pct_color((a.grade + b.grade) * 0.5);
                ctx.draw(&CLine {
                    x1: a.dist / 1000.0,
                    y1: a.elev,
                    x2: b.dist / 1000.0,
                    y2: b.elev,
                    color: col,
                });
            }

            // 3. Plot all other riders along the elevation crest
            let mut others: Vec<(f64, f64)> = Vec::new();
            let mut attackers: Vec<(f64, f64)> = Vec::new();
            for r in &app.riders {
                let Some(d) = r.dist else { continue };
                let is_hi = highlight_id.is_some_and(|id| id == r.id);
                if is_hi || r.is_ego || r.is_focal {
                    continue;
                }
                let d_mod = if route.is_loop && route.total_len > 0.0 {
                    d.rem_euclid(route.total_len)
                } else {
                    d.clamp(0.0, route.total_len)
                };
                let x = d_mod / 1000.0;
                let y = route.elev_at(d_mod);
                if r.attacking {
                    attackers.push((x, y));
                } else {
                    others.push((x, y));
                }
            }
            if !others.is_empty() {
                ctx.draw(&Points { coords: &others, color: Color::White });
            }
            if !attackers.is_empty() {
                ctx.draw(&Points { coords: &attackers, color: Color::LightRed });
            }

            // 4. Focal / ego cursor
            if let Some(focal) = app.focal_or_ego_rider() {
                if let Some(d) = focal.dist {
                    let d_mod = if route.is_loop && route.total_len > 0.0 {
                        d.rem_euclid(route.total_len)
                    } else {
                        d.clamp(0.0, route.total_len)
                    };
                    let x = d_mod / 1000.0;
                    let y = route.elev_at(d_mod);
                    ctx.draw(&CLine { x1: x, y1: y_lo, x2: x, y2: y, color: Color::LightGreen });
                    ctx.print(x, y, Span::styled("★", Style::new().bold().fg(Color::LightGreen)));
                }
            }

            // 5. Highlighted / selected rider (spectate)
            if let Some(hi_id) = highlight_id {
                if let Some(r) = app.riders.iter().find(|r| r.id == hi_id) {
                    if let Some(d) = r.dist {
                        let d_mod = if route.is_loop && route.total_len > 0.0 {
                            d.rem_euclid(route.total_len)
                        } else {
                            d.clamp(0.0, route.total_len)
                        };
                        let x = d_mod / 1000.0;
                        let y = route.elev_at(d_mod);
                        ctx.draw(&CLine { x1: x, y1: y_lo, x2: x, y2: y, color: Color::LightMagenta });
                        ctx.print(x, y, Span::styled("◆", Style::new().bold().fg(Color::LightMagenta)));
                    }
                }
            }
        });
    f.render_widget(canvas, area);
}

/// Fallback when full route geometry isn't loaded yet: plot filled recorded ego elevation.
fn fallback_elevation_chart(f: &mut Frame, area: Rect, app: &App) {
    let pts: Vec<(f64, f64)> = app
        .ego_hist
        .iter()
        .filter_map(|e| Some((e.distance? / 1000.0, e.elevation?)))
        .collect();
    let grade = app.ego.as_ref().and_then(|e| e.grade);
    let title = format!(" Elevation · {} % (waiting for btApp route) ", fmt(grade.map(|g| g * 100.0), 1));
    let block = Block::bordered().title(Span::styled(title, Style::new().fg(ELEV))).border_style(Style::new().fg(DIM));
    if pts.len() < 2 {
        let inner = block.inner(area);
        f.render_widget(block, area);
        f.render_widget(Paragraph::new(Span::styled("waiting for route…", Style::new().fg(DIM))).alignment(Alignment::Center), inner);
        return;
    }
    let (x0, x1) = pts.iter().fold((f64::MAX, f64::MIN), |(a, b), p| (a.min(p.0), b.max(p.0)));
    let (y0, y1) = pts.iter().fold((f64::MAX, f64::MIN), |(a, b), p| (a.min(p.1), b.max(p.1)));
    let x1 = if x1 - x0 < 0.01 { x0 + 0.01 } else { x1 };
    let display_span = (y1 - y0).max(120.0);
    let y_lo = y0 - display_span * 0.25;
    let y_hi = y_lo + display_span * 1.40;

    let canvas = Canvas::default()
        .block(block)
        .marker(Marker::Braille)
        .x_bounds([x0, x1])
        .y_bounds([y_lo, y_hi])
        .paint(|ctx| {
            for p in &pts {
                ctx.draw(&CLine { x1: p.0, y1: y_lo, x2: p.0, y2: p.1, color: Color::Rgb(112, 120, 136) });
            }
            for w in pts.windows(2) {
                ctx.draw(&CLine { x1: w[0].0, y1: w[0].1, x2: w[1].0, y2: w[1].1, color: ELEV });
            }
        });
    f.render_widget(canvas, area);
}

// ---------------------------------------------------------------- full route 2D map (when m cycles to FullMap)

fn route_map_canvas(f: &mut Frame, area: Rect, app: &App, highlight_id: Option<&str>) {
    let Some(route) = &app.route else {
        let block = Block::bordered().title(Span::styled(" Full Route Map ", Style::new().fg(Color::Cyan))).border_style(Style::new().fg(DIM));
        let inner = block.inner(area);
        f.render_widget(block, area);
        f.render_widget(Paragraph::new(Span::styled("waiting for btApp route…", Style::new().fg(DIM))).alignment(Alignment::Center), inner);
        return;
    };

    let (x_bounds, y_bounds) = compute_full_map_bounds(route, area);
    let title = format!(" Full Route Map · {} riders (m: road) ", app.riders.len());

    let canvas = Canvas::default()
        .block(Block::bordered().title(Span::styled(title, Style::new().fg(Color::Cyan))).border_style(Style::new().fg(DIM)))
        .marker(Marker::Braille)
        .x_bounds(x_bounds)
        .y_bounds(y_bounds)
        .paint(|ctx| {
            for w in route.points.windows(2) {
                ctx.draw(&CLine {
                    x1: w[0].x,
                    y1: -w[0].z,
                    x2: w[1].x,
                    y2: -w[1].z,
                    color: Color::DarkGray,
                });
            }
            if let Some(p0) = route.points.first() {
                ctx.print(p0.x, -p0.z, Span::styled("S", Style::new().fg(Color::White)));
            }
            for r in &app.riders {
                let (Some(x), Some(z)) = (r.x, r.z) else { continue };
                if highlight_id.is_some_and(|id| id == r.id) || r.is_ego || r.is_focal {
                    continue;
                }
                ctx.draw(&Points { coords: &[(x, -z)], color: rider_color(r) });
            }
            if let Some(focal) = app.focal_or_ego_rider() {
                if let (Some(x), Some(z)) = (focal.x, focal.z) {
                    ctx.print(x, -z, Span::styled("★", Style::new().bold().fg(Color::LightGreen)));
                }
            }
            if let Some(hi_id) = highlight_id {
                if let Some(r) = app.riders.iter().find(|r| r.id == hi_id) {
                    if let (Some(x), Some(z)) = (r.x, r.z) {
                        ctx.print(x, -z, Span::styled("◆", Style::new().bold().fg(Color::LightMagenta)));
                    }
                }
            }
        });
    f.render_widget(canvas, area);
}

fn compute_full_map_bounds(route: &RouteData, area: Rect) -> ([f64; 2], [f64; 2]) {
    let w_cells = area.width.saturating_sub(2).max(4) as f64;
    let h_cells = area.height.saturating_sub(2).max(4) as f64;
    let aspect = w_cells / (h_cells * 2.0);

    let (mut min_x, mut max_x, mut min_y, mut max_y) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
    for p in &route.points {
        let y = -p.z;
        if p.x < min_x { min_x = p.x; }
        if p.x > max_x { max_x = p.x; }
        if y < min_y { min_y = y; }
        if y > max_y { max_y = y; }
    }
    let cx = (min_x + max_x) * 0.5;
    let cy = (min_y + max_y) * 0.5;
    let mut span_x = (max_x - min_x).max(50.0) * 1.14;
    let mut span_y = (max_y - min_y).max(50.0) * 1.14;
    if span_x / span_y < aspect {
        span_x = span_y * aspect;
    } else {
        span_y = span_x / aspect;
    }
    ([cx - span_x * 0.5, cx + span_x * 0.5], [cy - span_y * 0.5, cy + span_y * 0.5])
}

// ---------------------------------------------------------------- rider table

fn fmt_rider_pos(r: &Rider) -> String {
    if let Some(dd) = r.dist_delta {
        if dd.abs() >= 1000.0 {
            format!("{:+.1}k", dd / 1000.0)
        } else {
            format!("{dd:+.0}m")
        }
    } else if let Some(g) = r.gap {
        fmt_signed(Some(g), 1)
    } else if let Some(d) = r.dist {
        format!("{:.1}k", d / 1000.0)
    } else {
        "—".into()
    }
}

fn rider_row(r: &Rider) -> Row<'static> {
    let prefix = if r.is_ego {
        "▲ "
    } else if r.attacking {
        "⚡"
    } else if r.is_focal {
        "◉ "
    } else {
        "  "
    };
    let sport_tag = if r.sport == 1 { " (Rowing)" } else { "" };
    let row = Row::new(vec![
        format!("{prefix}{}{sport_tag}", r.name),
        fmt_rider_pos(r),
        fmt(r.power, 0),
        fmt(r.wkg, 1),
        fmt(kmh(r.speed), 1),
        fmt(r.hr, 0),
    ]);
    if r.is_ego {
        row.style(Style::new().fg(Color::LightGreen).add_modifier(Modifier::BOLD))
    } else if r.attacking {
        row.style(Style::new().fg(Color::LightRed).add_modifier(Modifier::BOLD))
    } else if r.is_focal {
        row.style(Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD))
    } else {
        row
    }
}

const RIDER_WIDTHS: [Constraint; 6] = [
    Constraint::Min(14),
    Constraint::Length(7),
    Constraint::Length(5),
    Constraint::Length(5),
    Constraint::Length(5),
    Constraint::Length(4),
];

fn rider_header() -> Row<'static> {
    Row::new(vec!["Rider", "Δ/Gap", "W", "W/kg", "km/h", "HR"]).style(Style::new().fg(DIM).add_modifier(Modifier::BOLD))
}

// ---------------------------------------------------------------- ride screen

fn draw_ride(f: &mut Frame, area: Rect, app: &App) {
    let Some(ego) = app.ego.as_ref() else {
        message(
            f,
            area,
            "Ride",
            vec![
                Line::styled("Waiting for ride data…", Style::new().bold()),
                Line::raw(""),
                Line::raw("Biketerra broadcasts once the ride is running."),
                Line::styled("w  show browser   ·   g  3D mode", Style::new().fg(DIM)),
            ],
        );
        return;
    };

    let [row1, row2, rest] =
        Layout::vertical([Constraint::Length(4), Constraint::Length(4), Constraint::Min(10)]).areas(area);
    ride_tiles(f, row1, row2, ego);

    // Big full-height Road View on the left; Filled Elevation + Riders/Graphs on the right
    let [road_area, right] = Layout::horizontal([Constraint::Percentage(44), Constraint::Percentage(56)]).areas(rest);
    draw_road_or_map(f, road_area, app, None);

    let [elev_area, bottom_right] = Layout::vertical([Constraint::Percentage(40), Constraint::Percentage(60)]).areas(right);
    route_elevation_canvas(f, elev_area, app, None);

    let [riders_area, charts_area] = Layout::horizontal([Constraint::Percentage(60), Constraint::Percentage(40)]).areas(bottom_right);
    ride_riders(f, riders_area, app, ego);

    let [pw, hr] = Layout::vertical([Constraint::Percentage(55), Constraint::Percentage(45)]).areas(charts_area);
    let power: Vec<f64> = app.ego_hist.iter().map(|e| e.power.unwrap_or(0.0)).collect();
    let pmax = power.iter().cloned().fold(0.0, f64::max);
    series_chart(f, pw, format!(" Power · max {pmax:.0}W "), &power, POWER, true);

    let hrs: Vec<f64> = app.ego_hist.iter().filter_map(|e| e.hr).collect();
    series_chart(f, hr, format!(" HR · avg {} ", fmt(ego.hr_avg, 0)), &hrs, HR, false);
}

fn ride_tiles(f: &mut Frame, row1: Rect, row2: Rect, ego: &Ego) {
    let rowing = ego.sport == 1;
    let c1 = Layout::horizontal([Constraint::Ratio(1, 4); 4]).split(row1);
    tile(
        f,
        c1[0],
        "Power",
        fmt(ego.power, 0),
        "W",
        format!("{} W/kg · avg {}", fmt(ego.wkg, 1), fmt(ego.power_avg, 0)),
        POWER,
    );
    tile(f, c1[1], "Heart rate", fmt(ego.hr, 0), "bpm", format!("avg {}", fmt(ego.hr_avg, 0)), HR);
    tile(
        f,
        c1[2],
        if rowing { "Stroke rate" } else { "Cadence" },
        fmt(ego.cadence, 0),
        if rowing { "spm" } else { "rpm" },
        format!("avg {}", fmt(ego.cadence_avg, 0)),
        CAD,
    );
    tile(f, c1[3], "Speed", fmt(kmh(ego.speed), 1), "km/h", format!("avg {}", fmt(kmh(ego.speed_avg), 1)), SPEED);

    let draft_pct = ego.draft.map(|d| ((1.0 - d) * 100.0).clamp(0.0, 100.0));
    let draft_col = if draft_pct.is_some_and(|p| p >= 15.0) {
        Color::Cyan
    } else if draft_pct.is_some_and(|p| p >= 5.0) {
        Color::LightGreen
    } else {
        Color::Gray
    };

    let c2 = Layout::horizontal([Constraint::Ratio(1, 4); 4]).split(row2);
    tile(
        f,
        c2[0],
        "Distance",
        fmt(km(ego.distance), 2),
        "km",
        format!("{} km to go", fmt(km(ego.distance_remaining), 1)),
        Color::White,
    );
    tile(
        f,
        c2[1],
        "Grade",
        fmt(ego.grade.map(|g| g * 100.0), 1),
        "%",
        format!("elev {} m", fmt(ego.elevation, 0)),
        grade_color(ego.grade),
    );
    tile(
        f,
        c2[2],
        "Climbed",
        fmt(ego.climb, 0),
        "m",
        format!("{} m to go", fmt(ego.climb_remaining, 0)),
        ELEV,
    );
    tile(
        f,
        c2[3],
        "Draft",
        fmt(draft_pct, 0),
        "%",
        format!("factor {} · lap {}", fmt(ego.draft, 2), fmt(ego.lap, 0)),
        draft_col,
    );
}

fn ride_riders(f: &mut Frame, area: Rect, app: &App, ego: &Ego) {
    let has_ego = app.riders.iter().any(|r| r.is_ego);
    let (rows, focus_idx): (Vec<Row>, Option<usize>) = if has_ego {
        let idx = app.riders.iter().position(|r| r.is_ego);
        (app.riders.iter().map(rider_row).collect(), idx)
    } else {
        let you_lbl = if ego.sport == 1 { "▲ YOU (Rowing)" } else { "▲ YOU" };
        let you = Row::new(vec![
            you_lbl.to_string(),
            "0m".to_string(),
            fmt(ego.power, 0),
            fmt(ego.wkg, 1),
            fmt(kmh(ego.speed), 1),
            fmt(ego.hr, 0),
        ])
        .style(Style::new().fg(Color::Black).bg(Color::Green).add_modifier(Modifier::BOLD));

        let split = app.riders.iter().position(|r| r.gap.is_none_or(|g| g <= 0.0)).unwrap_or(app.riders.len());
        let mut rws = Vec::with_capacity(app.riders.len() + 1);
        rws.extend(app.riders[..split].iter().map(rider_row));
        rws.push(you);
        rws.extend(app.riders[split..].iter().map(rider_row));
        (rws, Some(split))
    };

    let mut state = TableState::default().with_selected(focus_idx);
    let table = Table::new(rows, RIDER_WIDTHS)
        .header(rider_header())
        .block(
            Block::bordered()
                .title(Span::styled(format!(" Riders · {} ", app.riders.len()), Style::new().fg(Color::White)))
                .border_style(Style::new().fg(DIM)),
        );
    f.render_stateful_widget(table, area, &mut state);
}

// ---------------------------------------------------------------- spectate screen

fn draw_spectate(f: &mut Frame, area: Rect, app: &App) {
    if app.riders.is_empty() {
        message(
            f,
            area,
            "Spectate",
            vec![
                Line::styled("No riders in the broadcast / btApp yet…", Style::new().bold()),
                Line::raw(""),
                Line::raw("Open a ride to spectate in the Terra window; then press w to hide it."),
                Line::styled("w  show browser   ·   g  3D mode", Style::new().fg(DIM)),
            ],
        );
        return;
    }

    let sel_id = app.selected_id.as_deref();

    // Big full-height Road View on the left; Tiles + Filled Elevation + Riders/Power on the right
    let [road_area, right] = Layout::horizontal([Constraint::Percentage(42), Constraint::Percentage(58)]).areas(area);
    draw_road_or_map(f, road_area, app, sel_id);

    let Some(r) = app.selected_rider() else { return };
    let [name, row1, row2, elev_area, bottom_right] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(4),
        Constraint::Length(4),
        Constraint::Percentage(36),
        Constraint::Min(8),
    ])
    .areas(right);

    let sport = if r.sport == 1 { "rowing" } else { "cycling" };
    let mut banner = vec![
        Span::styled(format!(" ◆ {} ", r.name), Style::new().bold().fg(Color::Black).bg(Color::Magenta)),
    ];
    if r.is_focal {
        banner.push(Span::styled(" FOCAL ", Style::new().bold().fg(Color::Black).bg(Color::Green)));
    }
    if r.attacking {
        banner.push(Span::styled(" ⚡ATTACK ", Style::new().bold().fg(Color::White).bg(Color::Red)));
    }
    banner.push(Span::styled(
        format!("  {sport} · {} km · lap {}", fmt(km(r.dist), 2), fmt(r.lap, 0)),
        Style::new().fg(DIM),
    ));
    f.render_widget(Paragraph::new(Line::from(banner)), name);

    let c1 = Layout::horizontal([Constraint::Ratio(1, 3); 3]).split(row1);
    tile(f, c1[0], "Power", fmt(r.power, 0), "W", format!("{} W/kg", fmt(r.wkg, 1)), POWER);
    tile(f, c1[1], "Heart rate", fmt(r.hr, 0), "bpm", String::new(), HR);
    tile(
        f,
        c1[2],
        if r.sport == 1 { "Stroke rate" } else { "Cadence" },
        fmt(r.cadence, 0),
        if r.sport == 1 { "spm" } else { "rpm" },
        String::new(),
        CAD,
    );
    let c2 = Layout::horizontal([Constraint::Ratio(1, 3); 3]).split(row2);
    tile(f, c2[0], "Speed", fmt(kmh(r.speed), 1), "km/h", String::new(), SPEED);
    tile(f, c2[1], "Δ / Gap", fmt_rider_pos(r), "", format!("{} km", fmt(km(r.dist), 2)), Color::White);
    tile(
        f,
        c2[2],
        "Grade",
        fmt(r.grade, 1),
        "%",
        format!("elev {} m", fmt(r.y, 0)),
        r.grade.map(grade_pct_color).unwrap_or(Color::White),
    );

    route_elevation_canvas(f, elev_area, app, sel_id);

    let [list_area, chart_area] = Layout::horizontal([Constraint::Percentage(62), Constraint::Percentage(38)]).areas(bottom_right);
    let rows: Vec<Row> = app.riders.iter().map(rider_row).collect();
    let mut state = TableState::default().with_selected(app.selected_index());
    let table = Table::new(rows, RIDER_WIDTHS)
        .header(rider_header())
        .row_highlight_style(Style::new().fg(Color::Black).bg(Color::Magenta).add_modifier(Modifier::BOLD))
        .block(
            Block::bordered()
                .title(Span::styled(
                    format!(" Riders · {} (↵ focus) ", app.riders.len()),
                    Style::new().fg(Color::White),
                ))
                .border_style(Style::new().fg(DIM)),
        );
    f.render_stateful_widget(table, list_area, &mut state);

    let hist: Vec<f64> = app.rider_hist.get(&r.id).map(|h| h.iter().cloned().collect()).unwrap_or_default();
    series_chart(f, chart_area, format!(" {} · power ", short_name(&r.name, 10)), &hist, POWER, true);
}
