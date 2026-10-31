//! App state and the modal keyboard model.

use crate::audio::{self, AudioOut};
use crate::draw::Visuals;
use eframe::egui::{self, Key, Modifiers, Vec2};
use engine::model::{seq_p, Curve, NodeKind, ParamSpec, Port, Step, CABLE_LEVEL, MAX_STEPS};
use engine::{Project, Session};
use pattern::ParseError;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

const UNDO_DEPTH: usize = 100;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Dir {
    Left,
    Right,
    Up,
    Down,
}

impl Dir {
    pub fn delta(self) -> (i32, i32) {
        match self {
            Dir::Left => (-1, 0),
            Dir::Right => (1, 0),
            Dir::Up => (0, -1),
            Dir::Down => (0, 1),
        }
    }
}

/// What the keyboard is currently talking to.
#[derive(Clone, Debug, PartialEq)]
pub enum Mode {
    Graph,
    /// Choosing a target for a connection from `from`.
    Connect {
        from: u32,
        target: u32,
    },
    /// Inspector focused on the selected node.
    Edit(Focus),
    /// Typing a pattern; `buf` is applied on Enter.
    Insert {
        buf: String,
        cursor: usize,
    },
    /// Typing an exact parameter value.
    Value(String),
    Command(String),
    Pick(Picker),
    Help,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Focus {
    Params,
    /// Step grid or pattern line, depending on the seq mode.
    Body,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PickKind {
    AddNode,
    Disconnect(Vec<(u32, u32)>),
    Device(Vec<String>),
    Open(Vec<String>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Picker {
    pub kind: PickKind,
    pub query: String,
    pub idx: usize,
}

impl Picker {
    pub fn new(kind: PickKind) -> Self {
        Picker { kind, query: String::new(), idx: 0 }
    }

    /// Visible entries as (label, index into the underlying list).
    pub fn entries(&self, session: &Session) -> Vec<(String, usize)> {
        let all: Vec<String> = match &self.kind {
            PickKind::AddNode => NodeKind::ALL.iter().map(|k| k.name().to_string()).collect(),
            PickKind::Disconnect(edges) => edges.iter().map(|(a, b)| format!("{} → {}", label(session, *a), label(session, *b))).collect(),
            PickKind::Device(v) | PickKind::Open(v) => v.clone(),
        };
        let q = self.query.to_lowercase();
        all.into_iter().enumerate().filter(|(_, s)| s.to_lowercase().contains(&q)).map(|(i, s)| (s, i)).collect()
    }

    pub fn title(&self) -> &'static str {
        match self.kind {
            PickKind::AddNode => "add node",
            PickKind::Disconnect(_) => "disconnect",
            PickKind::Device(_) => "audio device",
            PickKind::Open(_) => "open project",
        }
    }
}

/// A row of the edit-mode list: one of the node's params, or the level of
/// an audio cable coming into it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Row {
    Param(usize),
    Cable(u32),
}

impl Row {
    pub fn all(session: &Session, id: u32) -> Vec<Row> {
        let pr = &session.project;
        let Some(node) = pr.node(id) else { return Vec::new() };
        let params = (0..node.kind.params().len()).map(Row::Param);
        let audio_in = |from: u32| pr.node(from).is_some_and(|n| n.kind.output() == Port::Audio);
        let cables = pr.edges.iter().filter(|e| e.to == id && audio_in(e.from)).map(|e| Row::Cable(e.from));
        params.chain(cables).collect()
    }

    pub fn spec(self, session: &Session, id: u32) -> ParamSpec {
        match self {
            Row::Param(i) => session.project.node(id).unwrap().kind.params()[i],
            Row::Cable(_) => CABLE_LEVEL,
        }
    }

    pub fn value(self, session: &Session, id: u32) -> f32 {
        match self {
            Row::Param(i) => session.project.node(id).unwrap().params[i],
            Row::Cable(from) => session.project.edge(from, id).map_or(CABLE_LEVEL.default, |e| e.gain),
        }
    }

    pub fn set(self, session: &mut Session, id: u32, v: f32) {
        match self {
            Row::Param(i) => session.set_param(id, i, v),
            Row::Cable(from) => session.set_edge_gain(from, id, v),
        }
    }
}

pub fn label(session: &Session, id: u32) -> String {
    session.project.node(id).map(|n| format!("{} {}", n.kind.name(), n.id)).unwrap_or_else(|| "?".into())
}

#[derive(Clone, Copy, Debug)]
pub enum Input {
    Char(char),
    Key(Key, Modifiers),
}

/// Turns egui events into our inputs: printable characters arrive as text
/// (layout-aware), everything else as named keys.
pub fn collect_inputs(ctx: &egui::Context) -> Vec<Input> {
    ctx.input(|i| {
        i.events
            .iter()
            .filter_map(|e| match e {
                egui::Event::Text(t) => Some(t.chars().filter(|c| !c.is_control()).map(Input::Char).collect::<Vec<_>>()),
                egui::Event::Key { key, pressed: true, modifiers, .. } => match key {
                    Key::ArrowLeft
                    | Key::ArrowRight
                    | Key::ArrowUp
                    | Key::ArrowDown
                    | Key::Enter
                    | Key::Escape
                    | Key::Tab
                    | Key::Backspace
                    | Key::Delete
                    | Key::Home
                    | Key::End => Some(vec![Input::Key(*key, *modifiers)]),
                    _ => None,
                },
                _ => None,
            })
            .flatten()
            .collect()
    })
}

pub struct Message {
    pub text: String,
    pub error: bool,
    pub at: Instant,
}

pub struct App {
    pub session: Session,
    pub audio: Option<AudioOut>,
    pub selected: Option<u32>,
    pub mode: Mode,
    pub message: Option<Message>,
    pub undo: Vec<Project>,
    pub cam: Vec2,
    pub cam_ready: bool,
    pub visuals: Visuals,
    pub pattern_errors: HashMap<u32, ParseError>,
    pub param_focus: usize,
    pub step_cursor: usize,
    pub project_name: String,
    pub playing: bool,
    pub quit: bool,
}

pub fn projects_dir() -> PathBuf {
    dirs::document_dir().or_else(dirs::home_dir).unwrap_or_else(|| PathBuf::from(".")).join("Robat Synth")
}

fn autosave_path() -> PathBuf {
    projects_dir().join("autosave.json")
}

impl App {
    pub fn new() -> Self {
        let project =
            std::fs::read_to_string(autosave_path()).ok().and_then(|s| Session::project_from_json(&s).ok()).unwrap_or_else(Project::demo);
        let mut app = App {
            session: Session::new(project),
            audio: None,
            selected: None,
            mode: Mode::Graph,
            message: None,
            undo: Vec::new(),
            cam: Vec2::ZERO,
            cam_ready: false,
            visuals: Visuals::default(),
            pattern_errors: HashMap::new(),
            param_focus: 0,
            step_cursor: 0,
            project_name: "untitled".into(),
            playing: false,
            quit: false,
        };
        app.selected = app.session.project.nodes.first().map(|n| n.id);
        let device = app.session.project.device.clone();
        app.open_device(device.as_deref());
        if app.message.is_none() {
            app.info("space: play · hjkl: move · enter: edit · ?: help");
        }
        app
    }

    pub fn info(&mut self, text: impl Into<String>) {
        self.message = Some(Message { text: text.into(), error: false, at: Instant::now() });
    }

    pub fn error(&mut self, text: impl Into<String>) {
        self.message = Some(Message { text: text.into(), error: true, at: Instant::now() });
    }

    pub fn open_device(&mut self, name: Option<&str>) {
        self.audio = None; // stop the old stream before starting a new one
        self.session.detach();
        match audio::open(name, &mut self.session) {
            Ok(a) => {
                self.info(format!("audio: {} · {} ch · {} Hz", a.device, a.channels, a.sample_rate));
                self.session.project.device = name.map(str::to_string);
                self.audio = Some(a);
                if self.playing {
                    self.session.play(true);
                }
            }
            Err(e) => {
                self.error(format!("audio unavailable ({e}) — running silent"));
                let fallback = name.and_then(|_| audio::open(None, &mut self.session).ok());
                self.audio = Some(fallback.unwrap_or_else(|| audio::open_silent(&mut self.session)));
                if self.playing {
                    self.session.play(true);
                }
            }
        }
    }

    pub fn autosave(&self) {
        let _ = std::fs::create_dir_all(projects_dir());
        let _ = std::fs::write(autosave_path(), self.session.to_json());
    }

    fn snapshot(&mut self) {
        self.undo.push(self.session.project.clone());
        if self.undo.len() > UNDO_DEPTH {
            self.undo.remove(0);
        }
    }

    fn sel_kind(&self) -> Option<NodeKind> {
        self.selected.and_then(|id| self.session.project.node(id)).map(|n| n.kind)
    }

    /// The nearest node from `from` in direction `dir`.
    pub fn neighbour(&self, from: u32, dir: Dir) -> Option<u32> {
        let p = &self.session.project;
        let me = p.node(from)?;
        let (dx, dy) = dir.delta();
        p.nodes
            .iter()
            .filter(|n| n.id != from)
            .filter_map(|n| {
                let (ox, oy) = (n.x - me.x, n.y - me.y);
                let along = ox * dx + oy * dy;
                let across = (ox * dy - oy * dx).abs();
                (along > 0).then_some((along + 2 * across, n.id))
            })
            .min()
            .map(|(_, id)| id)
    }

    fn free_cell_near(&self, x: i32, y: i32) -> (i32, i32) {
        let taken = |x, y| self.session.project.nodes.iter().any(|n| n.x == x && n.y == y);
        (0..).map(|dy| (x, y + dy)).find(|(x, y)| !taken(*x, *y)).unwrap()
    }

    fn add_node(&mut self, kind: NodeKind) {
        let (x, y) = match self.selected.and_then(|id| self.session.project.node(id)) {
            Some(n) => self.free_cell_near(n.x + 1, n.y),
            None => self.free_cell_near(0, 0),
        };
        self.snapshot();
        let prev = self.selected;
        match self.session.add_node(kind, x, y) {
            Ok(id) => {
                self.selected = Some(id);
                // Auto-wire from the previous selection when the ports fit.
                if let Some(p) = prev {
                    if self.session.connect(p, id).is_ok() {
                        self.info(format!("{} → {}", label(&self.session, p), label(&self.session, id)));
                    }
                }
            }
            Err(e) => self.error(e.to_string()),
        }
    }

    pub fn handle(&mut self, input: Input) {
        let mode = self.mode.clone();
        match mode {
            Mode::Graph => self.graph_key(input),
            Mode::Connect { from, target } => self.connect_key(input, from, target),
            Mode::Edit(focus) => self.edit_key(input, focus),
            Mode::Insert { buf, cursor } => self.insert_key(input, buf, cursor),
            Mode::Value(buf) => self.value_key(input, buf),
            Mode::Command(buf) => self.command_key(input, buf),
            Mode::Pick(p) => self.pick_key(input, p),
            Mode::Help => {
                if matches!(input, Input::Key(Key::Escape, _) | Input::Char('?') | Input::Char('q') | Input::Key(Key::Enter, _)) {
                    self.mode = Mode::Graph;
                }
            }
        }
    }

    fn dir_of(input: Input) -> Option<(Dir, bool)> {
        match input {
            Input::Char('h') => Some((Dir::Left, false)),
            Input::Char('l') => Some((Dir::Right, false)),
            Input::Char('k') => Some((Dir::Up, false)),
            Input::Char('j') => Some((Dir::Down, false)),
            Input::Char('H') => Some((Dir::Left, true)),
            Input::Char('L') => Some((Dir::Right, true)),
            Input::Char('K') => Some((Dir::Up, true)),
            Input::Char('J') => Some((Dir::Down, true)),
            Input::Key(k, m) => {
                let d = match k {
                    Key::ArrowLeft => Dir::Left,
                    Key::ArrowRight => Dir::Right,
                    Key::ArrowUp => Dir::Up,
                    Key::ArrowDown => Dir::Down,
                    _ => return None,
                };
                Some((d, m.shift))
            }
            _ => None,
        }
    }

    fn toggle_play(&mut self) {
        self.playing = !self.playing;
        self.session.play(self.playing);
    }

    fn graph_key(&mut self, input: Input) {
        if let Some((dir, shift)) = Self::dir_of(input) {
            let Some(id) = self.selected else {
                self.selected = self.session.project.nodes.first().map(|n| n.id);
                return;
            };
            if shift {
                let n = self.session.project.node(id).unwrap();
                let (dx, dy) = dir.delta();
                let (x, y) = (n.x + dx, n.y + dy);
                // Swap places with whatever sits there.
                if let Some(other) = self.session.project.nodes.iter().find(|o| o.x == x && o.y == y).map(|o| o.id) {
                    let (ox, oy) = (n.x, n.y);
                    self.session.move_node(other, ox, oy);
                }
                self.session.move_node(id, x, y);
            } else if let Some(n) = self.neighbour(id, dir) {
                self.selected = Some(n);
            }
            return;
        }
        match input {
            Input::Char(' ') => self.toggle_play(),
            Input::Char('a') => self.mode = Mode::Pick(Picker::new(PickKind::AddNode)),
            Input::Char('?') => self.mode = Mode::Help,
            Input::Char(':') => self.mode = Mode::Command(String::new()),
            Input::Char('+') | Input::Char('=') => self.session.set_bpm(self.session.project.bpm.round() + 1.0),
            Input::Char('-') => self.session.set_bpm(self.session.project.bpm.round() - 1.0),
            Input::Char('u') => match self.undo.pop() {
                Some(p) => {
                    self.session.restore(p);
                    if self.selected.and_then(|id| self.session.project.node(id)).is_none() {
                        self.selected = self.session.project.nodes.first().map(|n| n.id);
                    }
                    self.info("undo");
                }
                None => self.info("nothing to undo"),
            },
            Input::Key(Key::Tab, m) => {
                let ids: Vec<u32> = self.session.project.nodes.iter().map(|n| n.id).collect();
                if !ids.is_empty() {
                    let i = self.selected.and_then(|s| ids.iter().position(|x| *x == s)).unwrap_or(0);
                    let next = if m.shift { (i + ids.len() - 1) % ids.len() } else { (i + 1) % ids.len() };
                    self.selected = Some(ids[next]);
                }
            }
            _ => {}
        }
        let Some(id) = self.selected else { return };
        match input {
            Input::Key(Key::Enter, _) | Input::Char('e') => {
                self.param_focus = 0;
                self.snapshot();
                self.mode = Mode::Edit(Focus::Params);
                if self.sel_kind() == Some(NodeKind::Seq) {
                    self.mode = Mode::Edit(Focus::Body);
                }
            }
            Input::Char('c') => {
                let target = self.neighbour(id, Dir::Right).unwrap_or(id);
                self.mode = Mode::Connect { from: id, target };
            }
            Input::Char('d') => {
                let edges: Vec<(u32, u32)> =
                    self.session.project.edges.iter().filter(|e| e.from == id || e.to == id).map(|e| (e.from, e.to)).collect();
                if edges.is_empty() {
                    self.info("no connections");
                } else {
                    self.mode = Mode::Pick(Picker::new(PickKind::Disconnect(edges)));
                }
            }
            Input::Char('x') | Input::Key(Key::Delete, _) | Input::Key(Key::Backspace, _) => {
                self.snapshot();
                let next = [Dir::Left, Dir::Up, Dir::Right, Dir::Down].iter().find_map(|d| self.neighbour(id, *d));
                self.session.remove_node(id);
                self.selected = next.or_else(|| self.session.project.nodes.first().map(|n| n.id));
                self.info("deleted (u to undo)");
            }
            Input::Char('m') => {
                self.session.toggle_mute(id);
            }
            Input::Char('y') => {
                let n = self.session.project.node(id).unwrap();
                let (x, y) = self.free_cell_near(n.x, n.y + 1);
                self.snapshot();
                match self.session.duplicate_node(id, x, y) {
                    Ok(new) => self.selected = Some(new),
                    Err(e) => self.error(e.to_string()),
                }
            }
            _ => {}
        }
    }

    fn connect_key(&mut self, input: Input, from: u32, target: u32) {
        if let Some((dir, _)) = Self::dir_of(input) {
            if let Some(t) = self.neighbour(target, dir) {
                self.mode = Mode::Connect { from, target: t };
            }
            return;
        }
        match input {
            Input::Key(Key::Escape, _) => self.mode = Mode::Graph,
            Input::Key(Key::Enter, _) | Input::Char('c') => {
                self.snapshot();
                match self.session.connect(from, target) {
                    Ok(()) => {
                        let looped = self.session.project.feedback_edges().iter().any(|e| (e.from, e.to) == (from, target));
                        let arrow = if looped { "↺" } else { "→" };
                        self.info(format!("{} {arrow} {}", label(&self.session, from), label(&self.session, target)));
                        self.mode = Mode::Graph;
                    }
                    Err(e) => {
                        self.undo.pop();
                        self.error(e.to_string());
                    }
                }
            }
            _ => {}
        }
    }

    fn edit_key(&mut self, input: Input, focus: Focus) {
        let Some(id) = self.selected else {
            self.mode = Mode::Graph;
            return;
        };
        let node = self.session.project.node(id).unwrap().clone();
        let is_seq = node.kind == NodeKind::Seq;
        let pattern_mode = is_seq && node.params[seq_p::MODE] >= 0.5;
        match input {
            Input::Key(Key::Escape, _) => {
                // Drop the undo point if nothing was changed.
                if self.undo.last() == Some(&self.session.project) {
                    self.undo.pop();
                }
                self.mode = Mode::Graph;
                return;
            }
            Input::Key(Key::Tab, _) if is_seq => {
                self.mode = Mode::Edit(if focus == Focus::Params { Focus::Body } else { Focus::Params });
                return;
            }
            Input::Char(' ') => {
                self.toggle_play();
                return;
            }
            Input::Char('?') => {
                self.mode = Mode::Help;
                return;
            }
            _ => {}
        }
        match focus {
            Focus::Params => {
                let rows = Row::all(&self.session, id);
                let len = rows.len();
                self.param_focus = self.param_focus.min(len - 1);
                let row = rows[self.param_focus];
                let spec = row.spec(&self.session, id);
                let dir = Self::dir_of(input);
                match (input, dir) {
                    (_, Some((Dir::Up, _))) => self.param_focus = (self.param_focus + len - 1) % len,
                    (_, Some((Dir::Down, _))) => self.param_focus = (self.param_focus + 1) % len,
                    (_, Some((d @ (Dir::Left | Dir::Right), coarse))) => {
                        let sign = if d == Dir::Left { -1.0 } else { 1.0 };
                        let steps = match (spec.curve, coarse) {
                            (Curve::Int, false) => 1.0,
                            (Curve::Int, true) => 4.0,
                            (_, false) => 1.0,
                            (_, true) => 10.0,
                        };
                        let v = spec.nudge(row.value(&self.session, id), sign * steps);
                        row.set(&mut self.session, id, v);
                    }
                    (Input::Char('='), _) | (Input::Key(Key::Enter, _), _) => self.mode = Mode::Value(String::new()),
                    (Input::Char('0'), _) => row.set(&mut self.session, id, spec.default),
                    _ => {}
                }
            }
            Focus::Body if pattern_mode => match input {
                Input::Char('i') | Input::Char('a') | Input::Key(Key::Enter, _) => {
                    let buf = node.seq.as_ref().unwrap().pattern.clone();
                    let cursor = buf.chars().count();
                    self.mode = Mode::Insert { buf, cursor };
                }
                _ => {}
            },
            Focus::Body => self.grid_key(input, id, &node),
        }
    }

    fn grid_key(&mut self, input: Input, id: u32, node: &engine::NodeModel) {
        let len = node.params[seq_p::LENGTH] as usize;
        let cur = self.step_cursor.min(len - 1);
        let step = node.seq.as_ref().unwrap().steps[cur];
        let set = |app: &mut App, s: Step| app.session.edit_steps(id, |d| d.steps[cur] = s);
        match (input, Self::dir_of(input)) {
            (_, Some((Dir::Left, big))) => self.step_cursor = (cur + len - if big { 4.min(len) } else { 1 }) % len,
            (_, Some((Dir::Right, big))) => self.step_cursor = (cur + if big { 4 } else { 1 }) % len,
            (_, Some((d @ (Dir::Up | Dir::Down), octave))) => {
                let delta: i32 = if octave { 12 } else { 1 } * if d == Dir::Up { 1 } else { -1 };
                let note = (step.note as i32 + delta).clamp(0, 127) as u8;
                set(self, Step { on: true, note, ..step });
            }
            (Input::Key(Key::Enter, _), _) | (Input::Char('x'), _) => {
                set(self, Step { on: !step.on, ..step });
            }
            (Input::Char('v'), _) => {
                let vel = match step.vel {
                    v if v < 0.5 => 0.6,
                    v if v < 0.7 => 0.8,
                    v if v < 0.9 => 1.0,
                    _ => 0.4,
                };
                set(self, Step { vel, ..step });
            }
            (Input::Char('<'), _) | (Input::Char('>'), _) => {
                let right = matches!(input, Input::Char('>'));
                self.session.edit_steps(id, |d| if right { d.steps[..len].rotate_right(1) } else { d.steps[..len].rotate_left(1) });
            }
            (Input::Char('C'), _) => self.session.edit_steps(id, |d| d.steps[..len].iter_mut().for_each(|s| s.on = false)),
            // Copy the step at the cursor onto the next one.
            (Input::Char('p'), _) if cur + 1 < len.min(MAX_STEPS) => {
                self.session.edit_steps(id, |d| d.steps[cur + 1] = step);
                self.step_cursor = cur + 1;
            }
            _ => {}
        }
    }

    fn insert_key(&mut self, input: Input, mut buf: String, mut cursor: usize) {
        let Some(id) = self.selected else { return };
        let byte = |s: &str, c: usize| s.char_indices().nth(c).map(|(i, _)| i).unwrap_or(s.len());
        match input {
            Input::Key(Key::Escape, _) => {
                self.mode = Mode::Edit(Focus::Body);
                return;
            }
            Input::Key(Key::Enter, _) => {
                match self.session.set_pattern(id, &buf) {
                    Ok(()) => {
                        self.pattern_errors.remove(&id);
                        self.info("pattern queued for next bar");
                        self.mode = Mode::Edit(Focus::Body);
                    }
                    Err(e) => {
                        self.error(e.to_string());
                        self.pattern_errors.insert(id, e);
                        self.mode = Mode::Insert { buf, cursor };
                    }
                }
                return;
            }
            Input::Char(c) => {
                buf.insert(byte(&buf, cursor), c);
                cursor += 1;
            }
            Input::Key(Key::Backspace, _) if cursor > 0 => {
                cursor -= 1;
                buf.remove(byte(&buf, cursor));
            }
            Input::Key(Key::Delete, _) if cursor < buf.chars().count() => {
                buf.remove(byte(&buf, cursor));
            }
            Input::Key(Key::ArrowLeft, _) => cursor = cursor.saturating_sub(1),
            Input::Key(Key::ArrowRight, _) => cursor = (cursor + 1).min(buf.chars().count()),
            Input::Key(Key::Home, _) | Input::Key(Key::ArrowUp, _) => cursor = 0,
            Input::Key(Key::End, _) | Input::Key(Key::ArrowDown, _) => cursor = buf.chars().count(),
            _ => {}
        }
        // Live syntax check while typing.
        match pattern::parse(&buf) {
            Ok(_) => self.pattern_errors.remove(&id),
            Err(e) => self.pattern_errors.insert(id, e),
        };
        self.mode = Mode::Insert { buf, cursor };
    }

    fn value_key(&mut self, input: Input, mut buf: String) {
        match input {
            Input::Key(Key::Escape, _) => self.mode = Mode::Edit(Focus::Params),
            Input::Key(Key::Backspace, _) => {
                buf.pop();
                self.mode = Mode::Value(buf);
            }
            Input::Key(Key::Enter, _) => {
                if let Some(row) = self.selected.and_then(|id| Row::all(&self.session, id).get(self.param_focus).map(|r| (id, *r))) {
                    let (id, row) = row;
                    let spec = row.spec(&self.session, id);
                    let parsed = buf
                        .trim()
                        .parse::<f32>()
                        .ok()
                        .or_else(|| spec.labels.iter().position(|l| l.starts_with(buf.trim())).map(|i| spec.min + i as f32));
                    match parsed {
                        Some(v) => row.set(&mut self.session, id, v),
                        None => self.error(format!("not a value: {buf}")),
                    }
                }
                self.mode = Mode::Edit(Focus::Params);
            }
            Input::Char(c) => {
                buf.push(c);
                self.mode = Mode::Value(buf);
            }
            _ => {}
        }
    }

    fn command_key(&mut self, input: Input, mut buf: String) {
        match input {
            Input::Key(Key::Escape, _) => self.mode = Mode::Graph,
            Input::Key(Key::Backspace, _) => {
                if buf.pop().is_none() {
                    self.mode = Mode::Graph;
                } else {
                    self.mode = Mode::Command(buf);
                }
            }
            Input::Key(Key::Enter, _) => {
                self.mode = Mode::Graph;
                self.run_command(buf.trim());
            }
            Input::Char(c) => {
                buf.push(c);
                self.mode = Mode::Command(buf);
            }
            _ => {}
        }
    }

    fn run_command(&mut self, cmd: &str) {
        let (name, arg) = cmd.split_once(' ').map(|(a, b)| (a, b.trim())).unwrap_or((cmd, ""));
        match name {
            "bpm" => match arg.parse::<f32>() {
                Ok(b) => self.session.set_bpm(b),
                Err(_) => self.error("usage: bpm 120"),
            },
            "w" | "write" | "save" => {
                if !arg.is_empty() {
                    self.project_name = arg.to_string();
                }
                let dir = projects_dir();
                let path = dir.join(format!("{}.json", self.project_name));
                let res = std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&path, self.session.to_json()));
                match res {
                    Ok(()) => self.info(format!("saved {}", path.display())),
                    Err(e) => self.error(format!("save failed: {e}")),
                }
            }
            "e" | "open" | "o" => {
                if arg.is_empty() {
                    let mut names: Vec<String> = std::fs::read_dir(projects_dir())
                        .into_iter()
                        .flatten()
                        .flatten()
                        .filter_map(|e| e.file_name().to_str()?.strip_suffix(".json").map(str::to_string))
                        .filter(|n| n != "autosave")
                        .collect();
                    names.sort();
                    if names.is_empty() {
                        self.info(format!("no projects in {}", projects_dir().display()));
                    } else {
                        self.mode = Mode::Pick(Picker::new(PickKind::Open(names)));
                    }
                } else {
                    self.open_project(arg);
                }
            }
            "new" => {
                self.snapshot();
                self.load(Project::default(), "untitled");
            }
            "demo" => {
                self.snapshot();
                self.load(Project::demo(), "demo");
            }
            "device" | "dev" => {
                let devices: Vec<String> = audio::list_devices().into_iter().map(|(n, ch)| format!("{n}  ({ch} ch)")).collect();
                if devices.is_empty() {
                    self.error("no output devices found");
                } else {
                    self.mode = Mode::Pick(Picker::new(PickKind::Device(devices)));
                }
            }
            "q" | "quit" => self.quit = true,
            "help" => self.mode = Mode::Help,
            "" => {}
            _ => self.error(format!("unknown command: {name}  (try :help)")),
        }
    }

    fn load(&mut self, project: Project, name: &str) {
        let device = project.device.clone();
        let same_device = device == self.session.project.device;
        self.playing = false;
        self.session.load(project);
        self.pattern_errors.clear();
        self.project_name = name.to_string();
        self.selected = self.session.project.nodes.first().map(|n| n.id);
        self.cam_ready = false;
        if !same_device {
            self.open_device(device.as_deref());
        }
    }

    fn open_project(&mut self, name: &str) {
        let path = projects_dir().join(format!("{name}.json"));
        match std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|s| Session::project_from_json(&s).map_err(|e| e.to_string()))
        {
            Ok(p) => {
                self.snapshot();
                self.load(p, name);
                self.info(format!("opened {name}"));
            }
            Err(e) => self.error(format!("can't open {}: {e}", path.display())),
        }
    }

    fn pick_key(&mut self, input: Input, mut p: Picker) {
        let entries = p.entries(&self.session);
        match input {
            Input::Key(Key::Escape, _) => {
                self.mode = Mode::Graph;
                return;
            }
            Input::Key(Key::ArrowDown, _) | Input::Key(Key::Tab, _) => p.idx = (p.idx + 1) % entries.len().max(1),
            Input::Key(Key::ArrowUp, _) => p.idx = (p.idx + entries.len().max(1) - 1) % entries.len().max(1),
            Input::Key(Key::Backspace, _) => {
                p.query.pop();
                p.idx = 0;
            }
            Input::Char(c) => {
                p.query.push(c);
                p.idx = 0;
            }
            Input::Key(Key::Enter, _) => {
                self.mode = Mode::Graph;
                if let Some((_, i)) = entries.get(p.idx) {
                    self.pick(&p.kind, *i);
                }
                return;
            }
            _ => {}
        }
        self.mode = Mode::Pick(p);
    }

    fn pick(&mut self, kind: &PickKind, i: usize) {
        match kind {
            PickKind::AddNode => self.add_node(NodeKind::ALL[i]),
            PickKind::Disconnect(edges) => {
                self.snapshot();
                let (a, b) = edges[i];
                self.session.disconnect(a, b);
                self.info("disconnected");
            }
            PickKind::Device(names) => {
                let name = names[i].rsplit_once("  (").map(|(n, _)| n).unwrap_or(&names[i]).to_string();
                self.open_device(Some(&name));
            }
            PickKind::Open(names) => {
                let name = names[i].clone();
                self.open_project(&name);
            }
        }
    }
}
