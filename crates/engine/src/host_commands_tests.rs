use serde_json::{Value, json};
use soundcraft_audio_io::{AudioBuffer, BitDepth, EncodeOptions, FileFormat};

use crate::cmd::find_command;
use crate::host_commands::{ARGUMENT_COMMANDS, ASSISTANT_COMMANDS, allowed};
use crate::host_open::open_audio_file;
use crate::{Engine, io};

/// One second of a 440 Hz tone, stereo, as a WAV at `rate`.
fn tone_wav(rate: u32, depth: BitDepth) -> Vec<u8> {
    let frames = rate as usize;
    let tone: Vec<f32> = (0..frames).map(|i| 0.25 * (i as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin()).collect();
    let buf = AudioBuffer { sample_rate: rate, channels: vec![tone.clone(), tone] };
    soundcraft_audio_io::encode(&buf, &EncodeOptions { format: FileFormat::Wav, bit_depth: depth, dither: false, bwf: None }).unwrap()
}

fn opened() -> Engine {
    let mut e = Engine::default();
    open_audio_file(&mut e, "voice.wav", &tone_wav(44_100, BitDepth::Int16)).unwrap();
    e
}

fn batch(e: &mut Engine, label: &str, steps: Value) -> Result<Value, String> {
    e.run_command_batch(&json!({ "label": label, "steps": steps }))
}

fn clips(e: &Engine) -> usize {
    e.session().tracks.iter().flat_map(|t| t.playlists.iter()).map(|p| p.clips.len()).sum()
}

#[test]
fn every_listed_command_is_a_registry_command_and_none_reaches_a_file_or_playback() {
    for id in ASSISTANT_COMMANDS {
        assert!(find_command(id).is_some(), "{id} is not a registry command");
        assert!(!id.starts_with("file.") && !id.starts_with("transport.play") && !id.starts_with("transport.record"), "{id}");
        assert!(!matches!(*id, "edit.undo" | "edit.redo" | "session.save" | "session.open"), "{id}");
    }
    assert!(ARGUMENT_COMMANDS.is_empty());
    let mut seen = std::collections::BTreeSet::new();
    for id in allowed() {
        assert!(seen.insert(id), "{id} is listed twice");
    }
}

#[test]
fn an_audio_file_opens_as_one_clip_at_its_own_rate_and_is_saved_back_in_its_own_format() {
    let mut e = Engine::default();
    let f = open_audio_file(&mut e, "voice.wav", &tone_wav(44_100, BitDepth::Int24)).unwrap();
    assert_eq!((f.format, f.bit_depth, f.rate), ("wav", "24", 44_100));
    assert_eq!(e.session().sample_rate.hz(), 44_100);
    assert_eq!(clips(&e), 1);
    assert!(!e.is_dirty(), "a freshly opened file is not unsaved work");
    assert_eq!(open_audio_file(&mut e, "a.wav", &tone_wav(48_000, BitDepth::Int16)).unwrap().rate, 48_000);
    // An MP3 or an Ogg file would be converted by a save: it is not opened as a document.
    let e2 = open_audio_file(&mut e, "song.mp3", &[0xff, 0xfb, 0x90, 0x00]).unwrap_err().to_string();
    assert!(e2.contains("song.mp3"), "{e2}");
}

#[test]
fn a_run_is_one_undo_step_named_by_its_label_and_the_saved_file_carries_it() {
    let mut e = opened();
    let depth = e.undo_history().len();
    let r = batch(
        &mut e,
        "Trim to half a second and turn it down",
        json!([
            { "name": "edit.select", "args": { "tracks": ["voice"], "start": 0, "end": 22_050 } },
            { "name": "edit.trim_to_selection", "args": {} },
            { "name": "clip.gain", "args": { "db": -6.0 } },
        ]),
    )
    .unwrap();
    assert_eq!(r["steps"].as_array().unwrap().len(), 3);
    assert_eq!(r["undoStep"], true);
    assert_eq!(e.undo_history().len(), depth + 1, "the steps are one");
    assert_eq!(e.undo_label(), Some("Trim to half a second and turn it down"));
    // The save: the main mix, in the file's own format, is what `file.bounce_mix` writes.
    let dir = std::env::temp_dir().join(format!("soundcraft-host-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let out = dir.join("voice.wav");
    e.execute("file.bounce_mix", &json!({ "path": out.to_string_lossy(), "format": "wav", "bit_depth": "16", "dither": false })).unwrap();
    let bytes = std::fs::read(&out).unwrap();
    let info = soundcraft_audio_io::probe(&bytes, Some("wav")).unwrap();
    assert_eq!((info.sample_rate, info.channels), (44_100, 2));
    assert!((22_000..=22_200).contains(&info.frames), "the file is the half second it was trimmed to: {} frames", info.frames);
    let _ = std::fs::remove_dir_all(&dir);
    // One undo takes the whole run back.
    assert!(e.undo());
    assert_eq!(e.session().tracks.iter().flat_map(|t| t.playlists.iter()).flat_map(|p| p.clips.iter()).count(), 1);
    let _ = io::session_file("x");
}

#[test]
fn a_failing_step_takes_back_the_steps_before_it() {
    let mut e = opened();
    let tracks = e.session().tracks.len();
    let depth = e.undo_history().len();
    let err = batch(
        &mut e,
        "Half done",
        json!([
            { "name": "track.new", "args": { "name": "Extra" } },
            { "name": "track.rename", "args": { "track": "No such track", "name": "x" } },
        ]),
    )
    .unwrap_err();
    assert!(err.starts_with("step-failed: step 2"), "{err}");
    assert_eq!(e.session().tracks.len(), tracks, "the new track is gone again");
    assert_eq!(e.undo_history().len(), depth);
}

#[test]
fn what_is_not_listed_is_refused_whole_and_nothing_runs() {
    let mut e = opened();
    let tracks = e.session().tracks.len();
    for (steps, code) in [
        (json!([{ "name": "track.new", "args": {} }, { "name": "session.save", "args": { "path": "x" } }]), "command-not-allowed"),
        (json!([{ "name": "edit.undo" }]), "command-not-allowed"),
        (json!([{ "name": "transport.play" }]), "command-not-allowed"),
        (json!([{ "name": "file.bounce_mix", "args": { "path": "x.wav" } }]), "command-not-allowed"),
        (json!([{ "name": "track.new", "args": { "name": "x", "path": "/etc/passwd" } }]), "argument-not-allowed"),
        (json!([{ "name": "mix.insert", "args": { "plugin": "x", "url": "https://x.example" } }]), "argument-not-allowed"),
        (json!([]), "run-empty"),
    ] {
        let err = batch(&mut e, "x", steps).unwrap_err();
        assert!(err.starts_with(code), "{err}");
        assert_eq!(e.session().tracks.len(), tracks, "{err}");
    }
    assert!(batch(&mut e, &"l".repeat(121), json!([{ "name": "session.inspect" }])).unwrap_err().starts_with("label-too-long"));
    let many: Vec<Value> = (0..51).map(|_| json!({ "name": "session.inspect" })).collect();
    assert!(batch(&mut e, "x", Value::Array(many)).unwrap_err().starts_with("run-too-long"));
}

#[test]
fn a_read_only_run_leaves_no_undo_step() {
    let mut e = opened();
    let depth = e.undo_history().len();
    let r = batch(&mut e, "Look", json!([{ "name": "session.inspect", "args": {} }])).unwrap();
    assert_eq!(r["undoStep"], false);
    assert_eq!(e.undo_history().len(), depth);
}
