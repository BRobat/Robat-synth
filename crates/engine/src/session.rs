//! UI-thread owner of the project. Every edit updates the model and is
//! mirrored to the audio thread as commands.

use crate::core::{new_engine, Command, EngineCore, EngineHandle, Meters, Slot, SlotInput, Structure};
use crate::model::{Edge, NodeKind, NodeModel, Port, Project, SeqData, CABLE_LEVEL};
use crate::nodes::{make_processor, pattern_table, steps_data, NodeData};
use pattern::ParseError;
use std::collections::VecDeque;

pub struct Session {
    pub project: Project,
    engine: Option<EngineHandle>,
    sr: f32,
    /// Commands waiting for room in the ring buffer (kept in order).
    backlog: VecDeque<Command>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum EditError {
    Full,
    NoSuchNode,
    Incompatible,
    Duplicate,
}

impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            EditError::Full => "node limit reached",
            EditError::NoSuchNode => "no such node",
            EditError::Incompatible => "those ports don't fit",
            EditError::Duplicate => "already connected",
        })
    }
}

impl Session {
    pub fn new(project: Project) -> Self {
        Session { project, engine: None, sr: 48000.0, backlog: VecDeque::new() }
    }

    /// A session wired to an engine core that the caller drives directly
    /// (offline rendering and tests).
    pub fn offline(project: Project, sr: f32) -> (Session, EngineCore) {
        let (handle, core) = new_engine(sr);
        let mut s = Session::new(project);
        s.attach(handle, sr);
        (s, core)
    }

    /// Connects to a (new) engine and uploads the whole project.
    pub fn attach(&mut self, handle: EngineHandle, sr: f32) {
        self.engine = Some(handle);
        self.sr = sr;
        self.backlog.clear();
        for node in self.project.nodes.clone() {
            self.send(Command::AddProc(node.id, make_processor(&node, sr)));
        }
        self.send(Command::Bpm(self.project.bpm));
        self.rebuild();
    }

    pub fn detach(&mut self) {
        self.engine = None;
        self.backlog.clear();
    }

    pub fn meters(&self) -> Option<&Meters> {
        self.engine.as_ref().map(|e| &*e.meters)
    }

    /// Call regularly (every UI frame): flushes queued commands and frees
    /// memory the audio thread handed back.
    pub fn pump(&mut self) {
        let Some(engine) = self.engine.as_mut() else {
            self.backlog.clear();
            return;
        };
        engine.collect_garbage();
        while let Some(cmd) = self.backlog.pop_front() {
            if let Err(cmd) = engine.try_send(cmd) {
                self.backlog.push_front(cmd);
                break;
            }
        }
    }

    fn send(&mut self, cmd: Command) {
        if self.engine.is_some() {
            self.backlog.push_back(cmd);
            self.pump();
        }
    }

    fn rebuild(&mut self) {
        let order = self.project.process_order();
        let index_of = |id: u32| order.iter().position(|x| *x == id);
        let slots = order
            .iter()
            .map(|&id| {
                let node = self.project.node(id).unwrap();
                let (mut audio, mut events) = (Vec::new(), Vec::new());
                for e in self.project.edges.iter().filter(|e| e.to == id) {
                    let src = self.project.node(e.from).unwrap();
                    let slot = index_of(e.from).unwrap();
                    match src.kind.output() {
                        Port::Audio => audio.push(SlotInput::new(slot, e.from, e.gain)),
                        // Events only ever flow forwards: no node both takes and makes them.
                        Port::Events => events.push(slot),
                        Port::None => {}
                    }
                }
                Slot::new(id, node.muted, audio, events)
            })
            .collect();
        self.send(Command::Structure(Box::new(Structure { slots })));
    }

    pub fn add_node(&mut self, kind: NodeKind, x: i32, y: i32) -> Result<u32, EditError> {
        let id = self.project.free_id().ok_or(EditError::Full)?;
        let node = NodeModel::new(id, kind, x, y);
        self.send(Command::AddProc(id, make_processor(&node, self.sr)));
        self.project.nodes.push(node);
        self.rebuild();
        Ok(id)
    }

    /// Adds a copy of an existing node (params and sequence included).
    pub fn duplicate_node(&mut self, id: u32, x: i32, y: i32) -> Result<u32, EditError> {
        let mut node = self.project.node(id).ok_or(EditError::NoSuchNode)?.clone();
        node.id = self.project.free_id().ok_or(EditError::Full)?;
        (node.x, node.y) = (x, y);
        self.send(Command::AddProc(node.id, make_processor(&node, self.sr)));
        let new_id = node.id;
        self.project.nodes.push(node);
        self.rebuild();
        Ok(new_id)
    }

    pub fn remove_node(&mut self, id: u32) {
        self.project.nodes.retain(|n| n.id != id);
        self.project.edges.retain(|e| e.from != id && e.to != id);
        self.rebuild();
        self.send(Command::RemoveProc(id));
    }

    pub fn move_node(&mut self, id: u32, x: i32, y: i32) {
        if let Some(n) = self.project.node_mut(id) {
            (n.x, n.y) = (x, y);
        }
    }

    pub fn connect(&mut self, from: u32, to: u32) -> Result<(), EditError> {
        let a = self.project.node(from).ok_or(EditError::NoSuchNode)?.kind;
        let b = self.project.node(to).ok_or(EditError::NoSuchNode)?.kind;
        if !a.can_feed(b) {
            return Err(EditError::Incompatible);
        }
        if self.project.edge(from, to).is_some() {
            return Err(EditError::Duplicate);
        }
        // Loops (and a node feeding itself) are fine: see `Project::feedback_edges`.
        self.project.edges.push(Edge::new(from, to));
        self.rebuild();
        Ok(())
    }

    pub fn disconnect(&mut self, from: u32, to: u32) {
        self.project.edges.retain(|e| (e.from, e.to) != (from, to));
        self.rebuild();
    }

    /// Sets the level of the cable `from -> to`.
    pub fn set_edge_gain(&mut self, from: u32, to: u32, gain: f32) {
        let Some(e) = self.project.edges.iter_mut().find(|e| (e.from, e.to) == (from, to)) else { return };
        e.gain = CABLE_LEVEL.clamp(gain);
        let gain = e.gain;
        self.send(Command::EdgeGain { from, to, gain });
    }

    pub fn set_param(&mut self, id: u32, idx: usize, value: f32) {
        let Some(node) = self.project.node_mut(id) else { return };
        let Some(spec) = node.kind.params().get(idx) else { return };
        let value = spec.clamp(value);
        node.params[idx] = value;
        self.send(Command::Param { node: id, idx, value });
    }

    pub fn toggle_mute(&mut self, id: u32) {
        if let Some(n) = self.project.node_mut(id) {
            n.muted = !n.muted;
            self.rebuild();
        }
    }

    /// Edits a sequencer's steps and uploads them.
    pub fn edit_steps(&mut self, id: u32, f: impl FnOnce(&mut SeqData)) {
        let Some(seq) = self.project.node_mut(id).and_then(|n| n.seq.as_mut()) else { return };
        f(seq);
        let data = steps_data(&seq.steps);
        self.send(Command::Data { node: id, data });
    }

    /// Sets a sequencer's pattern text. On a parse error the old pattern
    /// keeps playing; the text is stored either way.
    pub fn set_pattern(&mut self, id: u32, src: &str) -> Result<(), ParseError> {
        let Some(seq) = self.project.node_mut(id).and_then(|n| n.seq.as_mut()) else { return Ok(()) };
        seq.pattern = src.to_string();
        let table = pattern_table(src)?;
        self.send(Command::Data { node: id, data: NodeData::Table(table) });
        Ok(())
    }

    pub fn set_bpm(&mut self, bpm: f32) {
        self.project.bpm = bpm.clamp(20.0, 400.0);
        self.send(Command::Bpm(self.project.bpm));
    }

    pub fn play(&mut self, on: bool) {
        self.send(Command::Play(on));
    }

    /// Replaces the whole project (e.g. on load) and stops the transport.
    pub fn load(&mut self, project: Project) {
        self.send(Command::Play(false));
        self.restore(project);
    }

    /// Replaces the whole project without touching the transport (undo).
    pub fn restore(&mut self, project: Project) {
        let old: Vec<u32> = self.project.nodes.iter().map(|n| n.id).collect();
        self.project = project;
        for id in old {
            self.send(Command::RemoveProc(id));
        }
        for node in self.project.nodes.clone() {
            self.send(Command::AddProc(node.id, make_processor(&node, self.sr)));
        }
        self.send(Command::Bpm(self.project.bpm));
        self.rebuild();
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(&self.project).expect("project serializes")
    }

    pub fn project_from_json(s: &str) -> Result<Project, serde_json::Error> {
        let mut p: Project = serde_json::from_str(s)?;
        // Tolerate files from older versions with fewer params.
        for n in p.nodes.iter_mut() {
            let specs = n.kind.params();
            n.params.resize(specs.len(), 0.0);
            for (i, spec) in specs.iter().enumerate() {
                n.params[i] = spec.clamp(n.params[i]);
            }
            if n.kind == NodeKind::Seq && n.seq.is_none() {
                n.seq = Some(SeqData::default());
            }
        }
        p.nodes.retain(|n| (n.id as usize) < crate::model::MAX_NODES);
        p.nodes.dedup_by_key(|n| n.id);
        let ids: Vec<u32> = p.nodes.iter().map(|n| n.id).collect();
        p.edges.retain(|e| ids.contains(&e.from) && ids.contains(&e.to));
        let mut seen = Vec::new();
        p.edges.retain(|e| {
            let key = (e.from, e.to);
            let fresh = !seen.contains(&key);
            seen.push(key);
            fresh
        });
        Ok(p)
    }
}

/// Renders `seconds` of audio from a project, interleaved over `channels`.
pub fn render_offline(project: Project, sr: f32, channels: usize, seconds: f32) -> Vec<f32> {
    let (mut session, mut core) = Session::offline(project, sr);
    session.play(true);
    let frames = (seconds * sr) as usize;
    let mut out = vec![0.0; frames * channels];
    for chunk in out.chunks_mut(512 * channels) {
        session.pump();
        core.process(chunk, channels);
    }
    out
}
