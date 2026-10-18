use super::{Ctx, Io, Processor};
use crate::dsp::Smooth;
use crate::model::out_p;

/// Sends its input to a stereo pair of the audio device.
pub struct Out {
    pair: usize,
    level: Smooth,
}

impl Out {
    pub fn new() -> Self {
        Out { pair: 0, level: Smooth::new(0.8) }
    }
}

impl Processor for Out {
    fn process(&mut self, ctx: &Ctx, io: Io) {
        let [out_l, out_r] = io.audio_out;
        for f in 0..ctx.frames {
            let g = self.level.tick();
            // Soft clip so a hot patch never hard-clips the Logic input.
            out_l[f] = (io.audio_in[0][f] * g).tanh();
            out_r[f] = (io.audio_in[1][f] * g).tanh();
        }
    }

    fn set_param(&mut self, idx: usize, v: f32) {
        match idx {
            out_p::PAIR => self.pair = v as usize,
            out_p::LEVEL => self.level.target = v,
            _ => {}
        }
    }

    fn out_pair(&self) -> Option<usize> {
        Some(self.pair)
    }
}
