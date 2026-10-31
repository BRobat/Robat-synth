use engine::model::{amp_p, delay_p, out_p, seq_p, Edge, NodeKind, NodeModel, Step};
use engine::{render_offline, Project, Session};

const SR: f32 = 48000.0;

fn pair_rms(buf: &[f32], channels: usize, pair: usize) -> f32 {
    let frames = buf.len() / channels;
    let sq: f32 = (0..frames).map(|f| buf[f * channels + 2 * pair].powi(2) + buf[f * channels + 2 * pair + 1].powi(2)).sum();
    (sq / (2 * frames) as f32).sqrt()
}

#[test]
fn demo_renders_separate_channel_pairs() {
    let buf = render_offline(Project::demo(), SR, 16, 4.0);
    assert!(buf.iter().all(|x| x.is_finite() && x.abs() <= 1.0));
    let (bass, lead) = (pair_rms(&buf, 16, 0), pair_rms(&buf, 16, 1));
    assert!(bass > 0.01, "bass rms {bass}");
    assert!(lead > 0.005, "lead rms {lead}");
    for pair in 2..8 {
        assert_eq!(pair_rms(&buf, 16, pair), 0.0);
    }
}

#[test]
fn moving_an_out_node_moves_its_channels() {
    let mut pr = Project::demo();
    pr.node_mut(2).unwrap().params[out_p::PAIR] = 5.0;
    let buf = render_offline(pr, SR, 16, 2.0);
    assert_eq!(pair_rms(&buf, 16, 0), 0.0);
    assert!(pair_rms(&buf, 16, 5) > 0.01);
}

#[test]
fn live_edits_mute_and_disconnect() {
    let (mut s, mut core) = Session::offline(Project::demo(), SR);
    s.play(true);
    let mut buf = vec![0.0; 4800 * 4];
    let mut render = |s: &mut Session, secs: usize| {
        let mut total = 0.0;
        for _ in 0..secs * 10 {
            s.pump();
            core.process(&mut buf, 4);
            total += pair_rms(&buf, 4, 0);
        }
        total
    };
    assert!(render(&mut s, 1) > 0.0);
    s.toggle_mute(1);
    render(&mut s, 1); // let the release tail pass
    assert_eq!(render(&mut s, 1), 0.0);
    s.toggle_mute(1);
    s.disconnect(1, 2);
    assert_eq!(render(&mut s, 1), 0.0);
    s.connect(1, 2).unwrap();
    assert!(render(&mut s, 1) > 0.0);
}

#[test]
fn build_a_patch_from_scratch() {
    let (mut s, mut core) = Session::offline(Project::default(), SR);
    let seq = s.add_node(NodeKind::Seq, 0, 0).unwrap();
    let syn = s.add_node(NodeKind::Synth, 1, 0).unwrap();
    let out = s.add_node(NodeKind::Out, 2, 0).unwrap();
    assert!(s.connect(seq, out).is_err());
    s.connect(seq, syn).unwrap();
    s.connect(syn, out).unwrap();
    assert!(s.connect(syn, out).is_err());
    s.edit_steps(seq, |d| d.steps[0] = Step { on: true, note: 60, vel: 1.0 });
    s.set_param(seq, seq_p::MODE, 1.0);
    assert!(s.set_pattern(seq, "c [e g").is_err());
    s.set_pattern(seq, "c e g").unwrap();
    s.play(true);
    let mut buf = vec![0.0; 48000 * 2 * 2]; // two seconds = one bar at 120 bpm
    s.pump();
    core.process(&mut buf, 2);
    assert!(pair_rms(&buf, 2, 0) > 0.01);
    assert!(s.meters().unwrap().triggers[syn as usize].load(std::sync::atomic::Ordering::Relaxed) >= 3);
}

/// Renders `secs` seconds as 100 ms chunks, returning each chunk's RMS on pair 0.
fn render_chunks(s: &mut Session, core: &mut engine::EngineCore, secs: usize) -> Vec<f32> {
    let mut buf = vec![0.0; 4800 * 2];
    (0..secs * 10)
        .map(|_| {
            s.pump();
            core.process(&mut buf, 2);
            assert!(buf.iter().all(|x| x.is_finite() && x.abs() <= 1.0));
            pair_rms(&buf, 2, 0)
        })
        .collect()
}

/// The demo's bass voice (seq 0 -> synth 1) routed through `kind` into out 2.
fn bass_through(kind: NodeKind) -> (Project, u32) {
    let mut pr = Project::demo();
    pr.nodes.push(NodeModel::new(7, kind, 1, 2));
    pr.edges.retain(|e| (e.from, e.to) != (1, 2));
    pr.edges.extend([Edge::new(1, 7), Edge::new(7, 2)]);
    (pr, 7)
}

#[test]
fn every_effect_renders() {
    for kind in [NodeKind::Filter, NodeKind::Drive, NodeKind::Reverb, NodeKind::Chorus] {
        let (pr, _) = bass_through(kind);
        let buf = render_offline(pr, SR, 2, 3.0);
        assert!(buf.iter().all(|x| x.is_finite() && x.abs() <= 1.0), "{kind:?}");
        assert!(pair_rms(&buf, 2, 0) > 0.01, "{kind:?} is silent");
    }
}

#[test]
fn feedback_loop_rings_on_after_the_source_stops() {
    // synth 1 -> delay 7 -> amp 8 -> back into delay 7, delay 7 -> out 2.
    let (mut pr, delay) = bass_through(NodeKind::Delay);
    pr.node_mut(delay).unwrap().params[delay_p::FEEDBACK] = 0.0;
    let mut amp = NodeModel::new(8, NodeKind::Amp, 2, 2);
    amp.params[amp_p::GAIN] = 0.8;
    pr.nodes.push(amp);
    pr.edges.extend([Edge::new(delay, 8), Edge::new(8, delay)]);
    assert_eq!(pr.feedback_edges(), vec![Edge::new(8, delay)]);

    let (mut s, mut core) = Session::offline(pr, SR);
    s.play(true);
    assert!(render_chunks(&mut s, &mut core, 2).iter().sum::<f32>() > 0.0);
    s.toggle_mute(1);
    // The loop keeps the sound going well past the synth's release.
    let tail = render_chunks(&mut s, &mut core, 1);
    assert!(tail[5..].iter().all(|r| *r > 1e-4), "{tail:?}");
    // Cutting the return cable lets it die away.
    s.set_edge_gain(8, delay, 0.0);
    let tail = render_chunks(&mut s, &mut core, 2);
    assert!(*tail.last().unwrap() < 1e-4, "{tail:?}");
}

#[test]
fn self_loop_and_runaway_gain_stay_finite() {
    let (mut s, mut core) = Session::offline(Project::demo(), SR);
    // delay 5 feeds itself through a hot cable.
    s.connect(5, 5).unwrap();
    s.set_edge_gain(5, 5, 2.0);
    s.set_param(5, delay_p::FEEDBACK, 0.95);
    s.play(true);
    let mut buf = vec![0.0; 4800 * 4];
    for _ in 0..50 {
        s.pump();
        core.process(&mut buf, 4);
        assert!(buf.iter().all(|x| x.is_finite() && x.abs() <= 1.0));
    }
    assert!(pair_rms(&buf, 4, 1) > 0.01);
}

#[test]
fn cable_level_scales_and_mutes() {
    let (mut s, mut core) = Session::offline(Project::demo(), SR);
    s.play(true);
    let loud: f32 = render_chunks(&mut s, &mut core, 1).iter().sum();
    s.set_edge_gain(1, 2, 0.0);
    render_chunks(&mut s, &mut core, 1);
    assert_eq!(render_chunks(&mut s, &mut core, 1).iter().sum::<f32>(), 0.0);
    s.set_edge_gain(1, 2, 1.0);
    assert!(render_chunks(&mut s, &mut core, 1).iter().sum::<f32>() > loud * 0.2);
}

#[test]
fn json_roundtrip() {
    let s = Session::new(Project::demo());
    let back = Session::project_from_json(&s.to_json()).unwrap();
    assert_eq!(back, Project::demo());
}

#[test]
#[ignore = "writes demo.wav; run with --ignored to listen"]
fn write_demo_wav() {
    let buf = render_offline(Project::demo(), SR, 2, 16.0);
    let spec = hound::WavSpec { channels: 2, sample_rate: SR as u32, bits_per_sample: 32, sample_format: hound::SampleFormat::Float };
    let mut w = hound::WavWriter::create("demo.wav", spec).unwrap();
    for x in buf {
        w.write_sample(x).unwrap();
    }
}
