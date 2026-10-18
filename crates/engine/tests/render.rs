use engine::model::{out_p, seq_p, NodeKind, Step};
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
