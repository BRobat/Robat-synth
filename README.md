# robat synth

A keyboard-driven, node-based synth for live-coded / algorave-style sets that
streams **multichannel audio into Logic Pro**. Each `out` node writes to its
own stereo pair, so every voice lands on its own Logic track.

```
seq 0 ──▶ synth 1 ──────────────▶ out 2  (1-2)
seq 3 ──▶ synth 4 ──▶ delay 5 ──▶ out 6  (3-4)
```

Nodes glow, ripple and send pulses along their cables when they play, the
first seed of the "living organism / city" visual layer.

## Run

Requires Rust (stable). On macOS:

```sh
cargo run --release -p robat-synth
```

To build a `.app`: `cargo install cargo-bundle && cargo bundle --release -p robat-synth`
(the result is in `target/release/bundle/osx/`).

The app opens with a demo patch (or your last session, auto-saved on quit).
Press **space** to play and **?** for help.

## Getting the audio into Logic Pro

1. Install the free loopback driver **BlackHole 16ch**:
   `brew install blackhole-16ch` (or from https://existential.audio/blackhole/).
2. In robat synth, type `:device` and pick **BlackHole 16ch**. The status line
   shows the device and channel count.
3. In Logic: *Settings → Audio → Devices*: **Input Device = BlackHole 16ch**,
   Output Device = your speakers/interface.
4. Create one audio track per `out` node and set its input to the matching
   pair (`out` with *channels 1-2* → input 1-2, *3-4* → input 3-4, …). Turn on
   input monitoring (or arm the track) to hear it, and record.
5. Set the same tempo in both (`:bpm 118` in the app). The two clocks are not
   linked yet, so start the app's transport (space) and record in Logic.

Without BlackHole you can play straight to your speakers. Only the pairs your
device has are audible; an `out` node pointing at a missing pair shows its
channels in red. If no audio device can be opened, the app runs on a
silent clock so you can keep editing.

## Keyboard

Everything is keyboard-driven (vim-ish); the mouse is only for clicking a node
to select it.

| graph | |
|---|---|
| `h j k l` / arrows | select the nearest node in that direction |
| `H J K L` / shift+arrows | move the node (swaps with a node in the way) |
| `tab` / `shift+tab` | cycle through nodes |
| `a` | add a node (type to filter, enter); it auto-wires from the selection |
| `c` … `enter` | connect selection → target (pick the target with hjkl); loops allowed |
| `d` | disconnect (pick which cable) |
| `x` · `y` · `m` · `u` | delete · duplicate · mute · undo |
| `enter` / `e` | edit node |
| `space` | play / stop (in graph and edit modes) |
| `+` / `-` | bpm |
| `:` | command line |
| `?` | help |

| edit | |
|---|---|
| `j k` / `h l` / `H L` | choose param or cable level / adjust / adjust coarse |
| `=` · `0` | type an exact value · reset to default |
| `tab` | sequencer: switch between params and steps/pattern |
| steps: `h l` `j k` `J K` | move cursor, note ±1, octave ±1 |
| steps: `enter`/`x` `v` `<` `>` `p` `C` | toggle, velocity, rotate, copy to next, clear |
| pattern: `i` … `enter` | edit the pattern; it takes over at the next bar |
| `esc` | back |

Commands: `:w [name]`, `:e [name]` (projects live in `~/Documents/Robat Synth/`),
`:new`, `:demo`, `:device`, `:bpm 124`, `:q`.

## Nodes

| node | in → out | params |
|---|---|---|
| `seq` | – → notes | mode (grid/pattern), rate (steps per bar), length (1–64, odd lengths make polymeters), gate, swing, transpose |
| `synth` | notes → audio | 8 voices; wave (sine/saw/square/tri/noise), osc2 mix + detune, octave, ADSR, low-pass cutoff/res/env amount, level |
| `amp` | audio → audio | gain, pan |
| `filter` | audio → audio | mode (lp/bp/hp/notch), cutoff, res, drive |
| `drive` | audio → audio | shape (soft/hard/fold), drive, bits (16 = clean), rate (sample-rate ÷), mix, level |
| `delay` | audio → audio | time in 1/16 notes (tempo-synced), feedback, mix |
| `chorus` | audio → audio | rate, depth, time (ms), feedback, mix; short time + feedback = flanger |
| `reverb` | audio → audio | size, damp, width, mix (Freeverb) |
| `out` | audio → device | channel pair (1-2 … 15-16), level (soft-clipped) |

Several cables into one input are summed, so any node can be used as a mixer.

### Cables and loops

Every audio cable has its own **level** (0–2, default 1). In edit mode the
node's incoming cables are listed under *cable levels*, below its params, and
are adjusted like any param. A cable that isn't at 1 shows its level on the
canvas.

Audio cables may form **loops**, including a node feeding itself: e.g.
`delay → filter → drive → delay` for a dirty dub echo, or `chorus → chorus`
for a metallic resonator. The cable that closes a loop is drawn dashed and
carries the previous 64-sample block (≈1.3 ms at 48 kHz). Loop returns are
soft-limited and a node that blows up is reset, so a hot loop saturates rather
than exploding; turn its cable level down to tame it.

## Pattern language

A Tidal-style mini-notation where **one cycle = one bar**, and note names
follow Logic (C3 = MIDI 60):

| syntax | meaning |
|---|---|
| `c3 eb4 f#2 60` | notes (octave defaults to 3) or MIDI numbers |
| `~` | rest |
| `[a b c]` | squeeze a group into one step |
| `<a b c>` | one per bar, in turn |
| `a*2` `[a b]/2` | faster / slower |
| `a@3 b` | weight (a takes 3/4) |
| `a!3` / `a ! !` | repeat |
| `a, b` / `[c3, e3, g3]` | play at the same time (chords, layers) |

Example: `c1 ~ c1 [~ c2] <eb1 g0> ~ c1 [bb0 c1]`

## Architecture

```
crates/pattern  mini-notation parser + evaluator (exact rational time)
crates/engine   realtime node graph: model, DSP, processors, Session
app             egui/eframe UI, cpal (CoreAudio) device, keyboard modes
```

The UI owns a `Session` (the project model). Each edit is mirrored to the
audio thread as commands over lock-free ring buffers. The audio thread never
allocates, locks or frees: replaced graphs, processors and patterns are sent
back to the UI thread to be dropped. Sequencer events are sample-accurate
within the device buffer.

```sh
cargo test                                        # all tests
cargo test -p engine --release -- --ignored       # render demo.wav
```

## Roadmap

- visual layer: each node emits generative graphics, so the patch becomes a living city
- audio input nodes (guitar, mics) through the same multichannel device
- modulation (LFOs, envelopes into params), more oscillators, samples
- tempo/transport sync with Logic (MIDI clock / Ableton Link)
