//! Small DSP building blocks.

use std::f32::consts::PI;

pub fn midi_to_hz(note: f32) -> f32 {
    440.0 * 2f32.powf((note - 69.0) / 12.0)
}

/// PolyBLEP residual for band-limiting discontinuities.
#[inline]
fn blep(t: f32, dt: f32) -> f32 {
    if t < dt {
        let t = t / dt;
        t + t - t * t - 1.0
    } else if t > 1.0 - dt {
        let t = (t - 1.0) / dt;
        t * t + t + t + 1.0
    } else {
        0.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wave {
    Sine,
    Saw,
    Square,
    Tri,
    Noise,
}

impl Wave {
    pub fn from_index(i: f32) -> Wave {
        match i.round() as i32 {
            0 => Wave::Sine,
            1 => Wave::Saw,
            2 => Wave::Square,
            3 => Wave::Tri,
            _ => Wave::Noise,
        }
    }
}

/// One oscillator sample at `phase` in [0, 1) with increment `dt`.
#[inline]
pub fn osc(wave: Wave, phase: f32, dt: f32, noise: &mut u32) -> f32 {
    match wave {
        Wave::Sine => (2.0 * PI * phase).sin(),
        Wave::Saw => 2.0 * phase - 1.0 - blep(phase, dt),
        Wave::Square => {
            let naive = if phase < 0.5 { 1.0 } else { -1.0 };
            naive + blep(phase, dt) - blep((phase + 0.5).fract(), dt)
        }
        Wave::Tri => 4.0 * (phase - 0.5).abs() - 1.0,
        Wave::Noise => {
            *noise ^= *noise << 13;
            *noise ^= *noise >> 17;
            *noise ^= *noise << 5;
            (*noise as f32 / u32::MAX as f32) * 2.0 - 1.0
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Idle,
    Attack,
    Decay,
    Release,
}

/// Linear attack, exponential decay and release.
#[derive(Clone, Copy, Debug)]
pub struct Adsr {
    stage: Stage,
    pub level: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct AdsrRates {
    attack_inc: f32,
    decay_coef: f32,
    sustain: f32,
    release_coef: f32,
}

/// One-pole coefficient that covers ~99% of the distance in `secs`.
fn coef(secs: f32, sr: f32) -> f32 {
    (-4.6 / (secs.max(1e-4) * sr)).exp()
}

impl AdsrRates {
    pub fn new(attack: f32, decay: f32, sustain: f32, release: f32, sr: f32) -> Self {
        AdsrRates { attack_inc: 1.0 / (attack.max(1e-4) * sr), decay_coef: coef(decay, sr), sustain, release_coef: coef(release, sr) }
    }
}

impl Default for Adsr {
    fn default() -> Self {
        Adsr { stage: Stage::Idle, level: 0.0 }
    }
}

impl Adsr {
    pub fn gate_on(&mut self) {
        self.stage = Stage::Attack;
    }

    pub fn gate_off(&mut self) {
        if self.stage != Stage::Idle {
            self.stage = Stage::Release;
        }
    }

    pub fn is_idle(&self) -> bool {
        self.stage == Stage::Idle
    }

    pub fn is_released(&self) -> bool {
        matches!(self.stage, Stage::Release | Stage::Idle)
    }

    #[inline]
    pub fn next(&mut self, r: &AdsrRates) -> f32 {
        match self.stage {
            Stage::Idle => {}
            Stage::Attack => {
                self.level += r.attack_inc;
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.stage = Stage::Decay;
                }
            }
            Stage::Decay => self.level = r.sustain + (self.level - r.sustain) * r.decay_coef,
            Stage::Release => {
                self.level *= r.release_coef;
                if self.level < 1e-4 {
                    self.level = 0.0;
                    self.stage = Stage::Idle;
                }
            }
        }
        self.level
    }
}

/// Topology-preserving state variable filter (Cytomic), low-pass output.
#[derive(Clone, Copy, Debug, Default)]
pub struct Svf {
    ic1: f32,
    ic2: f32,
}

impl Svf {
    #[inline]
    pub fn lowpass(&mut self, x: f32, cutoff: f32, res: f32, sr: f32) -> f32 {
        let g = (PI * (cutoff / sr).min(0.49)).tan();
        let k = 2.0 - 2.0 * res;
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;
        let v3 = x - self.ic2;
        let v1 = a1 * self.ic1 + a2 * v3;
        let v2 = self.ic2 + a2 * self.ic1 + a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        v2
    }

    pub fn reset(&mut self) {
        *self = Svf::default();
    }
}

/// One-pole parameter smoother.
#[derive(Clone, Copy, Debug)]
pub struct Smooth {
    pub value: f32,
    pub target: f32,
}

impl Smooth {
    pub fn new(v: f32) -> Self {
        Smooth { value: v, target: v }
    }

    #[inline]
    pub fn tick(&mut self) -> f32 {
        self.value += (self.target - self.value) * 0.002;
        self.value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oscillator_frequency_by_zero_crossings() {
        let sr = 48000.0;
        for wave in [Wave::Sine, Wave::Saw, Wave::Square, Wave::Tri] {
            let hz = 441.0;
            let dt = hz / sr;
            let mut phase = 0.1;
            let mut n = 0u32;
            let mut prev = osc(wave, phase, dt, &mut n);
            let mut rising = 0;
            for _ in 0..sr as usize {
                phase = (phase + dt).fract();
                let s = osc(wave, phase, dt, &mut n);
                if prev < 0.0 && s >= 0.0 {
                    rising += 1;
                }
                prev = s;
            }
            assert!((rising as f32 - hz).abs() <= 1.0, "{wave:?}: {rising}");
        }
    }

    #[test]
    fn envelope_shape() {
        let sr = 1000.0;
        let r = AdsrRates::new(0.01, 0.1, 0.5, 0.1, sr);
        let mut e = Adsr::default();
        e.gate_on();
        let peak = (0..10).map(|_| e.next(&r)).fold(0.0, f32::max);
        assert!((peak - 1.0).abs() < 1e-3);
        for _ in 0..500 {
            e.next(&r);
        }
        assert!((e.level - 0.5).abs() < 1e-2);
        e.gate_off();
        for _ in 0..500 {
            e.next(&r);
        }
        assert!(e.is_idle());
    }

    #[test]
    fn lowpass_attenuates_highs() {
        let sr = 48000.0;
        let rms = |hz: f32| {
            let mut f = Svf::default();
            let mut acc = 0.0;
            for i in 0..48000 {
                let x = (2.0 * PI * hz * i as f32 / sr).sin();
                let y = f.lowpass(x, 500.0, 0.0, sr);
                if i > 4800 {
                    acc += y * y;
                }
            }
            (acc / 43200.0).sqrt()
        };
        assert!(rms(100.0) > 0.6);
        assert!(rms(8000.0) < 0.01);
    }
}
