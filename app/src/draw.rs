//! Rendering: the node canvas with its activity visuals, the inspector,
//! the status line and overlays.

use crate::theme::*;
use crate::ui::{label, App, Focus, Mode, Row};
use eframe::egui::{self, pos2, vec2, Align2, Color32, Painter, Pos2, Rect, Stroke, StrokeKind, Vec2};
use engine::model::{seq_p, Curve, NodeKind, NodeModel, FILTER_MODES, MAX_NODES, MAX_STEPS, PAIRS, SHAPES, WAVES};
use std::sync::atomic::Ordering::Relaxed;

const CELL: Vec2 = vec2(190.0, 104.0);
const NODE: Vec2 = vec2(144.0, 58.0);
const INSPECTOR_W: f32 = 340.0;
const STATUS_H: f32 = 26.0;
const RIPPLE_SECS: f64 = 0.9;
const PULSE_SECS: f64 = 0.28;

/// Per-node animation state derived from engine meters. This is the hook
/// for the future generative visual layer.
pub struct Visuals {
    last_triggers: Vec<u32>,
    flash: Vec<f32>,
    ripples: Vec<(u32, f64)>,
    pulses: Vec<(u32, u32, f64)>,
    last_time: f64,
}

impl Default for Visuals {
    fn default() -> Self {
        Visuals { last_triggers: vec![0; MAX_NODES], flash: vec![0.0; MAX_NODES], ripples: Vec::new(), pulses: Vec::new(), last_time: 0.0 }
    }
}

impl Visuals {
    fn update(&mut self, app_session: &engine::Session, now: f64) {
        let dt = (now - self.last_time).clamp(0.0, 0.1) as f32;
        self.last_time = now;
        for f in self.flash.iter_mut() {
            *f = (*f - dt * 4.0).max(0.0);
        }
        self.ripples.retain(|(_, t)| now - t < RIPPLE_SECS);
        self.pulses.retain(|(_, _, t)| now - t < PULSE_SECS);
        let Some(m) = app_session.meters() else { return };
        for n in &app_session.project.nodes {
            let id = n.id as usize;
            let t = m.triggers[id].load(Relaxed);
            if t != self.last_triggers[id] {
                self.last_triggers[id] = t;
                self.flash[id] = 1.0;
                if self.ripples.len() < 256 {
                    self.ripples.push((n.id, now));
                }
                for e in app_session.project.edges.iter().filter(|e| e.from == n.id) {
                    if self.pulses.len() < 256 {
                        self.pulses.push((e.from, e.to, now));
                    }
                }
            }
        }
    }
}

fn world(n: &NodeModel) -> Vec2 {
    vec2(n.x as f32 * CELL.x, n.y as f32 * CELL.y)
}

pub fn note_name(n: u8) -> String {
    const NAMES: [&str; 12] = ["c", "c#", "d", "d#", "e", "f", "f#", "g", "g#", "a", "a#", "b"];
    format!("{}{}", NAMES[n as usize % 12], n as i32 / 12 - 2)
}

fn fade(c: Color32, a: f32) -> Color32 {
    let a = a.clamp(0.0, 1.0);
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (a * 255.0) as u8)
}

/// A point along the cable from output port `a` to input port `b`. Loop
/// cables swing out of the way: over the node for a self-loop, under the
/// nodes for one running backwards.
fn bezier(a: Pos2, b: Pos2, back: bool, t: f32) -> Pos2 {
    let (c1, c2) = if !back {
        let k = ((b.x - a.x).abs() * 0.5).max(40.0);
        (a + vec2(k, 0.0), b - vec2(k, 0.0))
    } else if a.y == b.y && (a.x - b.x - NODE.x).abs() < 1.0 {
        (a + vec2(50.0, -90.0), b + vec2(-50.0, -90.0))
    } else {
        let drop = NODE.y * 0.5 + 40.0 + (a.x - b.x).abs() * 0.08;
        (pos2(a.x + 70.0, a.y.max(b.y) + drop), pos2(b.x - 70.0, a.y.max(b.y) + drop))
    };
    let u = 1.0 - t;
    let p = a.to_vec2() * u * u * u + c1.to_vec2() * 3.0 * u * u * t + c2.to_vec2() * 3.0 * u * t * t + b.to_vec2() * t * t * t;
    p.to_pos2()
}

/// Loop cables are dashed.
fn cable(p: &Painter, a: Pos2, b: Pos2, back: bool, stroke: Stroke) {
    let pts: Vec<Pos2> = (0..=32).map(|i| bezier(a, b, back, i as f32 / 32.0)).collect();
    if back {
        p.extend(egui::Shape::dashed_line(&pts, stroke, 6.0, 4.0));
    } else {
        p.add(egui::Shape::line(pts, stroke));
    }
}

fn text(p: &Painter, pos: Pos2, anchor: Align2, s: impl ToString, size: f32, c: Color32) -> Rect {
    p.text(pos, anchor, s, mono(size), c)
}

fn summary(n: &NodeModel, channels: usize) -> (String, Color32) {
    let v = &n.params;
    match n.kind {
        NodeKind::Seq if v[seq_p::MODE] >= 0.5 => {
            let pat = &n.seq.as_ref().unwrap().pattern;
            let short: String = pat.chars().take(17).collect();
            (if pat.chars().count() > 17 { format!("{short}…") } else { short }, TEXT)
        }
        NodeKind::Seq => (String::new(), TEXT),
        NodeKind::Synth => {
            let cut = v[engine::model::synth_p::CUTOFF];
            let cut = if cut >= 1000.0 { format!("{:.1}k", cut / 1000.0) } else { format!("{cut:.0}") };
            (format!("{} {cut}", WAVES[v[0] as usize]), TEXT)
        }
        NodeKind::Amp => (format!("×{:.2} ◦{:+.1}", v[0], v[1]), TEXT),
        NodeKind::Delay => (format!("{}/16 fb {:.2}", v[0] as i32, v[1]), TEXT),
        NodeKind::Filter => {
            let cut = v[1];
            let cut = if cut >= 1000.0 { format!("{:.1}k", cut / 1000.0) } else { format!("{cut:.0}") };
            (format!("{} {cut} q{:.2}", FILTER_MODES[v[0] as usize], v[2]), TEXT)
        }
        NodeKind::Drive => {
            let crush = if v[2] < 16.0 { format!(" {}b", v[2] as i32) } else { String::new() };
            (format!("{} {:.2}{crush}", SHAPES[v[0] as usize], v[1]), TEXT)
        }
        NodeKind::Reverb => (format!("size {:.2} mix {:.2}", v[0], v[3]), TEXT),
        NodeKind::Chorus => (format!("{:.1}Hz {:.0}ms fb{:+.1}", v[0], v[2], v[3]), TEXT),
        NodeKind::Out => {
            let pair = v[0] as usize;
            let ok = 2 * pair + 1 < channels || (channels == 1 && pair == 0);
            (format!("→ {}", PAIRS[pair]), if ok { TEXT } else { ERROR })
        }
    }
}

pub fn draw(app: &mut App, ui: &mut egui::Ui) {
    let now = ui.input(|i| i.time);
    app.visuals.update(&app.session, now);
    let full = ui.max_rect();
    let p = ui.painter().clone();
    p.rect_filled(full, 0.0, BG);

    let show_inspector = app.selected.is_some();
    let canvas = Rect::from_min_max(full.min, pos2(full.max.x - if show_inspector { INSPECTOR_W } else { 0.0 }, full.max.y - STATUS_H));

    // Camera: start centred on the patch, then only follow the selection
    // when it gets near the edge.
    if !app.cam_ready && !app.session.project.nodes.is_empty() {
        let pts: Vec<Vec2> = app.session.project.nodes.iter().map(world).collect();
        let min = pts.iter().fold(pts[0], |a, b| a.min(*b));
        let max = pts.iter().fold(pts[0], |a, b| a.max(*b));
        app.cam = (min + max + NODE) * 0.5;
        app.cam_ready = true;
    }
    if let Some(n) = app.selected.and_then(|id| app.session.project.node(id)) {
        let rel = world(n) + NODE * 0.5 - app.cam;
        let half = (canvas.size() * 0.5 - NODE - vec2(30.0, 30.0)).max(Vec2::ZERO);
        let push = vec2(rel.x - rel.x.clamp(-half.x, half.x), rel.y - rel.y.clamp(-half.y, half.y));
        app.cam += push * 0.2;
    }
    let to_screen = |w: Vec2| canvas.center() + (w - app.cam);

    draw_canvas(app, &p.with_clip_rect(canvas), canvas, now, &to_screen);
    if show_inspector {
        draw_inspector(app, &p, Rect::from_min_max(pos2(canvas.max.x, full.min.y), pos2(full.max.x, full.max.y - STATUS_H)), now);
    }
    draw_status(app, &p, Rect::from_min_max(pos2(full.min.x, full.max.y - STATUS_H), full.max));
    match &app.mode {
        Mode::Pick(_) => draw_picker(app, &p, full),
        Mode::Help => draw_help(&p, full),
        _ => {}
    }

    // Mouse is optional: click a node to select it.
    let response = ui.interact(canvas, egui::Id::unique("canvas"), egui::Sense::click());
    if let Some(pos) = response.interact_pointer_pos().filter(|_| response.clicked()) {
        let hit = app.session.project.nodes.iter().find(|n| Rect::from_min_size(to_screen(world(n)), NODE).contains(pos)).map(|n| n.id);
        if hit.is_some() && matches!(app.mode, Mode::Graph) {
            app.selected = hit;
        }
    }
}

fn draw_canvas(app: &App, p: &Painter, canvas: Rect, now: f64, to_screen: &dyn Fn(Vec2) -> Pos2) {
    // Dot grid at cell corners.
    let origin = to_screen(Vec2::ZERO) - NODE * 0.0 - (CELL - NODE) * 0.5;
    let (sx, sy) = ((canvas.min.x - origin.x) / CELL.x, (canvas.min.y - origin.y) / CELL.y);
    for gx in sx.floor() as i32..=(sx + canvas.width() / CELL.x).ceil() as i32 {
        for gy in sy.floor() as i32..=(sy + canvas.height() / CELL.y).ceil() as i32 {
            p.circle_filled(origin + vec2(gx as f32 * CELL.x, gy as f32 * CELL.y), 1.2, DOT);
        }
    }

    let pr = &app.session.project;
    let channels = app.audio.as_ref().map(|a| a.channels).unwrap_or(2);
    let meters = app.session.meters();
    let ports = |n: &NodeModel| {
        let r = Rect::from_min_size(to_screen(world(n)), NODE);
        (r, pos2(r.max.x, r.center().y), pos2(r.min.x, r.center().y))
    };

    // Cables, then pulses travelling along them.
    let feedback = pr.feedback_edges();
    let is_back = |a: u32, b: u32| feedback.iter().any(|e| (e.from, e.to) == (a, b));
    for e in &pr.edges {
        let (Some(na), Some(nb)) = (pr.node(e.from), pr.node(e.to)) else { continue };
        let (_, out, _) = ports(na);
        let (_, _, inp) = ports(nb);
        let back = is_back(e.from, e.to);
        let glow = app.visuals.flash[e.from as usize];
        let c = kind_color(na.kind);
        // Quieter cables fade towards the background.
        let level = 0.35 + 0.65 * (e.gain / 1.0).min(1.0);
        cable(p, out, inp, back, Stroke::new(1.0 + glow + (e.gain - 1.0).max(0.0), LINE.lerp_to_gamma(c, (0.25 + 0.6 * glow) * level)));
        if (e.gain - 1.0).abs() > 0.005 {
            text(p, bezier(out, inp, back, 0.5) - vec2(0.0, 4.0), Align2::CENTER_BOTTOM, format!("×{:.2}", e.gain), 10.0, DIM);
        }
    }
    for (a, b, t0) in &app.visuals.pulses {
        let (Some(na), Some(nb)) = (pr.node(*a), pr.node(*b)) else { continue };
        let t = ((now - t0) / PULSE_SECS) as f32;
        let pos = bezier(ports(na).1, ports(nb).2, is_back(*a, *b), t);
        p.circle_filled(pos, 3.0, fade(kind_color(na.kind), 1.0 - t * 0.5));
    }

    // Connection preview; a cable that would close a loop is drawn as one.
    if let Mode::Connect { from, target } = app.mode {
        if let (Some(a), Some(b)) = (pr.node(from), pr.node(target)) {
            let ok = a.kind.can_feed(b.kind);
            let back = from == target || pr.would_cycle(from, target);
            cable(p, ports(a).1, ports(b).2, back, Stroke::new(2.0, if ok { ACCENT } else { ERROR }));
            let r = ports(b).0.expand(6.0);
            p.rect_stroke(r, 3.0, Stroke::new(1.0, if ok { ACCENT } else { ERROR }), StrokeKind::Outside);
        }
    }

    // Ripples: each note-on breathes outward from its node.
    for (id, t0) in &app.visuals.ripples {
        let Some(n) = pr.node(*id) else { continue };
        let t = ((now - t0) / RIPPLE_SECS) as f32;
        let r = ports(n).0;
        let grow = t * 34.0;
        p.rect_stroke(r.expand(grow), 4.0 + grow * 0.5, Stroke::new(1.0, fade(kind_color(n.kind), 0.45 * (1.0 - t))), StrokeKind::Outside);
    }

    for n in &pr.nodes {
        let (r, out, inp) = ports(n);
        let c = kind_color(n.kind);
        let sel = app.selected == Some(n.id);
        let level = meters.map(|m| m.level(n.id)).unwrap_or(0.0);
        let flash = app.visuals.flash[n.id as usize];
        let body = PANEL.lerp_to_gamma(c, (0.06 + level * 0.8 + flash * 0.18).min(0.5));
        p.rect_filled(r, 3.0, body);
        let stroke_c = if sel {
            ACCENT
        } else if n.muted {
            LINE
        } else {
            LINE.lerp_to_gamma(c, 0.5 + flash * 0.5)
        };
        p.rect_stroke(r, 3.0, Stroke::new(if sel { 1.6 } else { 1.0 }, stroke_c), StrokeKind::Inside);
        let title_c = if n.muted { DIM } else { c };
        text(p, r.min + vec2(8.0, 6.0), Align2::LEFT_TOP, format!("{} {}", n.kind.name(), n.id), 13.0, title_c);
        if n.muted {
            text(p, pos2(r.max.x - 8.0, r.min.y + 6.0), Align2::RIGHT_TOP, "mute", 11.0, DIM);
        }
        let (s, sc) = summary(n, channels);
        text(p, r.min + vec2(8.0, 25.0), Align2::LEFT_TOP, s, 12.0, sc);
        if n.kind == NodeKind::Seq && n.params[seq_p::MODE] < 0.5 {
            mini_steps(p, n, r, meters.map(|m| m.position[n.id as usize].load(Relaxed)), app.playing);
        }
        // Level meter along the bottom edge.
        let w = (level * 3.0).min(1.0) * (r.width() - 2.0);
        p.rect_filled(Rect::from_min_size(pos2(r.min.x + 1.0, r.max.y - 3.0), vec2(w, 2.0)), 0.0, c);
        if n.kind.input() != engine::model::Port::None {
            p.circle_filled(inp, 2.5, LINE.lerp_to_gamma(c, 0.4));
        }
        if n.kind.output() != engine::model::Port::None {
            p.circle_filled(out, 2.5, LINE.lerp_to_gamma(c, 0.4));
        }
    }

    if pr.nodes.is_empty() {
        text(p, canvas.center(), Align2::CENTER_CENTER, "empty — press a to add a node", 14.0, DIM);
    }
}

fn mini_steps(p: &Painter, n: &NodeModel, r: Rect, playhead: Option<u32>, playing: bool) {
    let len = (n.params[seq_p::LENGTH] as usize).min(16);
    let steps = &n.seq.as_ref().unwrap().steps;
    let w = (r.width() - 16.0) / 16.0;
    for (i, s) in steps.iter().enumerate().take(len) {
        let cell = Rect::from_min_size(pos2(r.min.x + 8.0 + i as f32 * w, r.min.y + 30.0), vec2(w - 2.0, 14.0));
        let mut fill = if s.on { fade(kind_color(NodeKind::Seq), 0.35 + s.vel * 0.5) } else { DOT };
        if playing && playhead.map(|ph| ph as usize % 16 == i).unwrap_or(false) {
            fill = BRIGHT;
        }
        p.rect_filled(cell, 1.0, fill);
    }
}

fn draw_inspector(app: &App, p: &Painter, rect: Rect, now: f64) {
    p.rect_filled(rect, 0.0, PANEL);
    p.line_segment([rect.left_top(), rect.left_bottom()], Stroke::new(1.0, LINE));
    let Some(n) = app.selected.and_then(|id| app.session.project.node(id)) else { return };
    let x0 = rect.min.x + 16.0;
    let mut y = rect.min.y + 14.0;
    text(p, pos2(x0, y), Align2::LEFT_TOP, label(&app.session, n.id), 16.0, kind_color(n.kind));
    let ins: Vec<String> = app.session.project.edges.iter().filter(|e| e.to == n.id).map(|e| label(&app.session, e.from)).collect();
    let outs: Vec<String> = app.session.project.edges.iter().filter(|e| e.from == n.id).map(|e| label(&app.session, e.to)).collect();
    y += 24.0;
    if !ins.is_empty() {
        text(p, pos2(x0, y), Align2::LEFT_TOP, format!("← {}", ins.join(", ")), 11.0, DIM);
        y += 15.0;
    }
    if !outs.is_empty() {
        text(p, pos2(x0, y), Align2::LEFT_TOP, format!("→ {}", outs.join(", ")), 11.0, DIM);
        y += 15.0;
    }
    y += 10.0;

    let editing_params = matches!(app.mode, Mode::Edit(Focus::Params) | Mode::Value(_));
    for (i, row) in Row::all(&app.session, n.id).into_iter().enumerate() {
        let (spec, v) = (row.spec(&app.session, n.id), row.value(&app.session, n.id));
        let name = match row {
            Row::Param(_) => spec.name.to_string(),
            Row::Cable(from) => format!("← {}", label(&app.session, from)),
        };
        if i == n.kind.params().len() {
            text(p, pos2(x0, y + 2.0), Align2::LEFT_TOP, "cable levels", 11.0, DIM);
            y += 18.0;
        }
        let focused = editing_params && app.param_focus == i;
        let row = Rect::from_min_size(pos2(rect.min.x + 6.0, y - 2.0), vec2(rect.width() - 12.0, 20.0));
        if focused {
            p.rect_filled(row, 2.0, fade(ACCENT, 0.10));
        }
        let c = if focused { ACCENT } else { TEXT };
        text(p, pos2(x0, y), Align2::LEFT_TOP, name, 13.0, c);
        let bar = Rect::from_min_size(pos2(x0 + 96.0, y + 5.0), vec2(110.0, 6.0));
        p.rect_filled(bar, 1.0, DOT);
        let norm = spec.to_norm(v);
        if spec.curve != Curve::Int || spec.labels.is_empty() {
            p.rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * norm, bar.height())), 1.0, if focused { ACCENT } else { DIM });
        }
        let value = match (&app.mode, focused) {
            (Mode::Value(buf), true) => format!("={buf}▏"),
            _ => spec.format(v),
        };
        text(p, pos2(rect.max.x - 16.0, y), Align2::RIGHT_TOP, value, 13.0, c);
        y += 22.0;
    }

    if n.kind == NodeKind::Seq {
        y += 12.0;
        let body_focus = matches!(app.mode, Mode::Edit(Focus::Body) | Mode::Insert { .. });
        if n.params[seq_p::MODE] >= 0.5 {
            draw_pattern_editor(app, p, n, x0, y, rect.width() - 32.0, body_focus, now);
        } else {
            draw_grid_editor(app, p, n, x0, y, rect.width() - 32.0, body_focus);
        }
    }

    let hints = match &app.mode {
        Mode::Edit(Focus::Params) => "j/k param · h/l adjust · H/L coarse · = type · 0 reset · esc",
        Mode::Edit(Focus::Body) if n.params[seq_p::MODE] >= 0.5 => "i edit pattern · tab params · esc",
        Mode::Edit(Focus::Body) => "h/l step · j/k note · J/K oct · enter/x toggle · v vel · < > rotate · p copy→ · C clear · tab",
        Mode::Insert { .. } => "type · enter apply (next bar) · esc cancel",
        Mode::Value(_) => "type a value · enter · esc",
        _ => "enter: edit",
    };
    let galley = p.layout(hints.to_string(), mono(11.0), DIM, rect.width() - 32.0);
    p.galley(pos2(x0, rect.max.y - 14.0 - galley.size().y), galley, DIM);
}

fn draw_grid_editor(app: &App, p: &Painter, n: &NodeModel, x0: f32, y: f32, width: f32, focused: bool) {
    let len = n.params[seq_p::LENGTH] as usize;
    let steps = &n.seq.as_ref().unwrap().steps;
    let playhead = app.session.meters().map(|m| m.position[n.id as usize].load(Relaxed) as usize);
    let w = width / 8.0;
    let h = 30.0;
    let cursor = app.step_cursor.min(len - 1);
    for (i, &s) in steps.iter().enumerate().take(MAX_STEPS.min(len)) {
        let (col, row) = (i % 8, i / 8);
        let cell = Rect::from_min_size(pos2(x0 + col as f32 * w, y + row as f32 * (h + 4.0)), vec2(w - 3.0, h));
        let base = if s.on { fade(kind_color(NodeKind::Seq), 0.25 + 0.55 * s.vel) } else { DOT };
        p.rect_filled(cell, 2.0, base);
        if app.playing && playhead == Some(i) {
            p.rect_stroke(cell, 2.0, Stroke::new(1.0, BRIGHT), StrokeKind::Inside);
        }
        if focused && i == cursor {
            p.rect_stroke(cell.expand(1.5), 2.0, Stroke::new(1.5, ACCENT), StrokeKind::Outside);
        }
        if s.on {
            text(p, cell.center(), Align2::CENTER_CENTER, note_name(s.note), 11.0, BRIGHT);
        } else if i % 4 == 0 {
            text(p, cell.center(), Align2::CENTER_CENTER, "·", 11.0, DIM);
        }
    }
    if focused {
        let s = steps[cursor];
        let rows = len.div_ceil(8) as f32;
        let info = format!("step {}  {}  vel {:.1}  {}", cursor + 1, note_name(s.note), s.vel, if s.on { "on" } else { "off" });
        text(p, pos2(x0, y + rows * (h + 4.0) + 6.0), Align2::LEFT_TOP, info, 12.0, TEXT);
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_pattern_editor(app: &App, p: &Painter, n: &NodeModel, x0: f32, y: f32, width: f32, focused: bool, now: f64) {
    let stored = &n.seq.as_ref().unwrap().pattern;
    let (shown, cursor) = match &app.mode {
        Mode::Insert { buf, cursor } => (buf.as_str(), Some(*cursor)),
        _ => (stored.as_str(), None),
    };
    let box_rect = Rect::from_min_size(pos2(x0 - 6.0, y - 4.0), vec2(width + 12.0, 64.0));
    p.rect_filled(box_rect, 2.0, BG);
    let border = if cursor.is_some() {
        ACCENT
    } else if focused {
        LINE.lerp_to_gamma(ACCENT, 0.5)
    } else {
        LINE
    };
    p.rect_stroke(box_rect, 2.0, Stroke::new(1.0, border), StrokeKind::Inside);
    let galley = p.layout(shown.to_string(), mono(14.0), BRIGHT, width);
    if let Some(c) = cursor {
        // Blinking caret.
        if (now * 2.0) as i64 % 2 == 0 {
            let r = galley.pos_from_cursor(egui::text::CCursor::new(c));
            let top = pos2(x0, y) + r.min.to_vec2();
            p.line_segment([top, top + vec2(0.0, 16.0)], Stroke::new(1.5, ACCENT));
        }
    }
    let gh = galley.size().y;
    p.galley(pos2(x0, y), galley, BRIGHT);
    let mut yy = y + gh.max(56.0) + 10.0;
    if let Some(e) = app.pattern_errors.get(&n.id) {
        text(p, pos2(x0, yy), Align2::LEFT_TOP, format!("✗ {e}"), 12.0, ERROR);
        yy += 18.0;
    }
    let tips = ["c3 eb4 60 · ~ rest", "[a b] group · <a b> per bar", "a*2 a/2 a@3 a!3 · a, b chord"];
    for t in tips {
        text(p, pos2(x0, yy), Align2::LEFT_TOP, t, 11.0, DIM);
        yy += 15.0;
    }
}

fn draw_status(app: &App, p: &Painter, rect: Rect) {
    p.rect_filled(rect, 0.0, PANEL);
    p.line_segment([rect.left_top(), rect.right_top()], Stroke::new(1.0, LINE));
    let y = rect.center().y;
    if let Mode::Command(buf) = &app.mode {
        text(p, pos2(rect.min.x + 10.0, y), Align2::LEFT_CENTER, format!(":{buf}▏"), 14.0, ACCENT);
        return;
    }
    let cycle = app.session.meters().map(|m| m.cycle()).unwrap_or(0.0);
    let (bar, beat) = (cycle.floor() as i64 + 1, (cycle.fract() * 4.0).floor() as i64 + 1);
    let play = if app.playing { "▶" } else { "■" };
    let left = format!("{play} {:.0} bpm  {bar}.{beat}", app.session.project.bpm);
    let r = text(p, pos2(rect.min.x + 10.0, y), Align2::LEFT_CENTER, left, 13.0, if app.playing { ACCENT } else { TEXT });
    let mode = match app.mode {
        Mode::Graph => "graph",
        Mode::Connect { .. } => "connect",
        Mode::Edit(_) => "edit",
        Mode::Insert { .. } => "insert",
        Mode::Value(_) => "value",
        Mode::Command(_) => "command",
        Mode::Pick(_) => "pick",
        Mode::Help => "help",
    };
    let r = text(p, pos2(r.max.x + 18.0, y), Align2::LEFT_CENTER, format!("[{mode}] {}", app.project_name), 13.0, DIM);
    if let Some(m) = &app.message {
        let age = m.at.elapsed().as_secs_f32();
        if age < 6.0 {
            let c = if m.error { ERROR } else { TEXT };
            text(p, pos2(r.max.x + 18.0, y), Align2::LEFT_CENTER, &m.text, 13.0, fade(c, 1.0 - (age - 4.0).max(0.0) / 2.0));
        }
    }
    let dev = match &app.audio {
        Some(a) => {
            let clip = app.session.meters().map(|m| m.clipping.load(Relaxed)).unwrap_or(0);
            format!("{}{} · {}ch · {}k", if clip != 0 { "CLIP · " } else { "" }, a.device, a.channels, a.sample_rate / 1000)
        }
        None => "no audio · :device".into(),
    };
    text(p, pos2(rect.max.x - 10.0, y), Align2::RIGHT_CENTER, dev, 12.0, DIM);
}

fn overlay_box(p: &Painter, full: Rect, size: Vec2) -> Rect {
    p.rect_filled(full, 0.0, fade(BG, 0.6));
    let r = Rect::from_center_size(full.center(), size);
    p.rect_filled(r, 3.0, PANEL);
    p.rect_stroke(r, 3.0, Stroke::new(1.0, LINE), StrokeKind::Inside);
    r
}

fn draw_picker(app: &App, p: &Painter, full: Rect) {
    let Mode::Pick(picker) = &app.mode else { return };
    let entries = picker.entries(&app.session);
    let r = overlay_box(p, full, vec2(420.0, 70.0 + 22.0 * entries.len().clamp(1, 14) as f32));
    text(p, r.min + vec2(14.0, 12.0), Align2::LEFT_TOP, picker.title(), 13.0, DIM);
    text(p, r.min + vec2(14.0, 32.0), Align2::LEFT_TOP, format!("> {}▏", picker.query), 14.0, ACCENT);
    for (row, (name, _)) in entries.iter().take(14).enumerate() {
        let y = r.min.y + 58.0 + row as f32 * 22.0;
        let sel = row == picker.idx;
        if sel {
            p.rect_filled(Rect::from_min_size(pos2(r.min.x + 6.0, y - 2.0), vec2(r.width() - 12.0, 20.0)), 2.0, fade(ACCENT, 0.12));
        }
        text(p, pos2(r.min.x + 14.0, y), Align2::LEFT_TOP, name, 13.0, if sel { ACCENT } else { TEXT });
    }
    if entries.is_empty() {
        text(p, pos2(r.min.x + 14.0, r.min.y + 58.0), Align2::LEFT_TOP, "no match", 13.0, DIM);
    }
}

const HELP: &[(&str, &str)] = &[
    ("graph", ""),
    ("hjkl / arrows", "select neighbour"),
    ("HJKL / shift+arrows", "move node"),
    ("tab / shift+tab", "cycle nodes"),
    ("a", "add node (auto-wires from selection)"),
    ("c … enter", "connect selected → target (loops ok)"),
    ("d", "disconnect"),
    ("x", "delete node"),
    ("y", "duplicate node"),
    ("m", "mute"),
    ("u", "undo"),
    ("enter / e", "edit node"),
    ("space", "play / stop"),
    ("+ / -", "bpm"),
    (":", "command line"),
    ("edit", ""),
    ("j/k  h/l  H/L", "param / cable level · adjust · coarse"),
    ("=  0", "type value · reset to default"),
    ("tab", "seq: params ⇄ steps/pattern"),
    ("steps: h/l j/k J/K", "cursor · note · octave"),
    ("steps: enter/x v < >", "toggle · velocity · rotate"),
    ("pattern: i … enter", "edit; applies on next bar"),
    ("esc", "back"),
    ("commands", ""),
    (":w [name]  :e [name]", "save / open project"),
    (":new  :demo", "empty / demo project"),
    (":device", "pick audio output (BlackHole)"),
    (":bpm 124  :q", "tempo, quit"),
];

fn draw_help(p: &Painter, full: Rect) {
    let r = overlay_box(p, full, vec2(560.0, 30.0 + 19.0 * HELP.len() as f32));
    let mut y = r.min.y + 18.0;
    for (k, v) in HELP {
        if v.is_empty() {
            text(p, pos2(r.min.x + 20.0, y), Align2::LEFT_TOP, k, 12.0, DIM);
        } else {
            text(p, pos2(r.min.x + 20.0, y), Align2::LEFT_TOP, k, 13.0, ACCENT);
            text(p, pos2(r.min.x + 250.0, y), Align2::LEFT_TOP, v, 13.0, TEXT);
        }
        y += 19.0;
    }
}
