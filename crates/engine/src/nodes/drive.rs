use super::{Ctx, Io, Processor};
use crate::dsp::Smooth;
use crate::model::drive_p;
use std::f32::consts::FRAC_PI_2;

/// Waveshaper into a bit crusher and sample-rate reducer.
pub struct Drive {
    shape: usize,
    drive: Smooth,
    bits: f32,
    rate: u32,
    mix: Smooth,
    level: Smooth,
    /// Sample-and-hold state per channel.
    held: [f32; 2],
    count: u32,
}

impl Drive {
    pub fn new() -> Self {
        Drive {
            shape: 0,
            drive: Smooth::new(0.3),
            bits: 16.0,
            rate: 1,
            mix: Smooth::new(1.0),
            level: Smooth::new(0.7),
            held: [0.0; 2],
            count: 0,
        }
    }
}

impl Processor for Drive {
    fn process(&mut self, ctx: &Ctx, io: Io) {
        let [out_l, out_r] = io.audio_out;
        let steps = 2f32.powf(self.bits - 1.0);
        for f in 0..ctx.frames {
            let g = 60f32.powf(self.drive.tick());
            let (mix, level) = (self.mix.tick(), self.level.tick());
            let sample = self.count == 0;
            self.count = (self.count + 1) % self.rate.max(1);
            for (ch, out) in [(0, &mut *out_l), (1, &mut *out_r)] {
                let x = io.audio_in[ch][f];
                if sample {
                    let mut y = match self.shape {
                        0 => (x * g).tanh(),
                        1 => (x * g).clamp(-1.0, 1.0),
                        _ => (x * g * FRAC_PI_2).sin(),
                    };
                    if self.bits < 16.0 {
                        y = (y * steps).round() / steps;
                    }
                    self.held[ch] = y;
                }
                out[f] = x * (1.0 - mix) + self.held[ch] * level * mix;
            }
        }
    }

    fn set_param(&mut self, idx: usize, v: f32) {
        match idx {
            drive_p::SHAPE => self.shape = v as usize,
            drive_p::DRIVE => self.drive.target = v,
            drive_p::BITS => self.bits = v,
            drive_p::RATE => self.rate = v as u32,
            drive_p::MIX => self.mix.target = v,
            drive_p::LEVEL => self.level.target = v,
            _ => {}
        }
    }

    fn stop(&mut self) {
        self.held = [0.0; 2];
        self.count = 0;
    }
}
