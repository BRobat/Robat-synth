use super::{Ctx, Io, NoteKind, Processor};
use crate::dsp::{midi_to_hz, osc, Adsr, AdsrRates, Smooth, Svf, Wave};
use crate::model::synth_p;

const VOICES: usize = 8;

#[derive(Clone, Copy)]
struct Voice {
    note: u8,
    vel: f32,
    phase1: f32,
    phase2: f32,
    env: Adsr,
    filter: Svf,
    noise: u32,
    /// Monotonic start stamp, for stealing the oldest voice.
    age: u64,
}

pub struct Synth {
    sr: f32,
    voices: [Voice; VOICES],
    counter: u64,
    wave: Wave,
    osc2: f32,
    detune: f32,
    octave: f32,
    attack: f32,
    decay: f32,
    sustain: f32,
    release: f32,
    rates: AdsrRates,
    cutoff: Smooth,
    res: f32,
    env_amt: f32,
    level: Smooth,
}

impl Synth {
    pub fn new(sr: f32) -> Self {
        let voice = Voice {
            note: 0,
            vel: 0.0,
            phase1: 0.0,
            phase2: 0.0,
            env: Adsr::default(),
            filter: Svf::default(),
            noise: 1,
            age: 0,
        };
        let mut voices = [voice; VOICES];
        for (i, v) in voices.iter_mut().enumerate() {
            v.noise = 0x9E37_79B9 ^ (i as u32 + 1).wrapping_mul(0x85EB_CA6B);
        }
        Synth {
            sr,
            voices,
            counter: 0,
            wave: Wave::Saw,
            osc2: 0.0,
            detune: 7.0,
            octave: 0.0,
            attack: 0.005,
            decay: 0.25,
            sustain: 0.6,
            release: 0.3,
            rates: AdsrRates::new(0.005, 0.25, 0.6, 0.3, sr),
            cutoff: Smooth::new(3000.0),
            res: 0.2,
            env_amt: 0.3,
            level: Smooth::new(0.5),
        }
    }

    fn update_rates(&mut self) {
        self.rates = AdsrRates::new(self.attack, self.decay, self.sustain, self.release, self.sr);
    }

    fn note_on(&mut self, note: u8, vel: f32) {
        self.counter += 1;
        // Prefer a voice already playing this note, then an idle one, then the
        // oldest released, then the oldest.
        let idx = (0..VOICES)
            .find(|&i| self.voices[i].note == note && !self.voices[i].env.is_idle())
            .or_else(|| (0..VOICES).find(|&i| self.voices[i].env.is_idle()))
            .or_else(|| (0..VOICES).filter(|&i| self.voices[i].env.is_released()).min_by_key(|&i| self.voices[i].age))
            .unwrap_or_else(|| (0..VOICES).min_by_key(|&i| self.voices[i].age).unwrap());
        let v = &mut self.voices[idx];
        if v.env.is_idle() {
            v.phase1 = 0.0;
            v.phase2 = 0.25;
            v.filter.reset();
        }
        v.note = note;
        v.vel = vel;
        v.age = self.counter;
        v.env.gate_on();
    }

    fn note_off(&mut self, note: u8) {
        for v in self.voices.iter_mut().filter(|v| v.note == note) {
            v.env.gate_off();
        }
    }
}

impl Processor for Synth {
    fn process(&mut self, ctx: &Ctx, io: Io) {
        let [out_l, out_r] = io.audio_out;
        let mut events = io.events_in.iter().peekable();
        let sr = self.sr;
        let detune = 2f32.powf(self.detune / 1200.0);
        let nyquist = sr * 0.45;
        for f in 0..ctx.frames {
            while let Some(e) = events.next_if(|e| e.frame as usize <= f) {
                match e.kind {
                    NoteKind::On => self.note_on(e.note, e.vel),
                    NoteKind::Off => self.note_off(e.note),
                }
            }
            let cutoff = self.cutoff.tick();
            let level = self.level.tick();
            let mut sum = 0.0;
            for v in self.voices.iter_mut() {
                if v.env.is_idle() {
                    continue;
                }
                let env = v.env.next(&self.rates);
                let hz = midi_to_hz(v.note as f32 + self.octave * 12.0);
                let dt1 = (hz / sr).min(0.5);
                let dt2 = (hz * detune / sr).min(0.5);
                let mut s = osc(self.wave, v.phase1, dt1, &mut v.noise);
                if self.osc2 > 0.0 {
                    let s2 = osc(self.wave, v.phase2, dt2, &mut v.noise);
                    s = s * (1.0 - 0.5 * self.osc2) + s2 * 0.5 * self.osc2;
                }
                v.phase1 = (v.phase1 + dt1).fract();
                v.phase2 = (v.phase2 + dt2).fract();
                let fc = (cutoff * 2f32.powf(env * self.env_amt * 6.0)).min(nyquist);
                let y = v.filter.lowpass(s, fc, self.res, sr);
                sum += y * env * v.vel;
            }
            let y = sum * level;
            out_l[f] = y;
            out_r[f] = y;
        }
    }

    fn set_param(&mut self, idx: usize, v: f32) {
        match idx {
            synth_p::WAVE => self.wave = Wave::from_index(v),
            synth_p::OSC2 => self.osc2 = v,
            synth_p::DETUNE => self.detune = v,
            synth_p::OCTAVE => self.octave = v,
            synth_p::ATTACK => self.attack = v,
            synth_p::DECAY => self.decay = v,
            synth_p::SUSTAIN => self.sustain = v,
            synth_p::RELEASE => self.release = v,
            synth_p::CUTOFF => self.cutoff.target = v,
            synth_p::RES => self.res = v,
            synth_p::ENV_AMT => self.env_amt = v,
            synth_p::LEVEL => self.level.target = v,
            _ => {}
        }
        if matches!(idx, synth_p::ATTACK | synth_p::DECAY | synth_p::SUSTAIN | synth_p::RELEASE) {
            self.update_rates();
        }
    }

    fn stop(&mut self) {
        for v in self.voices.iter_mut() {
            v.env.gate_off();
        }
    }
}
