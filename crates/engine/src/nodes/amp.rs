use super::{Ctx, Io, Processor};
use crate::dsp::Smooth;
use crate::model::amp_p;
use std::f32::consts::FRAC_PI_4;

pub struct Amp {
    gain: Smooth,
    pan: Smooth,
}

impl Amp {
    pub fn new() -> Self {
        Amp { gain: Smooth::new(1.0), pan: Smooth::new(0.0) }
    }
}

impl Processor for Amp {
    fn process(&mut self, ctx: &Ctx, io: Io) {
        let [in_l, in_r] = io.audio_in;
        let [out_l, out_r] = io.audio_out;
        for f in 0..ctx.frames {
            let g = self.gain.tick();
            // Equal-power balance.
            let angle = (self.pan.tick() + 1.0) * FRAC_PI_4;
            out_l[f] = in_l[f] * g * angle.cos() * std::f32::consts::SQRT_2;
            out_r[f] = in_r[f] * g * angle.sin() * std::f32::consts::SQRT_2;
        }
    }

    fn set_param(&mut self, idx: usize, v: f32) {
        match idx {
            amp_p::GAIN => self.gain.target = v,
            amp_p::PAN => self.pan.target = v,
            _ => {}
        }
    }
}
