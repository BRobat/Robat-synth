use super::{Ctx, Io, Processor};
use crate::model::delay_p;

const MAX_SECONDS: f32 = 4.0;

/// Tempo-synced stereo feedback delay; time is in 1/16 notes.
pub struct Delay {
    buf: [Vec<f32>; 2],
    write: usize,
    sixteenths: f32,
    feedback: f32,
    mix: f32,
}

impl Delay {
    pub fn new(sr: f32) -> Self {
        let len = (sr * MAX_SECONDS) as usize;
        Delay { buf: [vec![0.0; len], vec![0.0; len]], write: 0, sixteenths: 3.0, feedback: 0.4, mix: 0.35 }
    }
}

impl Processor for Delay {
    fn process(&mut self, ctx: &Ctx, io: Io) {
        let len = self.buf[0].len();
        let sixteenth = 60.0 / ctx.transport.bpm as f32 / 4.0;
        let delay = ((self.sixteenths * sixteenth * ctx.sr) as usize).clamp(1, len - 1);
        let [out_l, out_r] = io.audio_out;
        for f in 0..ctx.frames {
            let read = (self.write + len - delay) % len;
            for (ch, out) in [(0, &mut *out_l), (1, &mut *out_r)] {
                let x = io.audio_in[ch][f];
                let wet = self.buf[ch][read];
                self.buf[ch][self.write] = x + wet * self.feedback;
                out[f] = x + wet * self.mix;
            }
            self.write = (self.write + 1) % len;
        }
    }

    fn set_param(&mut self, idx: usize, v: f32) {
        match idx {
            delay_p::TIME => self.sixteenths = v,
            delay_p::FEEDBACK => self.feedback = v,
            delay_p::MIX => self.mix = v,
            _ => {}
        }
    }

    fn stop(&mut self) {
        for b in self.buf.iter_mut() {
            b.fill(0.0);
        }
    }
}
