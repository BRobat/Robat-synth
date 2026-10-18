use super::{Ctx, Io, NodeData, NoteEvent, NoteKind, Processor};
use crate::model::{seq_p, Step, MAX_STEPS};
use pattern::{CycleEvent, ParseError, Pattern};

const MAX_PENDING: usize = 128;
const PATTERN_VEL: f32 = 0.8;

/// Parses a pattern into the per-cycle table the sequencer plays.
pub fn pattern_table(src: &str) -> Result<Vec<Vec<CycleEvent>>, ParseError> {
    Ok(Pattern::parse(src)?.tabulate())
}

#[derive(Clone, Copy)]
struct PendingOff {
    note: u8,
    /// Absolute cycle position of the note-off.
    at: f64,
}

pub struct Seq {
    pattern_mode: bool,
    rate: f64,
    length: usize,
    gate: f64,
    swing: f64,
    transpose: i32,
    steps: Box<[Step; MAX_STEPS]>,
    table: Vec<Vec<CycleEvent>>,
    /// A new pattern waiting for the next cycle boundary.
    queued: Option<Vec<Vec<CycleEvent>>>,
    retired: Option<Vec<Vec<CycleEvent>>>,
    pending: [PendingOff; MAX_PENDING],
    n_pending: usize,
    step_pos: u32,
}

impl Seq {
    pub fn new() -> Self {
        Seq {
            pattern_mode: false,
            rate: 16.0,
            length: 16,
            gate: 0.5,
            swing: 0.0,
            transpose: 0,
            steps: Box::new([Step::default(); MAX_STEPS]),
            table: Vec::new(),
            queued: None,
            retired: None,
            pending: [PendingOff { note: 0, at: 0.0 }; MAX_PENDING],
            n_pending: 0,
            step_pos: 0,
        }
    }

    fn note_on(&mut self, out: &mut Vec<NoteEvent>, frame: u32, note: i32, vel: f32, off_at: f64) {
        let Ok(note) = u8::try_from(note) else { return };
        if note > 127 || out.len() >= out.capacity() || self.n_pending >= MAX_PENDING {
            return;
        }
        out.push(NoteEvent { frame, kind: NoteKind::On, note, vel });
        self.pending[self.n_pending] = PendingOff { note, at: off_at };
        self.n_pending += 1;
    }

    fn grid(&mut self, start: f64, end: f64, frame_of: impl Fn(f64) -> u32, out: &mut Vec<NoteEvent>) {
        let step_len = 1.0 / self.rate;
        let first = (start * self.rate).floor() as i64 - 1;
        let last = (end * self.rate).floor() as i64;
        for k in first..=last {
            let swing = if k.rem_euclid(2) == 1 { self.swing * step_len } else { 0.0 };
            let onset = k as f64 * step_len + swing;
            if onset < start || onset >= end {
                continue;
            }
            let idx = k.rem_euclid(self.length as i64) as usize;
            self.step_pos = idx as u32;
            let s = self.steps[idx];
            if s.on {
                let off = onset + step_len * self.gate;
                self.note_on(out, frame_of(onset), s.note as i32 + self.transpose, s.vel, off);
            }
        }
    }

    fn pattern(&mut self, start: f64, end: f64, frame_of: impl Fn(f64) -> u32, out: &mut Vec<NoteEvent>) {
        for c in start.floor() as i64..=end.floor() as i64 {
            let cycle_start = c as f64;
            if cycle_start >= start && cycle_start < end {
                if let Some(q) = self.queued.take() {
                    self.retired = Some(std::mem::replace(&mut self.table, q));
                }
            }
            if self.table.is_empty() {
                continue;
            }
            let idx = c.rem_euclid(self.table.len() as i64) as usize;
            for i in 0..self.table[idx].len() {
                let ev = self.table[idx][i];
                let onset = cycle_start + ev.pos;
                if onset >= start && onset < end {
                    let off = onset + ev.dur * self.gate;
                    self.note_on(out, frame_of(onset), ev.note as i32 + self.transpose, PATTERN_VEL, off);
                }
            }
        }
        self.step_pos = ((start.fract()) * 16.0) as u32;
    }
}

impl Processor for Seq {
    fn process(&mut self, ctx: &Ctx, io: Io) {
        let t = ctx.transport;
        if !t.playing {
            return;
        }
        let start = t.pos;
        let end = t.pos + ctx.frames as f64 * t.cps;
        let last = ctx.frames as u32 - 1;
        let frame_of = |pos: f64| (((pos - start) / t.cps).round() as u32).min(last);
        let out = io.events_out;

        // Note-offs due in this block (or overdue after a gate change).
        let mut i = 0;
        while i < self.n_pending {
            let p = self.pending[i];
            if p.at < end {
                if out.len() < out.capacity() {
                    out.push(NoteEvent { frame: frame_of(p.at.max(start)), kind: NoteKind::Off, note: p.note, vel: 0.0 });
                }
                self.n_pending -= 1;
                self.pending[i] = self.pending[self.n_pending];
            } else {
                i += 1;
            }
        }

        if self.pattern_mode {
            self.pattern(start, end, frame_of, out);
        } else {
            self.grid(start, end, frame_of, out);
        }
        out.sort_unstable_by_key(NoteEvent::sort_key);
    }

    fn set_param(&mut self, idx: usize, v: f32) {
        match idx {
            seq_p::MODE => self.pattern_mode = v >= 0.5,
            seq_p::RATE => self.rate = v.max(1.0) as f64,
            seq_p::LENGTH => self.length = (v as usize).clamp(1, MAX_STEPS),
            seq_p::GATE => self.gate = v as f64,
            seq_p::SWING => self.swing = v as f64,
            seq_p::TRANSPOSE => self.transpose = v as i32,
            _ => {}
        }
    }

    fn set_data(&mut self, data: NodeData, garbage: &mut dyn FnMut(NodeData)) {
        match data {
            NodeData::Steps(steps) => garbage(NodeData::Steps(std::mem::replace(&mut self.steps, steps))),
            NodeData::Table(table) => {
                if self.table.is_empty() {
                    garbage(NodeData::Table(std::mem::replace(&mut self.table, table)));
                } else if let Some(old) = self.queued.replace(table) {
                    garbage(NodeData::Table(old));
                }
            }
        }
    }

    fn stop(&mut self) {
        self.n_pending = 0;
        self.step_pos = 0;
        if let Some(q) = self.queued.take() {
            self.retired = Some(std::mem::replace(&mut self.table, q));
        }
    }

    fn take_garbage(&mut self) -> Option<NodeData> {
        self.retired.take().map(NodeData::Table)
    }

    fn position(&self) -> u32 {
        self.step_pos
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nodes::Transport;

    /// Runs the sequencer for `frames`, returning (absolute frame, kind, note).
    fn run(seq: &mut Seq, bpm: f64, sr: f32, frames: usize, block: usize) -> Vec<(usize, NoteKind, u8)> {
        let cps = bpm / 60.0 / 4.0 / sr as f64;
        let silence = vec![0.0; block];
        let (mut l, mut r) = (vec![0.0; block], vec![0.0; block]);
        let mut events = Vec::with_capacity(256);
        let mut got = Vec::new();
        let mut done = 0;
        while done < frames {
            let n = block.min(frames - done);
            events.clear();
            let ctx = Ctx { sr, frames: n, transport: Transport { playing: true, bpm, pos: done as f64 * cps, cps } };
            seq.process(
                &ctx,
                Io {
                    audio_in: [&silence[..n], &silence[..n]],
                    events_in: &[],
                    audio_out: [&mut l[..n], &mut r[..n]],
                    events_out: &mut events,
                },
            );
            got.extend(events.iter().map(|e| (done + e.frame as usize, e.kind, e.note)));
            done += n;
        }
        got
    }

    fn ons(evs: &[(usize, NoteKind, u8)]) -> Vec<(usize, u8)> {
        evs.iter().filter(|e| e.1 == NoteKind::On).map(|e| (e.0, e.2)).collect()
    }

    #[test]
    fn grid_steps_land_on_exact_frames() {
        let mut seq = Seq::new();
        seq.steps[0] = Step { on: true, note: 60, vel: 1.0 };
        seq.steps[4] = Step { on: true, note: 62, vel: 1.0 };
        // 120 bpm, 48 kHz: a 1/16 note is 6000 frames; a bar is 96000.
        let evs = run(&mut seq, 120.0, 48000.0, 96000 * 2, 37);
        assert_eq!(ons(&evs), vec![(0, 60), (24000, 62), (96000, 60), (120000, 62)]);
        // Gate 0.5 releases half a step later.
        let offs: Vec<usize> = evs.iter().filter(|e| e.1 == NoteKind::Off).map(|e| e.0).collect();
        assert_eq!(&offs[..2], &[3000, 27000]);
    }

    #[test]
    fn grid_length_makes_polymeter() {
        let mut seq = Seq::new();
        seq.set_param(seq_p::LENGTH, 3.0);
        seq.steps[0] = Step { on: true, note: 60, vel: 1.0 };
        let evs = run(&mut seq, 120.0, 48000.0, 6000 * 7, 64);
        assert_eq!(ons(&evs), vec![(0, 60), (18000, 60), (36000, 60)]);
    }

    #[test]
    fn pattern_mode_and_swap_on_cycle_boundary() {
        let mut seq = Seq::new();
        seq.set_param(seq_p::MODE, 1.0);
        let mut sink = |_| {};
        seq.set_data(NodeData::Table(pattern_table("c d").unwrap()), &mut sink);
        // Mid-bar change: takes effect on the next bar.
        let mut evs = run(&mut seq, 120.0, 48000.0, 60000, 64);
        seq.set_data(NodeData::Table(pattern_table("e").unwrap()), &mut sink);
        let cps = 120.0 / 60.0 / 4.0 / 48000.0;
        let ctx_from = |start: usize, n: usize| Ctx {
            sr: 48000.0,
            frames: n,
            transport: Transport { playing: true, bpm: 120.0, pos: start as f64 * cps, cps },
        };
        let silence = vec![0.0; 64];
        let (mut l, mut r) = (vec![0.0; 64], vec![0.0; 64]);
        let mut events = Vec::with_capacity(256);
        let mut t = 60000;
        while t < 192000 {
            events.clear();
            seq.process(
                &ctx_from(t, 64),
                Io { audio_in: [&silence, &silence], events_in: &[], audio_out: [&mut l, &mut r], events_out: &mut events },
            );
            evs.extend(events.iter().map(|e| (t + e.frame as usize, e.kind, e.note)));
            t += 64;
        }
        assert_eq!(ons(&evs), vec![(0, 60), (48000, 62), (96000, 64), (192000, 64)]);
        assert!(seq.take_garbage().is_some());
    }
}
