//! Node processors. They run on the audio thread: no allocation, no locks.

mod amp;
mod chorus;
mod delay;
mod drive;
mod filter;
mod out;
mod reverb;
mod seq;
mod synth;

use crate::model::{NodeKind, NodeModel, Step, MAX_STEPS};
use pattern::CycleEvent;

pub use seq::pattern_table;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum NoteKind {
    Off,
    On,
}

/// A note event inside the current block. Offs sort before ons at the same
/// frame so a repeated note retriggers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NoteEvent {
    pub frame: u32,
    pub kind: NoteKind,
    pub note: u8,
    pub vel: f32,
}

impl NoteEvent {
    pub fn sort_key(&self) -> (u32, NoteKind) {
        (self.frame, self.kind)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Transport {
    pub playing: bool,
    pub bpm: f64,
    /// Position in cycles (bars) at the start of the block.
    pub pos: f64,
    /// Cycles advanced per sample.
    pub cps: f64,
}

pub struct Ctx {
    pub sr: f32,
    pub frames: usize,
    pub transport: Transport,
}

pub struct Io<'a> {
    pub audio_in: [&'a [f32]; 2],
    pub events_in: &'a [NoteEvent],
    pub audio_out: [&'a mut [f32]; 2],
    pub events_out: &'a mut Vec<NoteEvent>,
}

/// Bulk data swapped into a processor from the UI thread.
pub enum NodeData {
    Steps(Box<[Step; MAX_STEPS]>),
    Table(Vec<Vec<CycleEvent>>),
}

pub trait Processor: Send {
    fn process(&mut self, ctx: &Ctx, io: Io);
    fn set_param(&mut self, idx: usize, value: f32);
    /// Accepts new data; anything replaced goes to `garbage` to be freed off
    /// the audio thread.
    fn set_data(&mut self, data: NodeData, garbage: &mut dyn FnMut(NodeData)) {
        garbage(data);
    }
    /// Data the processor retired by itself; collected after every block.
    fn take_garbage(&mut self) -> Option<NodeData> {
        None
    }
    /// Transport stopped: silence and forget pending notes.
    fn stop(&mut self) {}
    /// For `Out` nodes: the stereo pair this node writes to.
    fn out_pair(&self) -> Option<usize> {
        None
    }
    /// Playhead step, for the UI.
    fn position(&self) -> u32 {
        0
    }
}

pub fn make_processor(node: &NodeModel, sr: f32) -> Box<dyn Processor> {
    let mut p: Box<dyn Processor> = match node.kind {
        NodeKind::Seq => Box::new(seq::Seq::new()),
        NodeKind::Synth => Box::new(synth::Synth::new(sr)),
        NodeKind::Amp => Box::new(amp::Amp::new()),
        NodeKind::Delay => Box::new(delay::Delay::new(sr)),
        NodeKind::Out => Box::new(out::Out::new()),
        NodeKind::Filter => Box::new(filter::Filter::new()),
        NodeKind::Drive => Box::new(drive::Drive::new()),
        NodeKind::Reverb => Box::new(reverb::Reverb::new(sr)),
        NodeKind::Chorus => Box::new(chorus::Chorus::new(sr)),
    };
    for (i, v) in node.params.iter().enumerate() {
        p.set_param(i, *v);
    }
    if let Some(seq) = &node.seq {
        let mut drop_here = |_| {};
        p.set_data(steps_data(&seq.steps), &mut drop_here);
        if let Ok(table) = pattern_table(&seq.pattern) {
            p.set_data(NodeData::Table(table), &mut drop_here);
        }
    }
    p
}

pub fn steps_data(steps: &[Step]) -> NodeData {
    let mut arr = Box::new([Step::default(); MAX_STEPS]);
    for (d, s) in arr.iter_mut().zip(steps) {
        *d = *s;
    }
    NodeData::Steps(arr)
}
