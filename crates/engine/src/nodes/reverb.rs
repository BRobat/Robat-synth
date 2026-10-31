use super::{Ctx, Io, Processor};
use crate::dsp::{Allpass, Comb, Smooth};
use crate::model::reverb_p;

// Freeverb tunings at 44.1 kHz.
const COMBS: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const ALLPASSES: [usize; 4] = [556, 441, 341, 225];
const SPREAD: usize = 23;

/// Freeverb-style stereo reverb.
pub struct Reverb {
    combs: [Vec<Comb>; 2],
    allpasses: [Vec<Allpass>; 2],
    size: f32,
    damp: f32,
    width: Smooth,
    mix: Smooth,
}

impl Reverb {
    pub fn new(sr: f32) -> Self {
        let scale = |n: usize| (n as f32 * sr / 44100.0) as usize;
        let side = |spread: usize| -> (Vec<Comb>, Vec<Allpass>) {
            (
                COMBS.iter().map(|n| Comb::new(scale(n + spread))).collect(),
                ALLPASSES.iter().map(|n| Allpass::new(scale(n + spread))).collect(),
            )
        };
        let ((cl, al), (cr, ar)) = (side(0), side(SPREAD));
        Reverb { combs: [cl, cr], allpasses: [al, ar], size: 0.7, damp: 0.4, width: Smooth::new(1.0), mix: Smooth::new(0.3) }
    }
}

impl Processor for Reverb {
    fn process(&mut self, ctx: &Ctx, io: Io) {
        let [out_l, out_r] = io.audio_out;
        let feedback = 0.7 + 0.28 * self.size;
        let damp = 0.4 * self.damp;
        for f in 0..ctx.frames {
            let (x_l, x_r) = (io.audio_in[0][f], io.audio_in[1][f]);
            let input = (x_l + x_r) * 0.015;
            let mut wet = [0.0; 2];
            for (ch, w) in wet.iter_mut().enumerate() {
                let mut acc: f32 = self.combs[ch].iter_mut().map(|c| c.tick(input, feedback, damp)).sum();
                for a in self.allpasses[ch].iter_mut() {
                    acc = a.tick(acc);
                }
                *w = acc * 3.0;
            }
            let width = self.width.tick();
            let (w1, w2) = (0.5 + 0.5 * width, 0.5 - 0.5 * width);
            let mix = self.mix.tick();
            out_l[f] = x_l * (1.0 - mix) + (wet[0] * w1 + wet[1] * w2) * mix;
            out_r[f] = x_r * (1.0 - mix) + (wet[1] * w1 + wet[0] * w2) * mix;
        }
    }

    fn set_param(&mut self, idx: usize, v: f32) {
        match idx {
            reverb_p::SIZE => self.size = v,
            reverb_p::DAMP => self.damp = v,
            reverb_p::WIDTH => self.width.target = v,
            reverb_p::MIX => self.mix.target = v,
            _ => {}
        }
    }

    fn stop(&mut self) {
        self.combs.iter_mut().flatten().for_each(Comb::reset);
        self.allpasses.iter_mut().flatten().for_each(Allpass::reset);
    }
}
