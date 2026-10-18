//! Serializable project model shared by the UI and the engine.

use serde::{Deserialize, Serialize};

pub const MAX_NODES: usize = 256;
pub const MAX_STEPS: usize = 64;
pub const OUT_PAIRS: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NodeKind {
    Seq,
    Synth,
    Amp,
    Delay,
    Out,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Port {
    None,
    Events,
    Audio,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Curve {
    Lin,
    /// Exponential mapping, for frequencies and times.
    Log,
    /// Whole numbers; `labels` (if any) name each value from `min`.
    Int,
}

#[derive(Clone, Copy, Debug)]
pub struct ParamSpec {
    pub name: &'static str,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub curve: Curve,
    pub unit: &'static str,
    pub labels: &'static [&'static str],
}

const fn p(name: &'static str, min: f32, max: f32, default: f32, curve: Curve, unit: &'static str) -> ParamSpec {
    ParamSpec { name, min, max, default, curve, unit, labels: &[] }
}

const fn labeled(name: &'static str, default: f32, labels: &'static [&'static str]) -> ParamSpec {
    ParamSpec { name, min: 0.0, max: (labels.len() - 1) as f32, default, curve: Curve::Int, unit: "", labels }
}

/// Parameter indices per node kind.
pub mod seq_p {
    pub const MODE: usize = 0;
    pub const RATE: usize = 1;
    pub const LENGTH: usize = 2;
    pub const GATE: usize = 3;
    pub const SWING: usize = 4;
    pub const TRANSPOSE: usize = 5;
}
pub mod synth_p {
    pub const WAVE: usize = 0;
    pub const OSC2: usize = 1;
    pub const DETUNE: usize = 2;
    pub const OCTAVE: usize = 3;
    pub const ATTACK: usize = 4;
    pub const DECAY: usize = 5;
    pub const SUSTAIN: usize = 6;
    pub const RELEASE: usize = 7;
    pub const CUTOFF: usize = 8;
    pub const RES: usize = 9;
    pub const ENV_AMT: usize = 10;
    pub const LEVEL: usize = 11;
}
pub mod amp_p {
    pub const GAIN: usize = 0;
    pub const PAN: usize = 1;
}
pub mod delay_p {
    pub const TIME: usize = 0;
    pub const FEEDBACK: usize = 1;
    pub const MIX: usize = 2;
}
pub mod out_p {
    pub const PAIR: usize = 0;
    pub const LEVEL: usize = 1;
}

pub const WAVES: &[&str] = &["sine", "saw", "square", "tri", "noise"];
pub const PAIRS: &[&str] = &["1-2", "3-4", "5-6", "7-8", "9-10", "11-12", "13-14", "15-16"];

static SEQ_PARAMS: [ParamSpec; 6] = [
    labeled("mode", 0.0, &["grid", "pattern"]),
    p("rate", 1.0, 32.0, 16.0, Curve::Int, "/bar"),
    p("length", 1.0, MAX_STEPS as f32, 16.0, Curve::Int, "steps"),
    p("gate", 0.05, 1.0, 0.5, Curve::Lin, ""),
    p("swing", 0.0, 0.5, 0.0, Curve::Lin, ""),
    p("transpose", -24.0, 24.0, 0.0, Curve::Int, "st"),
];

static SYNTH_PARAMS: [ParamSpec; 12] = [
    labeled("wave", 1.0, WAVES),
    p("osc2", 0.0, 1.0, 0.0, Curve::Lin, ""),
    p("detune", 0.0, 50.0, 7.0, Curve::Lin, "ct"),
    p("octave", -3.0, 3.0, 0.0, Curve::Int, ""),
    p("attack", 0.001, 4.0, 0.005, Curve::Log, "s"),
    p("decay", 0.005, 4.0, 0.25, Curve::Log, "s"),
    p("sustain", 0.0, 1.0, 0.6, Curve::Lin, ""),
    p("release", 0.005, 8.0, 0.3, Curve::Log, "s"),
    p("cutoff", 20.0, 18000.0, 3000.0, Curve::Log, "Hz"),
    p("res", 0.0, 0.95, 0.2, Curve::Lin, ""),
    p("env amt", 0.0, 1.0, 0.3, Curve::Lin, ""),
    p("level", 0.0, 1.0, 0.5, Curve::Lin, ""),
];

static AMP_PARAMS: [ParamSpec; 2] = [p("gain", 0.0, 2.0, 1.0, Curve::Lin, ""), p("pan", -1.0, 1.0, 0.0, Curve::Lin, "")];

static DELAY_PARAMS: [ParamSpec; 3] =
    [p("time", 1.0, 16.0, 3.0, Curve::Int, "/16"), p("feedback", 0.0, 0.95, 0.4, Curve::Lin, ""), p("mix", 0.0, 1.0, 0.35, Curve::Lin, "")];

static OUT_PARAMS: [ParamSpec; 2] = [labeled("channels", 0.0, PAIRS), p("level", 0.0, 1.0, 0.8, Curve::Lin, "")];

impl NodeKind {
    pub const ALL: [NodeKind; 5] = [NodeKind::Seq, NodeKind::Synth, NodeKind::Amp, NodeKind::Delay, NodeKind::Out];

    pub fn name(self) -> &'static str {
        match self {
            NodeKind::Seq => "seq",
            NodeKind::Synth => "synth",
            NodeKind::Amp => "amp",
            NodeKind::Delay => "delay",
            NodeKind::Out => "out",
        }
    }

    pub fn input(self) -> Port {
        match self {
            NodeKind::Seq => Port::None,
            NodeKind::Synth => Port::Events,
            NodeKind::Amp | NodeKind::Delay | NodeKind::Out => Port::Audio,
        }
    }

    pub fn output(self) -> Port {
        match self {
            NodeKind::Seq => Port::Events,
            NodeKind::Synth | NodeKind::Amp | NodeKind::Delay => Port::Audio,
            NodeKind::Out => Port::None,
        }
    }

    pub fn params(self) -> &'static [ParamSpec] {
        match self {
            NodeKind::Seq => &SEQ_PARAMS,
            NodeKind::Synth => &SYNTH_PARAMS,
            NodeKind::Amp => &AMP_PARAMS,
            NodeKind::Delay => &DELAY_PARAMS,
            NodeKind::Out => &OUT_PARAMS,
        }
    }

    pub fn can_feed(self, to: NodeKind) -> bool {
        self.output() != Port::None && self.output() == to.input()
    }
}

impl ParamSpec {
    pub fn to_norm(&self, v: f32) -> f32 {
        match self.curve {
            Curve::Log => (v / self.min).ln() / (self.max / self.min).ln(),
            _ => (v - self.min) / (self.max - self.min),
        }
        .clamp(0.0, 1.0)
    }

    pub fn from_norm(&self, n: f32) -> f32 {
        let n = n.clamp(0.0, 1.0);
        match self.curve {
            Curve::Log => self.min * (self.max / self.min).powf(n),
            Curve::Int => (self.min + n * (self.max - self.min)).round(),
            Curve::Lin => self.min + n * (self.max - self.min),
        }
    }

    /// Moves a value by `steps` fine increments (coarse = more steps).
    pub fn nudge(&self, v: f32, steps: f32) -> f32 {
        match self.curve {
            Curve::Int => (v + steps).clamp(self.min, self.max),
            _ => self.from_norm(self.to_norm(v) + steps * 0.01),
        }
    }

    pub fn clamp(&self, v: f32) -> f32 {
        let v = v.clamp(self.min, self.max);
        if self.curve == Curve::Int {
            v.round()
        } else {
            v
        }
    }

    pub fn format(&self, v: f32) -> String {
        if let Some(label) = self.labels.get((v - self.min).round() as usize) {
            return (*label).to_string();
        }
        let num = match self.curve {
            Curve::Int => format!("{}", v as i32),
            _ if v.abs() >= 100.0 => format!("{v:.0}"),
            _ if v.abs() >= 10.0 => format!("{v:.1}"),
            _ => format!("{v:.3}"),
        };
        if self.unit.is_empty() {
            num
        } else {
            format!("{num} {}", self.unit)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Step {
    pub on: bool,
    pub note: u8,
    pub vel: f32,
}

impl Default for Step {
    fn default() -> Self {
        Step { on: false, note: 48, vel: 0.8 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SeqData {
    pub steps: Vec<Step>,
    pub pattern: String,
}

impl Default for SeqData {
    fn default() -> Self {
        SeqData { steps: vec![Step::default(); MAX_STEPS], pattern: String::new() }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NodeModel {
    pub id: u32,
    pub kind: NodeKind,
    /// Position on the canvas grid.
    pub x: i32,
    pub y: i32,
    pub params: Vec<f32>,
    #[serde(default)]
    pub muted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seq: Option<SeqData>,
}

impl NodeModel {
    pub fn new(id: u32, kind: NodeKind, x: i32, y: i32) -> Self {
        NodeModel {
            id,
            kind,
            x,
            y,
            params: kind.params().iter().map(|p| p.default).collect(),
            muted: false,
            seq: (kind == NodeKind::Seq).then(SeqData::default),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub bpm: f32,
    pub nodes: Vec<NodeModel>,
    /// Connections `(from, to)` by node id.
    pub edges: Vec<(u32, u32)>,
    #[serde(default)]
    pub device: Option<String>,
}

impl Default for Project {
    fn default() -> Self {
        Project { bpm: 120.0, nodes: Vec::new(), edges: Vec::new(), device: None }
    }
}

impl Project {
    pub fn node(&self, id: u32) -> Option<&NodeModel> {
        self.nodes.iter().find(|n| n.id == id)
    }

    pub fn node_mut(&mut self, id: u32) -> Option<&mut NodeModel> {
        self.nodes.iter_mut().find(|n| n.id == id)
    }

    pub fn free_id(&self) -> Option<u32> {
        (0..MAX_NODES as u32).find(|id| self.node(*id).is_none())
    }

    /// Whether adding `from -> to` would create a cycle.
    pub fn would_cycle(&self, from: u32, to: u32) -> bool {
        let mut stack = vec![to];
        let mut seen = vec![false; MAX_NODES];
        while let Some(n) = stack.pop() {
            if n == from {
                return true;
            }
            if std::mem::replace(&mut seen[n as usize], true) {
                continue;
            }
            stack.extend(self.edges.iter().filter(|(a, _)| *a == n).map(|(_, b)| *b));
        }
        false
    }

    /// Nodes in dependency order (sources first).
    pub fn topo_order(&self) -> Vec<u32> {
        let mut indeg = vec![0usize; MAX_NODES];
        for (_, b) in &self.edges {
            indeg[*b as usize] += 1;
        }
        let mut ready: Vec<u32> = self.nodes.iter().map(|n| n.id).filter(|id| indeg[*id as usize] == 0).collect();
        ready.reverse();
        let mut order = Vec::with_capacity(self.nodes.len());
        while let Some(n) = ready.pop() {
            order.push(n);
            for (_, b) in self.edges.iter().filter(|(a, _)| *a == n) {
                indeg[*b as usize] -= 1;
                if indeg[*b as usize] == 0 {
                    ready.push(*b);
                }
            }
        }
        order
    }

    /// A small two-voice starter: a pattern bass on 1-2 and a grid lead
    /// through a delay on 3-4.
    pub fn demo() -> Self {
        let mut pr = Project { bpm: 118.0, ..Default::default() };

        let mut bass_seq = NodeModel::new(0, NodeKind::Seq, 0, 0);
        bass_seq.params[seq_p::MODE] = 1.0;
        bass_seq.params[seq_p::GATE] = 0.7;
        bass_seq.seq.as_mut().unwrap().pattern = "c1 ~ c1 [~ c2] <eb1 g0> ~ c1 [bb0 c1]".into();
        let mut bass = NodeModel::new(1, NodeKind::Synth, 1, 0);
        bass.params[synth_p::CUTOFF] = 600.0;
        bass.params[synth_p::ENV_AMT] = 0.5;
        bass.params[synth_p::RES] = 0.45;
        bass.params[synth_p::DECAY] = 0.18;
        bass.params[synth_p::SUSTAIN] = 0.3;
        let mut bass_out = NodeModel::new(2, NodeKind::Out, 2, 0);
        bass_out.params[out_p::PAIR] = 0.0;

        let mut lead_seq = NodeModel::new(3, NodeKind::Seq, 0, 1);
        let notes = [(0, 60), (3, 63), (6, 67), (8, 70), (10, 67), (13, 72)];
        let steps = &mut lead_seq.seq.as_mut().unwrap().steps;
        for (i, n) in notes {
            steps[i] = Step { on: true, note: n, vel: 0.8 };
        }
        lead_seq.params[seq_p::GATE] = 0.3;
        let mut lead = NodeModel::new(4, NodeKind::Synth, 1, 1);
        lead.params[synth_p::WAVE] = 2.0;
        lead.params[synth_p::OSC2] = 0.5;
        lead.params[synth_p::CUTOFF] = 2200.0;
        lead.params[synth_p::LEVEL] = 0.3;
        let delay = NodeModel::new(5, NodeKind::Delay, 2, 1);
        let mut lead_out = NodeModel::new(6, NodeKind::Out, 3, 1);
        lead_out.params[out_p::PAIR] = 1.0;

        pr.nodes = vec![bass_seq, bass, bass_out, lead_seq, lead, delay, lead_out];
        pr.edges = vec![(0, 1), (1, 2), (3, 4), (4, 5), (5, 6)];
        pr
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topo_and_cycles() {
        let pr = Project::demo();
        let order = pr.topo_order();
        assert_eq!(order.len(), pr.nodes.len());
        let pos = |id| order.iter().position(|x| *x == id).unwrap();
        for (a, b) in &pr.edges {
            assert!(pos(*a) < pos(*b));
        }
        assert!(pr.would_cycle(5, 4));
        assert!(!pr.would_cycle(1, 5));
    }

    #[test]
    fn param_mapping_roundtrips() {
        let cutoff = &NodeKind::Synth.params()[synth_p::CUTOFF];
        for v in [20.0, 440.0, 18000.0] {
            assert!((cutoff.from_norm(cutoff.to_norm(v)) - v).abs() / v < 1e-4);
        }
        let len = &NodeKind::Seq.params()[seq_p::LENGTH];
        assert_eq!(len.nudge(16.0, 1.0), 17.0);
        assert_eq!(len.nudge(64.0, 1.0), 64.0);
        assert_eq!(NodeKind::Synth.params()[synth_p::WAVE].format(2.0), "square");
    }

    #[test]
    fn port_rules() {
        assert!(NodeKind::Seq.can_feed(NodeKind::Synth));
        assert!(!NodeKind::Seq.can_feed(NodeKind::Out));
        assert!(NodeKind::Synth.can_feed(NodeKind::Delay));
        assert!(!NodeKind::Out.can_feed(NodeKind::Amp));
    }
}
