use super::{Ctx, Io, Processor};
use crate::dsp::{Smooth, Svf};
use crate::model::filter_p;

/// Stereo multimode state-variable filter with optional input drive.
pub struct Filter {
    svf: [Svf; 2],
    mode: usize,
    cutoff: Smooth,
    res: f32,
    drive: Smooth,
}

impl Filter {
    pub fn new() -> Self {
        Filter { svf: [Svf::default(); 2], mode: 0, cutoff: Smooth::new(1200.0), res: 0.3, drive: Smooth::new(0.0) }
    }
}

impl Processor for Filter {
    fn process(&mut self, ctx: &Ctx, io: Io) {
        let [out_l, out_r] = io.audio_out;
        for f in 0..ctx.frames {
            let cutoff = self.cutoff.tick();
            let g = 1.0 + 9.0 * self.drive.tick();
            for (ch, out) in [(0, &mut *out_l), (1, &mut *out_r)] {
                let mut x = io.audio_in[ch][f];
                if g > 1.001 {
                    x = (x * g).tanh() / g.sqrt();
                }
                let (lp, bp, hp) = self.svf[ch].tick(x, cutoff, self.res, ctx.sr);
                out[f] = match self.mode {
                    0 => lp,
                    1 => bp,
                    2 => hp,
                    _ => lp + hp,
                };
            }
        }
    }

    fn set_param(&mut self, idx: usize, v: f32) {
        match idx {
            filter_p::MODE => self.mode = v as usize,
            filter_p::CUTOFF => self.cutoff.target = v,
            filter_p::RES => self.res = v,
            filter_p::DRIVE => self.drive.target = v,
            _ => {}
        }
    }

    fn stop(&mut self) {
        for s in self.svf.iter_mut() {
            s.reset();
        }
    }
}
