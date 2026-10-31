//! WebAssembly bridge for the browser build.
//!
//! The same module is instantiated twice in the page:
//!
//! * on the main thread **without** an engine: it owns the project model
//!   (validation, ids, pattern errors, JSON) for the touch UI;
//! * inside the `AudioWorklet` **with** an engine: it receives the very same
//!   edit ops and renders audio.
//!
//! Both sides apply identical ops in identical order, so their projects stay
//! in sync without sharing memory (GitHub Pages can't serve the headers
//! `SharedArrayBuffer` needs).
//!
//! The ABI is plain C (`rb_*`), no wasm-bindgen, so the worklet can load it
//! with nothing but `WebAssembly.Instance`. Ops and results are UTF-8 JSON.

use engine::model::{Curve, NodeKind, Port, Project, Step, MAX_NODES, MAX_STEPS};
use engine::{EngineCore, Session};
use serde::Deserialize;
use serde_json::{json, Value};
use std::cell::RefCell;

/// Device channels rendered internally (all 8 `out` pairs), folded to stereo.
const CHANNELS: usize = 16;
/// Floats in the meter snapshot: cycle, playing, then (level, triggers, step) per node.
pub const METER_LEN: usize = 2 + 3 * MAX_NODES;

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    Specs,
    Project,
    /// Replace the project and stop the transport.
    Load { project: Value },
    /// Replace the project, keep the transport running (undo).
    Restore { project: Value },
    Demo,
    New,
    AddNode { kind: String, x: i32, y: i32 },
    Duplicate { id: u32, x: i32, y: i32 },
    Remove { id: u32 },
    Move { id: u32, x: i32, y: i32 },
    Connect { from: u32, to: u32 },
    Disconnect { from: u32, to: u32 },
    Param { id: u32, idx: usize, value: f32 },
    Mute { id: u32 },
    Steps { id: u32, steps: Vec<Step> },
    Pattern { id: u32, src: String },
    Bpm { bpm: f32 },
    Play { on: bool },
}

pub struct Bridge {
    session: Session,
    core: Option<EngineCore>,
    /// Interleaved 16-channel render buffer and the stereo fold-down.
    wide: Vec<f32>,
    stereo: Vec<f32>,
    meters: Vec<f32>,
}

fn kind_from(name: &str) -> Option<NodeKind> {
    NodeKind::ALL.into_iter().find(|k| k.name() == name)
}

fn port_name(p: Port) -> &'static str {
    match p {
        Port::None => "none",
        Port::Events => "events",
        Port::Audio => "audio",
    }
}

fn specs() -> Value {
    let kinds: Vec<Value> = NodeKind::ALL
        .iter()
        .map(|k| {
            let params: Vec<Value> = k
                .params()
                .iter()
                .map(|p| {
                    json!({
                        "name": p.name, "min": p.min, "max": p.max, "default": p.default,
                        "curve": match p.curve { Curve::Lin => "lin", Curve::Log => "log", Curve::Int => "int" },
                        "unit": p.unit, "labels": p.labels,
                    })
                })
                .collect();
            json!({ "name": k.name(), "input": port_name(k.input()), "output": port_name(k.output()), "params": params })
        })
        .collect();
    json!({ "kinds": kinds, "maxNodes": MAX_NODES, "maxSteps": MAX_STEPS })
}

fn valid(id: u32) -> Result<u32, String> {
    if (id as usize) < MAX_NODES {
        Ok(id)
    } else {
        Err("no such node".into())
    }
}

fn parse_project(v: Value) -> Result<Project, String> {
    Session::project_from_json(&v.to_string()).map_err(|e| format!("bad project: {e}"))
}

impl Bridge {
    pub fn new(sample_rate: f32, with_engine: bool) -> Self {
        let (session, core) = if with_engine {
            let (s, c) = Session::offline(Project::default(), sample_rate);
            (s, Some(c))
        } else {
            (Session::new(Project::default()), None)
        };
        Bridge { session, core, wide: Vec::new(), stereo: Vec::new(), meters: vec![0.0; METER_LEN] }
    }

    /// Applies one JSON op and returns the JSON result. The main-thread
    /// instance also returns the whole project so the UI can redraw.
    pub fn call(&mut self, src: &str) -> String {
        let (ok, extra) = match serde_json::from_str::<Op>(src) {
            Ok(op) => match self.apply(op) {
                Ok(v) => (true, v),
                Err(e) => (false, json!({ "error": e })),
            },
            Err(e) => (false, json!({ "error": format!("bad op: {e}") })),
        };
        let mut out = match extra {
            Value::Object(m) => m,
            v => {
                let mut m = serde_json::Map::new();
                m.insert("value".into(), v);
                m
            }
        };
        out.insert("ok".into(), ok.into());
        if self.core.is_none() {
            out.insert("project".into(), serde_json::to_value(&self.session.project).unwrap_or(Value::Null));
        }
        Value::Object(out).to_string()
    }

    fn apply(&mut self, op: Op) -> Result<Value, String> {
        let s = &mut self.session;
        let none = || Ok(json!({}));
        match op {
            Op::Specs => Ok(specs()),
            Op::Project => none(),
            Op::Load { project } => {
                s.load(parse_project(project)?);
                none()
            }
            Op::Restore { project } => {
                s.restore(parse_project(project)?);
                none()
            }
            Op::Demo => {
                s.load(Project::demo());
                none()
            }
            Op::New => {
                s.load(Project { bpm: s.project.bpm, ..Project::default() });
                none()
            }
            Op::AddNode { kind, x, y } => {
                let kind = kind_from(&kind).ok_or("unknown node kind")?;
                let id = s.add_node(kind, x, y).map_err(|e| e.to_string())?;
                Ok(json!({ "id": id }))
            }
            Op::Duplicate { id, x, y } => {
                let id = s.duplicate_node(valid(id)?, x, y).map_err(|e| e.to_string())?;
                Ok(json!({ "id": id }))
            }
            Op::Remove { id } => {
                s.remove_node(valid(id)?);
                none()
            }
            Op::Move { id, x, y } => {
                s.move_node(id, x, y);
                none()
            }
            Op::Connect { from, to } => {
                s.connect(valid(from)?, valid(to)?).map_err(|e| e.to_string())?;
                none()
            }
            Op::Disconnect { from, to } => {
                s.disconnect(from, to);
                none()
            }
            Op::Param { id, idx, value } => {
                if value.is_finite() {
                    s.set_param(valid(id)?, idx, value);
                }
                none()
            }
            Op::Mute { id } => {
                s.toggle_mute(valid(id)?);
                none()
            }
            Op::Steps { id, steps } => {
                s.edit_steps(valid(id)?, |d| {
                    for (dst, src) in d.steps.iter_mut().zip(&steps) {
                        *dst = Step { note: src.note.min(127), vel: src.vel.clamp(0.0, 1.0), ..*src };
                    }
                });
                none()
            }
            Op::Pattern { id, src } => {
                // The text is stored even when it doesn't parse; the old
                // pattern keeps playing.
                match s.set_pattern(valid(id)?, &src) {
                    Ok(()) => none(),
                    Err(e) => Ok(json!({ "patternError": { "pos": e.pos, "msg": e.msg, "text": e.to_string() } })),
                }
            }
            Op::Bpm { bpm } => {
                if bpm.is_finite() {
                    s.set_bpm(bpm);
                }
                none()
            }
            Op::Play { on } => {
                s.play(on);
                none()
            }
        }
    }

    /// Renders `frames` of stereo audio (interleaved L/R). Every `out` pair
    /// is summed into the one stereo output a phone has.
    pub fn process(&mut self, frames: usize) -> &[f32] {
        self.wide.resize(frames * CHANNELS, 0.0);
        self.stereo.resize(frames * 2, 0.0);
        let Some(core) = self.core.as_mut() else {
            self.stereo.fill(0.0);
            return &self.stereo;
        };
        self.session.pump();
        core.process(&mut self.wide, CHANNELS);
        for (frame, st) in self.wide.chunks_exact(CHANNELS).zip(self.stereo.chunks_exact_mut(2)) {
            let (mut l, mut r) = (0.0, 0.0);
            for pair in frame.chunks_exact(2) {
                l += pair[0];
                r += pair[1];
            }
            st[0] = l;
            st[1] = r;
        }
        &self.stereo
    }

    /// Snapshot of the engine meters for the visuals (see [`METER_LEN`]).
    pub fn meters(&mut self) -> &[f32] {
        if let Some(m) = self.session.meters() {
            use std::sync::atomic::Ordering::Relaxed;
            self.meters[0] = m.cycle() as f32;
            self.meters[1] = if m.playing.load(Relaxed) { 1.0 } else { 0.0 };
            for id in 0..MAX_NODES {
                let base = 2 + 3 * id;
                self.meters[base] = m.level(id as u32);
                // Wraps harmlessly; the UI only looks for changes.
                self.meters[base + 1] = (m.triggers[id].load(Relaxed) % (1 << 24)) as f32;
                self.meters[base + 2] = m.position[id].load(Relaxed) as f32;
            }
        }
        &self.meters
    }
}

// ---- C ABI ---------------------------------------------------------------

#[derive(Default)]
struct Global {
    bridge: Option<Bridge>,
    input: Vec<u8>,
    result: Vec<u8>,
}

thread_local! {
    static G: RefCell<Global> = RefCell::new(Global::default());
}

/// Creates the bridge. `with_engine != 0` inside the AudioWorklet.
#[no_mangle]
pub extern "C" fn rb_init(sample_rate: f32, with_engine: u32) {
    G.with_borrow_mut(|g| g.bridge = Some(Bridge::new(sample_rate, with_engine != 0)));
}

/// Returns a buffer of `len` bytes for the caller to write an op into.
#[no_mangle]
pub extern "C" fn rb_input(len: u32) -> *mut u8 {
    G.with_borrow_mut(|g| {
        g.input.resize(len as usize, 0);
        g.input.as_mut_ptr()
    })
}

/// Runs the op in the input buffer; the result is at [`rb_result`].
/// Returns the result length in bytes.
#[no_mangle]
pub extern "C" fn rb_call(len: u32) -> u32 {
    G.with_borrow_mut(|g| {
        let Some(bridge) = g.bridge.as_mut() else { return 0 };
        let n = (len as usize).min(g.input.len());
        let src = String::from_utf8_lossy(&g.input[..n]);
        g.result = bridge.call(&src).into_bytes();
        g.result.len() as u32
    })
}

#[no_mangle]
pub extern "C" fn rb_result() -> *const u8 {
    G.with_borrow(|g| g.result.as_ptr())
}

/// Renders `frames` of interleaved stereo; returns a pointer to it.
#[no_mangle]
pub extern "C" fn rb_process(frames: u32) -> *const f32 {
    G.with_borrow_mut(|g| match g.bridge.as_mut() {
        Some(b) => b.process(frames as usize).as_ptr(),
        None => std::ptr::null(),
    })
}

/// Fills and returns the meter snapshot ([`METER_LEN`] floats).
#[no_mangle]
pub extern "C" fn rb_meters() -> *const f32 {
    G.with_borrow_mut(|g| match g.bridge.as_mut() {
        Some(b) => b.meters().as_ptr(),
        None => std::ptr::null(),
    })
}

#[no_mangle]
pub extern "C" fn rb_meter_len() -> u32 {
    METER_LEN as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(b: &mut Bridge, op: Value) -> Value {
        serde_json::from_str(&b.call(&op.to_string())).unwrap()
    }

    #[test]
    fn mirrored_instances_stay_in_sync_and_make_sound() {
        let mut ui = Bridge::new(48000.0, false);
        let mut audio = Bridge::new(48000.0, true);
        let ops = [
            json!({"op": "demo"}),
            json!({"op": "add_node", "kind": "amp", "x": 4, "y": 0}),
            json!({"op": "param", "id": 1, "idx": 8, "value": 900.0}),
            json!({"op": "pattern", "id": 0, "src": "c2 [eb2 g2]"}),
            json!({"op": "bpm", "bpm": 130}),
            json!({"op": "play", "on": true}),
        ];
        for op in &ops {
            let r = call(&mut ui, op.clone());
            assert_eq!(r["ok"], true, "{op}: {r}");
            assert_eq!(call(&mut audio, op.clone())["ok"], true);
        }
        let r = call(&mut ui, json!({"op": "add_node", "kind": "seq", "x": 0, "y": 5}));
        assert_eq!(r["id"], 8);
        assert_eq!(ui.session.project.nodes.len(), 9);
        assert_eq!(ui.session.project.bpm, 130.0);

        let mut energy = 0.0;
        for _ in 0..400 {
            energy += audio.process(128).iter().map(|x| x * x).sum::<f32>();
        }
        assert!(energy > 1.0, "silent: {energy}");
        assert!(audio.meters()[0] > 0.0);
    }

    #[test]
    fn errors_are_reported_not_panics() {
        let mut b = Bridge::new(48000.0, true);
        call(&mut b, json!({"op": "demo"}));
        for op in [
            json!({"op": "nope"}),
            json!({"op": "remove", "id": 9999}),
            json!({"op": "connect", "from": 0, "to": 2}),
            json!({"op": "add_node", "kind": "laser", "x": 0, "y": 0}),
            json!({"op": "load", "project": {"bpm": "fast"}}),
        ] {
            assert_eq!(call(&mut b, op)["ok"], false);
        }
        let r = call(&mut b, json!({"op": "pattern", "id": 0, "src": "c3 [e3"}));
        assert!(r["patternError"]["msg"].is_string());
        assert!(call(&mut b, json!({"op": "specs"}))["kinds"].as_array().unwrap().len() == 5);
    }
}
