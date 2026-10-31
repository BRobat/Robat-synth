use super::{Ctx, Io, Processor};
use crate::dsp::Smooth;
use crate::model::chorus_p;
use std::f32::consts::TAU;

const MAX_MS: f32 = 64.0;

/// Stereo chorus / flanger: a short delay swept by a sine LFO, the right
/// channel a quarter cycle ahead.
pub struct Chorus {
    buf: [Vec<f32>; 2],
    write: usize,
    phase: f32,
    rate: f32,
    depth: f32,
    time: Smooth,
    feedback: f32,
    mix: Smooth,
}

impl Chorus {
    pub fn new(sr: f32) -> Self {
        let len = (sr * MAX_MS / 1000.0) as usize;
        Chorus {
            buf: [vec![0.0; len], vec![0.0; len]],
            write: 0,
            phase: 0.0,
            rate: 0.6,
            depth: 0.5,
            time: Smooth::new(12.0),
            feedback: 0.0,
            mix: Smooth::new(0.5),
        }
    }
}

impl Processor for Chorus {
    fn process(&mut self, ctx: &Ctx, io: Io) {
        let len = self.buf[0].len();
        let [out_l, out_r] = io.audio_out;
        for f in 0..ctx.frames {
            let time = self.time.tick();
            let mix = self.mix.tick();
            for (ch, out) in [(0, &mut *out_l), (1, &mut *out_r)] {
                let lfo = (TAU * (self.phase + 0.25 * ch as f32)).sin();
                let delay = (time * (1.0 + 0.9 * self.depth * lfo) * ctx.sr / 1000.0).clamp(1.0, (len - 2) as f32);
                let read = (self.write as f32 - delay + len as f32) % len as f32;
                let (i, frac) = (read as usize, read.fract());
                let b = &mut self.buf[ch];
                let wet = b[i] + (b[(i + 1) % len] - b[i]) * frac;
                let x = io.audio_in[ch][f];
                b[self.write] = x + wet * self.feedback;
                out[f] = x * (1.0 - mix) + wet * mix;
            }
            self.write = (self.write + 1) % len;
            self.phase = (self.phase + self.rate / ctx.sr).fract();
        }
    }

    fn set_param(&mut self, idx: usize, v: f32) {
        match idx {
            chorus_p::RATE => self.rate = v,
            chorus_p::DEPTH => self.depth = v,
            chorus_p::TIME => self.time.target = v,
            chorus_p::FEEDBACK => self.feedback = v,
            chorus_p::MIX => self.mix.target = v,
            _ => {}
        }
    }

    fn stop(&mut self) {
        for b in self.buf.iter_mut() {
            b.fill(0.0);
        }
    }
}
