//! The audio-thread half of the engine.

use crate::model::MAX_NODES;
use crate::nodes::{Ctx, Io, NodeData, NoteEvent, NoteKind, Processor, Transport};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::Relaxed};
use std::sync::Arc;

/// Frames processed per internal block, whatever the device buffer size.
pub const BLOCK: usize = 64;
const EVENT_CAP: usize = 256;
const RING: usize = 4096;

pub struct Slot {
    pub id: u32,
    pub muted: bool,
    /// Indices of earlier slots feeding audio / events into this one.
    pub audio_in: Vec<usize>,
    pub event_in: Vec<usize>,
    in_audio: [Vec<f32>; 2],
    in_events: Vec<NoteEvent>,
    out_audio: [Vec<f32>; 2],
    out_events: Vec<NoteEvent>,
}

impl Slot {
    pub fn new(id: u32, muted: bool, audio_in: Vec<usize>, event_in: Vec<usize>) -> Self {
        Slot {
            id,
            muted,
            audio_in,
            event_in,
            in_audio: [vec![0.0; BLOCK], vec![0.0; BLOCK]],
            in_events: Vec::with_capacity(EVENT_CAP),
            out_audio: [vec![0.0; BLOCK], vec![0.0; BLOCK]],
            out_events: Vec::with_capacity(EVENT_CAP),
        }
    }
}

/// The compiled graph: slots in dependency order, with preallocated buffers.
#[derive(Default)]
pub struct Structure {
    pub slots: Vec<Slot>,
}

pub enum Command {
    Structure(Box<Structure>),
    AddProc(u32, Box<dyn Processor>),
    RemoveProc(u32),
    Param { node: u32, idx: usize, value: f32 },
    Data { node: u32, data: NodeData },
    Play(bool),
    Bpm(f32),
}

/// Values handed back by the audio thread purely to be dropped elsewhere.
#[allow(dead_code)]
pub enum Garbage {
    Structure(Box<Structure>),
    Proc(Box<dyn Processor>),
    Data(NodeData),
}

/// Lock-free values the engine publishes for the UI's visuals.
pub struct Meters {
    /// Output RMS per node (f32 bits).
    pub level: Vec<AtomicU32>,
    /// Count of note-ons emitted or received per node.
    pub triggers: Vec<AtomicU32>,
    /// Playhead step per node.
    pub position: Vec<AtomicU32>,
    /// Transport position in cycles (f64 bits).
    pub cycle: AtomicU64,
    pub playing: AtomicBool,
    /// Device channels currently clipping above 0 dBFS (bitmask of pairs).
    pub clipping: AtomicU32,
}

impl Meters {
    fn new() -> Self {
        let atomics = || (0..MAX_NODES).map(|_| AtomicU32::new(0)).collect();
        Meters {
            level: atomics(),
            triggers: atomics(),
            position: atomics(),
            cycle: AtomicU64::new(0),
            playing: AtomicBool::new(false),
            clipping: AtomicU32::new(0),
        }
    }

    pub fn level(&self, id: u32) -> f32 {
        f32::from_bits(self.level[id as usize].load(Relaxed))
    }

    pub fn cycle(&self) -> f64 {
        f64::from_bits(self.cycle.load(Relaxed))
    }
}

/// UI-thread handle: send commands, collect garbage, read meters.
pub struct EngineHandle {
    commands: rtrb::Producer<Command>,
    garbage: rtrb::Consumer<Garbage>,
    pub meters: Arc<Meters>,
}

impl EngineHandle {
    /// Returns the command back if the queue is full.
    pub fn try_send(&mut self, cmd: Command) -> Result<(), Command> {
        self.commands.push(cmd).map_err(|rtrb::PushError::Full(c)| c)
    }

    pub fn collect_garbage(&mut self) {
        while self.garbage.pop().is_ok() {}
    }
}

pub struct EngineCore {
    sr: f32,
    commands: rtrb::Consumer<Command>,
    garbage: rtrb::Producer<Garbage>,
    meters: Arc<Meters>,
    procs: Vec<Option<Box<dyn Processor>>>,
    structure: Box<Structure>,
    playing: bool,
    bpm: f64,
    pos: f64,
}

pub fn new_engine(sample_rate: f32) -> (EngineHandle, EngineCore) {
    let (cmd_tx, cmd_rx) = rtrb::RingBuffer::new(RING);
    let (gar_tx, gar_rx) = rtrb::RingBuffer::new(RING);
    let meters = Arc::new(Meters::new());
    let handle = EngineHandle { commands: cmd_tx, garbage: gar_rx, meters: meters.clone() };
    let core = EngineCore {
        sr: sample_rate,
        commands: cmd_rx,
        garbage: gar_tx,
        meters,
        procs: (0..MAX_NODES).map(|_| None).collect(),
        structure: Box::default(),
        playing: false,
        bpm: 120.0,
        pos: 0.0,
    };
    (handle, core)
}

impl EngineCore {
    pub fn sample_rate(&self) -> f32 {
        self.sr
    }

    fn discard(&mut self, g: Garbage) {
        // If the UI stops collecting, dropping here is the lesser evil.
        let _ = self.garbage.push(g);
    }

    fn drain_commands(&mut self) {
        while let Ok(cmd) = self.commands.pop() {
            match cmd {
                Command::Structure(s) => {
                    let old = std::mem::replace(&mut self.structure, s);
                    self.discard(Garbage::Structure(old));
                }
                Command::AddProc(id, p) => {
                    if let Some(old) = self.procs[id as usize].replace(p) {
                        self.discard(Garbage::Proc(old));
                    }
                }
                Command::RemoveProc(id) => {
                    if let Some(old) = self.procs[id as usize].take() {
                        self.discard(Garbage::Proc(old));
                    }
                }
                Command::Param { node, idx, value } => {
                    if let Some(p) = self.procs[node as usize].as_mut() {
                        p.set_param(idx, value);
                    }
                }
                Command::Data { node, data } => match self.procs[node as usize].as_mut() {
                    Some(p) => {
                        let garbage = &mut self.garbage;
                        p.set_data(data, &mut |d| {
                            let _ = garbage.push(Garbage::Data(d));
                        });
                    }
                    None => self.discard(Garbage::Data(data)),
                },
                Command::Play(on) => {
                    if !on || !self.playing {
                        self.pos = 0.0;
                        for p in self.procs.iter_mut().flatten() {
                            p.stop();
                        }
                    }
                    self.playing = on;
                    self.meters.playing.store(on, Relaxed);
                }
                Command::Bpm(b) => self.bpm = b.clamp(20.0, 400.0) as f64,
            }
        }
    }

    /// Renders interleaved audio for `channels` device channels.
    pub fn process(&mut self, out: &mut [f32], channels: usize) {
        self.drain_commands();
        out.fill(0.0);
        if channels == 0 {
            return;
        }
        let total = out.len() / channels;
        let mut done = 0;
        let mut clipping = 0u32;
        while done < total {
            let n = BLOCK.min(total - done);
            self.process_block(n);
            for slot in &self.structure.slots {
                let Some(pair) = self.procs[slot.id as usize].as_ref().and_then(|p| p.out_pair()) else { continue };
                let (l, r) = (2 * pair, 2 * pair + 1);
                for f in 0..n {
                    let frame = &mut out[(done + f) * channels..(done + f + 1) * channels];
                    if channels == 1 && pair == 0 {
                        frame[0] += 0.5 * (slot.out_audio[0][f] + slot.out_audio[1][f]);
                    } else if r < channels {
                        frame[l] += slot.out_audio[0][f];
                        frame[r] += slot.out_audio[1][f];
                        if frame[l].abs() > 1.0 || frame[r].abs() > 1.0 {
                            clipping |= 1 << pair;
                        }
                    }
                }
            }
            done += n;
        }
        self.meters.clipping.store(clipping, Relaxed);
        self.meters.cycle.store(self.pos.to_bits(), Relaxed);
    }

    fn process_block(&mut self, n: usize) {
        let cps = self.bpm / 60.0 / 4.0 / self.sr as f64;
        let ctx = Ctx {
            sr: self.sr,
            frames: n,
            transport: Transport { playing: self.playing, bpm: self.bpm, pos: self.pos, cps },
        };
        let slots = &mut self.structure.slots;
        for i in 0..slots.len() {
            let (before, rest) = slots.split_at_mut(i);
            let slot = &mut rest[0];
            for ch in 0..2 {
                slot.in_audio[ch][..n].fill(0.0);
                for &j in &slot.audio_in {
                    for (d, s) in slot.in_audio[ch][..n].iter_mut().zip(&before[j].out_audio[ch][..n]) {
                        *d += *s;
                    }
                }
                slot.out_audio[ch][..n].fill(0.0);
            }
            slot.in_events.clear();
            for &j in &slot.event_in {
                for e in &before[j].out_events {
                    if slot.in_events.len() < slot.in_events.capacity() {
                        slot.in_events.push(*e);
                    }
                }
            }
            slot.in_events.sort_unstable_by_key(NoteEvent::sort_key);
            slot.out_events.clear();

            let id = slot.id as usize;
            let Some(proc) = self.procs[id].as_mut() else { continue };
            let [out_l, out_r] = &mut slot.out_audio;
            proc.process(
                &ctx,
                Io {
                    audio_in: [&slot.in_audio[0][..n], &slot.in_audio[1][..n]],
                    events_in: &slot.in_events,
                    audio_out: [&mut out_l[..n], &mut out_r[..n]],
                    events_out: &mut slot.out_events,
                },
            );
            if slot.muted {
                out_l[..n].fill(0.0);
                out_r[..n].fill(0.0);
                slot.out_events.clear();
            }
            if let Some(d) = proc.take_garbage() {
                let _ = self.garbage.push(Garbage::Data(d));
            }

            // Meters: smoothed RMS, note-on count, playhead.
            let sq: f32 = out_l[..n].iter().chain(&out_r[..n]).map(|x| x * x).sum();
            let rms = (sq / (2 * n) as f32).sqrt();
            let prev = f32::from_bits(self.meters.level[id].load(Relaxed));
            let level = if rms > prev { rms } else { prev * 0.97 + rms * 0.03 };
            self.meters.level[id].store(level.to_bits(), Relaxed);
            let ons = slot.out_events.iter().chain(&slot.in_events).filter(|e| e.kind == NoteKind::On).count();
            if ons > 0 {
                self.meters.triggers[id].fetch_add(ons as u32, Relaxed);
            }
            self.meters.position[id].store(proc.position(), Relaxed);
        }
        if self.playing {
            self.pos += n as f64 * cps;
        }
    }
}
