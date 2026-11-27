//! The probe's host tests: the clip format, the bundled clips themselves, and the machine over a simulated player.

use super::clip::{self, Clip, ClipError, Codec, Packet};
use super::machine::{Callback, Cmd, Kind, Outcome, Probe, Scenario, Stream, SEEK_TO_MS};
use super::*;
use std::collections::VecDeque;

fn bundled_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tizen/esprobe")
}

fn bundled(name: &str) -> Clip {
    clip::parse(&std::fs::read(bundled_dir().join(format!("{name}.plxes"))).expect("bundled clip")).expect("parses")
}

/// **Every bundled clip parses, and the list the probe reads is the list the package stages.** A clip the packager
/// does not stage would be a FAIL on every set; one the probe does not read would ship for nothing.
#[test]
fn every_bundled_clip_parses_and_the_probe_and_the_packager_name_the_same_clips() {
    for (name, c) in load_clips(&bundled_dir()) {
        let c = c.unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(!c.packets.is_empty(), "{name}");
        assert!(c.packets[0].key, "{name} starts on a keyframe");
    }
    let package = include_str!("../../../../tizen/package.py");
    let line = package.lines().find(|l| l.starts_with("ESPROBE_CLIPS = (")).expect("package.py names the clips");
    let listed: Vec<&str> = line.split('"').skip(1).step_by(2).collect();
    assert_eq!(listed, CLIPS, "tizen/package.py ESPROBE_CLIPS and esprobe::CLIPS");
}

/// **The clips are in the form the Feed engine hands its sink** (`plx_play_feed::ff`): H.264 and HEVC Annex-B with the
/// parameter sets in-band on the keyframe, B-frames in decode order (the H.264 presentation times are not monotonic),
/// AAC as ADTS frames, and the clock clip long enough for the pause, the seek target and the end.
#[test]
fn the_clips_are_annex_b_with_in_band_parameter_sets_and_adts_aac() {
    let nal_types = |data: &[u8], hevc: bool| -> Vec<u8> {
        let mut out = Vec::new();
        let mut i = 0;
        while i + 3 < data.len() {
            if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
                let b = data[i + 3];
                out.push(if hevc { (b >> 1) & 0x3f } else { b & 0x1f });
                i += 3;
            }
            i += 1;
        }
        out
    };
    let h264 = bundled("h264");
    assert_eq!(h264.codec, Codec::H264);
    assert_eq!((h264.a, h264.b), (640, 360));
    let first = nal_types(&h264.packets[0].data, false);
    assert!(first.contains(&7) && first.contains(&8) && first.contains(&5), "SPS, PPS and IDR in the first AU: {first:?}");
    assert!(h264.packets.windows(2).any(|w| w[1].pts_ms < w[0].pts_ms), "B-frames: decode order is not presentation order");
    assert!(h264.end_ms() >= 7_900, "the clock clip runs past the seek target to an end: {}", h264.end_ms());
    assert!(h264.end_ms() > SEEK_TO_MS + 1_000);
    for name in ["hevc", "hevc10"] {
        let c = bundled(name);
        let first = nal_types(&c.packets[0].data, true);
        assert!(first.contains(&32) && first.contains(&33) && first.contains(&34), "{name}: VPS, SPS, PPS in-band: {first:?}");
    }
    assert!(bundled("hevc10").hdr10 && !bundled("hevc").hdr10);
    let aac = bundled("aac");
    assert_eq!((aac.a, aac.b), (2, 48_000));
    assert!(aac.packets.iter().all(|p| p.data.len() > 7 && p.data[0] == 0xff && p.data[1] & 0xf0 == 0xf0), "ADTS sync words");
    assert!(!bundled("av1").codec_data.is_empty(), "AV1 carries its av1C");
}

fn header(kind: u8, codec: u8, packets: &[(i64, u32, u8, &[u8])]) -> Vec<u8> {
    let mut b = clip::MAGIC.to_vec();
    b.extend_from_slice(&[kind, codec, 0, 0]);
    for v in [320u32, 180, 25, 1, 0] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    b.extend_from_slice(&(packets.len() as u32).to_le_bytes());
    for (pts, dur, key, data) in packets {
        b.extend_from_slice(&pts.to_le_bytes());
        b.extend_from_slice(&dur.to_le_bytes());
        b.extend_from_slice(&[*key, 0, 0, 0]);
        b.extend_from_slice(&(data.len() as u32).to_le_bytes());
        b.extend_from_slice(data);
    }
    b
}

/// **A clip that is not exactly the format is refused with a reason**, never read past its end.
#[test]
fn a_damaged_clip_is_refused_with_its_reason() {
    let good = header(1, 1, &[(0, 40_000, 1, b"abc"), (40_000, 40_000, 0, b"de")]);
    let c = clip::parse(&good).expect("good");
    assert_eq!(c.packets[1], Packet { pts_ms: 40, duration_ms: 40, key: false, data: b"de".to_vec() });
    assert_eq!(clip::parse(&good[..good.len() - 1]), Err(ClipError::Truncated("packet data")));
    let mut trailing = good.clone();
    trailing.push(0);
    assert_eq!(clip::parse(&trailing), Err(ClipError::Trailing(1)));
    let mut magic = good.clone();
    magic[0] = b'X';
    assert_eq!(clip::parse(&magic), Err(ClipError::Magic));
    assert_eq!(clip::parse(&header(2, 1, &[(0, 1, 1, b"a")])), Err(ClipError::KindMismatch), "an audio kind with a video codec");
    assert_eq!(clip::parse(&header(1, 99, &[(0, 1, 1, b"a")])), Err(ClipError::Codec(99)));
    assert_eq!(clip::parse(&header(1, 1, &[])), Err(ClipError::Empty));
    let mut huge = header(1, 1, &[]);
    let at = huge.len() - 4;
    huge[at..].copy_from_slice(&100_000u32.to_le_bytes());
    assert_eq!(clip::parse(&huge), Err(ClipError::TooMany(100_000)));
}

/// **After a seek the feed resumes where the decoder can start**: video at the last keyframe at or before the target,
/// audio at the first packet that ends after it; past the end, nowhere.
#[test]
fn a_seek_resumes_video_at_the_keyframe_before_the_target_and_audio_at_the_target() {
    let h264 = bundled("h264");
    let i = h264.resume_index(SEEK_TO_MS).expect("inside the clip");
    let p = &h264.packets[i];
    assert!(p.key && p.pts_ms <= SEEK_TO_MS);
    assert!(h264.packets[i + 1..].iter().filter(|q| q.key).all(|q| q.pts_ms > SEEK_TO_MS), "the LAST such keyframe");
    let aac = bundled("aac");
    let j = aac.resume_index(SEEK_TO_MS).expect("inside the clip");
    assert!(aac.packets[j].pts_ms + aac.packets[j].duration_ms > SEEK_TO_MS);
    assert!(j == 0 || aac.packets[j - 1].pts_ms + aac.packets[j - 1].duration_ms <= SEEK_TO_MS);
    assert_eq!(h264.resume_index(h264.end_ms()), None);
}

/// The PCM tone is 48 kHz stereo 16-bit in 20 ms packets, bounded and continuous.
#[test]
fn the_pcm_tone_is_twenty_millisecond_packets_of_stereo_sixteen_bit() {
    let c = clip::pcm_tone(2);
    assert_eq!((c.codec, c.a, c.b, c.packets.len()), (Codec::PcmS16le, 2, 48_000, 100));
    assert!(c.packets.iter().enumerate().all(|(i, p)| p.pts_ms == i as u64 * 20 && p.data.len() == 960 * 4));
    let samples: Vec<i16> = c.packets[0].data.chunks(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
    assert!(samples.iter().all(|s| s.unsigned_abs() <= 6000));
    assert!(samples.iter().any(|s| *s > 5000) && samples.iter().any(|s| *s < -5000));
}

/// **The layouts are the library's, or nothing is submitted.** On a 64-bit build they are the emulator library's own
/// sizes (64, 40, 32); any other answer refuses.
#[test]
fn the_struct_layouts_must_equal_the_librarys_sizes() {
    let ours = api::compiled_sizes();
    #[cfg(target_pointer_width = "64")]
    assert_eq!(ours, [64, 40, 32], "the sizes libesplusplayer.so 1.11.3 reports on the TV 10.0 emulator");
    let as_native = ours.map(|n| n as i64);
    assert!(api::layout_matches(as_native));
    assert!(!api::layout_matches([as_native[0] + 8, as_native[1], as_native[2]]), "a grown packet struct");
    assert!(!api::layout_matches([as_native[0], as_native[1], -1]));
}

/// The scenarios, in their order, from the bundled clips; a clip that failed to load drops its scenario only.
#[test]
fn the_scenarios_are_the_clock_run_the_pair_and_one_per_codec() {
    let clips = load_clips(&bundled_dir());
    let names: Vec<&str> = scenarios(&clips).iter().map(|s| s.name).collect();
    assert_eq!(names, ["h264-clock", "h264+aac", "hevc", "hevc10-hdr10", "av1", "vp9", "aac", "ac3", "eac3", "pcm_s16le"]);
    let without_hevc: Vec<_> = clips.into_iter().map(|(n, c)| (n, if n == "hevc" { Err("gone".to_owned()) } else { c })).collect();
    let names: Vec<&str> = scenarios(&without_hevc).iter().map(|s| s.name).collect();
    assert!(!names.contains(&"hevc") && names.contains(&"hevc10-hdr10"));
}

/// A simulated player: answers every command at once, raises the callbacks a real one raises, and keeps a clock that
/// runs while playing and never passes what was fed.
#[derive(Default)]
struct Sim {
    now: u64,
    playing: bool,
    pos: u64,
    fed_to: u64,
    prepared: bool,
    submitted: usize,
    submitted_before_ready: usize,
    ready: bool,
    eos_streams: usize,
    eos_wanted: usize,
    full_every: usize,
    pending: VecDeque<(u64, Callback)>,
    seeks: Vec<u64>,
    frozen: bool,
    never_prepare: bool,
    /// Report the first decoded picture once prepared.
    first_frame: bool,
    /// The packet last refused with `Full`, and how many times the next submit on its stream was another packet.
    refused: Option<(Stream, usize)>,
    retry_skipped: usize,
    log: Vec<Cmd>,
}

impl Sim {
    fn answer(&mut self, probe: &Probe, cmd: Cmd) -> Outcome {
        self.log.push(cmd);
        match cmd {
            Cmd::Open => Outcome::Open(Ok(())),
            Cmd::Display | Cmd::StreamInfo(_) => Outcome::Rc(0),
            Cmd::Prepare => {
                self.eos_wanted = [Stream::Audio, Stream::Video].iter().filter(|&&s| probe.clip(s).is_some()).count();
                for s in [Stream::Video, Stream::Audio] {
                    if probe.clip(s).is_some() {
                        self.pending.push_back((self.now + 5, Callback::ReadyToPrepare(s)));
                    }
                }
                Outcome::Rc(0)
            }
            Cmd::Submit(s, i) => {
                if !self.ready {
                    self.submitted_before_ready += 1;
                }
                self.submitted += 1;
                if let Some((rs, ri)) = self.refused {
                    if rs == s {
                        self.retry_skipped += usize::from(ri != i);
                        self.refused = None;
                    }
                }
                if self.full_every > 0 && self.submitted % self.full_every == 0 {
                    self.refused = Some((s, i));
                    return Outcome::Rc(api::SUBMIT_FULL);
                }
                let (p, _) = probe.packet(s, i).expect("a packet the clip has");
                self.fed_to = self.fed_to.max(p.pts_ms + p.duration_ms);
                if !self.prepared && !self.never_prepare && self.submitted >= 5 {
                    self.prepared = true;
                    self.pending.push_back((self.now + 20, Callback::PrepareDone(true)));
                    if self.first_frame && probe.clip(Stream::Video).is_some() {
                        self.pending.push_back((self.now + 30, Callback::FirstFrame));
                    }
                }
                Outcome::Rc(api::SUBMIT_SUCCESS)
            }
            Cmd::Eos(_) => {
                self.eos_streams += 1;
                Outcome::Rc(api::SUBMIT_SUCCESS)
            }
            Cmd::Start | Cmd::Resume => {
                self.playing = true;
                Outcome::Rc(0)
            }
            Cmd::Pause => {
                self.playing = false;
                Outcome::Rc(0)
            }
            Cmd::Seek(t) => {
                self.seeks.push(t);
                self.pos = t;
                self.fed_to = 0;
                for s in [Stream::Video, Stream::Audio] {
                    if probe.clip(s).is_some() {
                        self.pending.push_back((self.now + 10, Callback::ReadyToSeek(s, t)));
                    }
                }
                self.pending.push_back((self.now + 200, Callback::SeekDone));
                Outcome::Rc(0)
            }
            Cmd::Time => Outcome::Time(Ok(self.pos)),
            Cmd::Teardown => Outcome::Teardown(0, 0, 0),
        }
    }

    fn tick(&mut self, ms: u64, end: u64) {
        self.now += ms;
        if self.playing && !self.frozen && self.fed_to > self.pos {
            self.pos = (self.pos + ms).min(self.fed_to);
        }
        if self.eos_wanted > 0 && self.eos_streams == self.eos_wanted && self.pos + 40 >= end {
            self.eos_wanted = 0;
            self.pending.push_back((self.now, Callback::Eos));
        }
    }
}

/// Run `probe` over `sim` until it finishes (or 300 simulated seconds), 10 ms a step. Returns the lines.
fn drive(probe: &mut Probe, sim: &mut Sim) -> Vec<String> {
    let mut lines = Vec::new();
    let mut end = 0;
    while !probe.finished() && sim.now < 300_000 {
        let due: Vec<Callback> = {
            let (now_due, later): (Vec<_>, Vec<_>) = sim.pending.drain(..).partition(|(at, _)| *at <= sim.now);
            sim.pending = later.into();
            now_due.into_iter().map(|(_, cb)| cb).collect()
        };
        for cb in due {
            if let Callback::ReadyToPrepare(_) | Callback::ReadyToSeek(..) = cb {
                sim.ready = true;
            }
            probe.callback(cb, sim.now);
        }
        while let Some(cmd) = probe.next(sim.now) {
            if cmd == Cmd::Open {
                *sim = Sim { now: sim.now, full_every: sim.full_every, frozen: sim.frozen, never_prepare: sim.never_prepare, first_frame: sim.first_frame, log: std::mem::take(&mut sim.log), ..Sim::default() };
                end = [Stream::Audio, Stream::Video].iter().filter_map(|&s| probe.clip(s)).map(|c| c.end_ms()).max().unwrap_or(0);
            }
            let outcome = sim.answer(probe, cmd);
            probe.done(cmd, outcome, sim.now, 1);
        }
        lines.extend(probe.take_lines());
        sim.tick(10, end);
    }
    lines.extend(probe.take_lines());
    lines
}

fn verdicts(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .filter(|l| l.starts_with("esprobe: PASS") || l.starts_with("esprobe: FAIL"))
        .map(|l| l.split_whitespace().take(4).collect::<Vec<_>>().join(" "))
        .collect()
}

fn clock() -> Scenario {
    Scenario { name: "h264-clock", kind: Kind::Clock, video: Some(Arc::new(bundled("h264"))), audio: None }
}

/// **The clock scenario walks every step the Feed engine needs, in order, and passes each** over a player that
/// behaves: open (and the privilege line), display, stream info, prepare, the position advancing, pause holding it,
/// resume, a seek landing at its target, the end of stream, and the teardown.
#[test]
fn the_clock_scenario_passes_every_step_in_order_over_a_player_that_behaves() {
    let mut probe = Probe::new(vec![clock()]);
    let mut sim = Sim::default();
    let lines = drive(&mut probe, &mut sim);
    assert_eq!(
        verdicts(&lines),
        [
            "esprobe: PASS h264-clock open",
            "esprobe: PASS privilege create",
            "esprobe: PASS h264-clock display",
            "esprobe: PASS h264-clock video-info",
            "esprobe: PASS h264-clock prepare",
            "esprobe: PASS h264-clock position",
            "esprobe: PASS h264-clock pause",
            "esprobe: PASS h264-clock resume",
            "esprobe: PASS h264-clock seek",
            "esprobe: PASS h264-clock eos",
            "esprobe: PASS h264-clock teardown",
        ],
        "{lines:#?}"
    );
    assert_eq!((probe.pass, probe.fail), (11, 0));
    assert_eq!(sim.seeks, [SEEK_TO_MS]);
    assert!(lines.iter().any(|l| l.contains("scenario h264-clock passed")));
}

/// **Nothing is submitted before the player asks** (`ready_to_prepare`), and a `Full` answer is retried, not skipped:
/// every packet of the clip is accepted exactly once.
#[test]
fn nothing_is_submitted_before_the_player_asks_and_a_full_buffer_is_retried() {
    let mut probe = Probe::new(vec![clock()]);
    let mut sim = Sim { full_every: 7, ..Sim::default() };
    let lines = drive(&mut probe, &mut sim);
    assert_eq!(sim.submitted_before_ready, 0, "a packet before ready_to_prepare");
    assert_eq!(probe.fail, 0, "{lines:#?}");
    assert!(sim.submitted > 200, "the whole clip was fed: {}", sim.submitted);
    assert_eq!(sim.retry_skipped, 0, "a refused packet was skipped instead of retried");
}

/// **A player that never finishes preparing fails the prepare step and is still torn down**; the next scenario runs.
#[test]
fn a_prepare_that_never_finishes_fails_and_the_player_is_still_torn_down() {
    let accept = Scenario { name: "hevc", kind: Kind::Accept, video: Some(Arc::new(bundled("hevc"))), audio: None };
    let mut probe = Probe::new(vec![accept, Scenario { name: "aac", kind: Kind::Accept, video: None, audio: Some(Arc::new(bundled("aac"))) }]);
    let mut sim = Sim { never_prepare: true, ..Sim::default() };
    let lines = drive(&mut probe, &mut sim);
    let v = verdicts(&lines);
    assert!(v.contains(&"esprobe: FAIL hevc prepare".to_owned()), "{lines:#?}");
    assert!(v.contains(&"esprobe: PASS hevc teardown".to_owned()));
    assert!(v.contains(&"esprobe: FAIL aac prepare".to_owned()), "the second scenario ran");
    assert_eq!(sim.log.iter().filter(|c| **c == Cmd::Teardown).count(), 2);
}

/// **A position that does not move fails the clock step**, with what was read.
#[test]
fn a_position_that_never_advances_fails() {
    let mut probe = Probe::new(vec![clock()]);
    let mut sim = Sim { frozen: true, ..Sim::default() };
    let lines = drive(&mut probe, &mut sim);
    let failed = lines.iter().find(|l| l.starts_with("esprobe: FAIL h264-clock position")).expect("a FAIL");
    assert!(failed.contains("did not advance"), "{failed}");
}

/// **A refused open says what it shows about the privilege** (permission denied is the privilege's FAIL), and the
/// player that was created is torn down.
#[test]
fn a_permission_denied_open_is_the_privileges_fail() {
    let mut probe = Probe::new(vec![clock()]);
    assert_eq!(probe.next(0), Some(Cmd::Open));
    probe.done(Cmd::Open, Outcome::Open(Err(("open", -13))), 1, 1);
    assert_eq!(probe.next(2), Some(Cmd::Teardown), "the created player is released");
    let lines = probe.take_lines();
    assert!(lines.iter().any(|l| l.starts_with("esprobe: FAIL h264-clock open open -> -13 (permission_denied)")), "{lines:#?}");
    assert!(lines.iter().any(|l| l.starts_with("esprobe: FAIL privilege")));
}

/// **An error callback ends the scenario as a FAIL that names the code**, and the player is torn down. What the same
/// cause raises after it (the emulator's AV1 refusal: the error, then `prepare_async_done(false)`) is a note, not a
/// second FAIL, so the tally counts the scenario once.
#[test]
fn an_error_callback_fails_the_scenario_with_its_code() {
    let mut probe = Probe::new(vec![clock()]);
    assert_eq!(probe.next(0), Some(Cmd::Open));
    probe.done(Cmd::Open, Outcome::Open(Ok(())), 1, 1);
    let fail_before = probe.fail;
    probe.callback(Callback::Error(-26476497), 2);
    assert_eq!(probe.next(3), Some(Cmd::Teardown));
    probe.callback(Callback::PrepareDone(false), 4);
    probe.callback(Callback::ResourceConflicted, 5);
    let lines = probe.take_lines();
    assert!(lines.iter().any(|l| l.contains("FAIL h264-clock error error callback -26476497 (not_supported_video_codec)")), "{lines:#?}");
    assert_eq!(probe.fail, fail_before + 1, "one cause, one FAIL: {lines:#?}");
    assert_eq!(lines.iter().filter(|l| l.starts_with("esprobe: FAIL")).count(), 1, "{lines:#?}");
    assert!(lines.iter().any(|l| l == "esprobe: h264-clock after the failure: prepare prepare_async_done(false)"), "{lines:#?}");
}

/// **A failed scenario's refused teardown is a note, not a second FAIL.** A refused `open` (what a set without the Partner
/// privilege may answer) is followed by the teardown of the created player, which such a player may refuse too; that is
/// the same cause again, so the tally counts the scenario once.
#[test]
fn a_refused_teardown_after_a_failed_open_is_a_note_not_a_second_fail() {
    let mut probe = Probe::new(vec![clock()]);
    assert_eq!(probe.next(0), Some(Cmd::Open));
    probe.done(Cmd::Open, Outcome::Open(Err(("open", -13))), 1, 1);
    let failed = probe.fail;
    assert_eq!(probe.next(2), Some(Cmd::Teardown));
    probe.done(Cmd::Teardown, Outcome::Teardown(-38, -38, 0), 3, 1);
    let lines = probe.take_lines();
    assert_eq!(probe.fail, failed, "{lines:#?}");
    assert!(lines.iter().any(|l| l == "esprobe: h264-clock after the failure: teardown stop -38, close -38, destroy 0 (1 ms)"), "{lines:#?}");
    assert_eq!(probe.next(4), None);
    assert!(probe.finished());
}

/// **Where the library offers the first-decoded-picture callback, a video scenario notes whether it came**, with its
/// time when it did; it is never a PASS or FAIL (the TV 10.0 emulator never raises it), and an audio scenario notes
/// nothing.
#[test]
fn the_first_frame_note_says_whether_the_callback_came_and_never_counts() {
    let hevc = || Scenario { name: "hevc", kind: Kind::Accept, video: Some(Arc::new(bundled("hevc"))), audio: None };
    let aac = || Scenario { name: "aac", kind: Kind::Accept, video: None, audio: Some(Arc::new(bundled("aac"))) };
    let notes = |lines: &[String]| -> Vec<String> { lines.iter().filter(|l| l.contains("first-frame")).cloned().collect() };

    let mut probe = Probe::new(vec![hevc(), aac()]);
    probe.watch_first_frame(true);
    let mut sim = Sim { first_frame: true, ..Sim::default() };
    let lines = drive(&mut probe, &mut sim);
    let n = notes(&lines);
    assert_eq!(n.len(), 1, "{n:#?}");
    assert!(n[0].starts_with("esprobe: hevc first-frame: first_video_decoding_done ") && n[0].ends_with(" ms after prepare"), "{n:#?}");
    let counted = (probe.pass, probe.fail);

    let mut probe = Probe::new(vec![hevc(), aac()]);
    probe.watch_first_frame(true);
    let lines = drive(&mut probe, &mut Sim::default());
    assert_eq!(notes(&lines), ["esprobe: hevc first-frame: no first_video_decoding_done callback"]);
    assert_eq!((probe.pass, probe.fail), counted, "the note changes no tally");
    assert!(lines.iter().any(|l| l.contains("scenario hevc passed")));

    let mut probe = Probe::new(vec![hevc()]);
    assert!(notes(&drive(&mut probe, &mut Sim::default())).is_empty(), "nothing when the export is missing");
}

/// **The hook's answer**: running while a probe runs, idle before the first, then the last tally.
#[test]
fn the_status_is_running_then_the_last_tally() {
    use plx_os::tv::packet_probe::Status;
    assert_eq!(status_of(false, NEVER, 0, false), Status::Idle);
    assert_eq!(status_of(true, NEVER, 0, false), Status::Running);
    assert_eq!(status_of(true, 5, pack(3, 1), false), Status::Running);
    assert_eq!(status_of(false, pack(57, 6), 0, false), Status::Done { pass: 57, fail: 6 });
    assert_eq!(status_of(false, 0, 0, false), Status::Done { pass: 0, fail: 0 });
}

/// **A run stuck in a native call past its deadline reads done**, with the tally it reached and one more FAIL for the
/// call that never returned, instead of `Running` for the rest of the session.
#[test]
fn a_run_stuck_past_its_deadline_reads_done_with_one_more_fail() {
    use plx_os::tv::packet_probe::Status;
    assert_eq!(status_of(true, NEVER, pack(12, 1), true), Status::Done { pass: 12, fail: 2 });
    assert_eq!(status_of(true, pack(57, 5), pack(3, 0), true), Status::Done { pass: 3, fail: 1 }, "this run's tally, not the last one's");
    assert_eq!(status_of(false, pack(57, 5), 0, true), Status::Done { pass: 57, fail: 5 }, "a run that ended reads its own tally");
}

/// **A cancel (the app went to the background, or the deadline) tears the open player down and runs nothing more**:
/// one FAIL that says why, the ordinary teardown, no further scenario, and a second cancel adds nothing.
#[test]
fn a_cancel_tears_the_open_player_down_and_runs_nothing_more() {
    let hevc = Scenario { name: "hevc", kind: Kind::Accept, video: Some(Arc::new(bundled("hevc"))), audio: None };
    let mut probe = Probe::new(vec![clock(), hevc]);
    assert_eq!(probe.next(0), Some(Cmd::Open));
    probe.done(Cmd::Open, Outcome::Open(Ok(())), 1, 1);
    let fail = probe.fail;
    probe.cancel("the app went to the background");
    probe.cancel("again");
    assert_eq!(probe.next(2), Some(Cmd::Teardown), "the open player is torn down");
    probe.done(Cmd::Teardown, Outcome::Teardown(0, 0, 0), 3, 1);
    assert_eq!(probe.next(4), None);
    assert!(probe.finished());
    let lines = probe.take_lines();
    assert_eq!(probe.fail, fail + 1, "{lines:#?}");
    assert!(lines.iter().any(|l| l == "esprobe: FAIL h264-clock cancelled the app went to the background"), "{lines:#?}");
    assert!(!lines.iter().any(|l| l.contains("hevc")), "no further scenario: {lines:#?}");

    // Before any player: nothing to tear down.
    let mut probe = Probe::new(vec![clock()]);
    probe.cancel("the app is stopping");
    assert_eq!(probe.next(0), None);
    assert!(probe.finished());
    assert_eq!(probe.fail, 1);
}
