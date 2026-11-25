//! The actual master -> media cursor, with playlist responses in memory: no native codec or wire.
use super::*;
use std::collections::VecDeque;
use std::sync::Mutex;

const MASTER: &str = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1000000\nmedia.m3u8\n";
const LIVE_ENTRY: &str = "/start.m3u8?path=%2Flivetv%2Fsessions%2F9b9fe727-0000-46db-b94c-ef512e3e61d2&X-Plex-Token=fixture";
const LIBRARY_ENTRY: &str = "/start.m3u8?path=%2Flibrary%2Fmetadata%2F5&X-Plex-Token=fixture";

struct Playlists {
    replies: VecDeque<(&'static str, String)>,
    body: VecDeque<u8>,
    size: i64,
}

impl RangeReader for Playlists {
    fn reopen_until(&mut self, url: &str, _: Option<std::time::Instant>,
        _: &mut dyn plx_base::checkpoint::Checkpoint) -> Result<(), range_io::OpenErr> {
        let (path, text) = self.replies.pop_front().expect("unexpected playlist request");
        assert!(url.split('?').next().unwrap().ends_with(&format!("/{path}")), "{url}");
        self.size = text.len() as i64;
        self.body = text.into_bytes().into();
        Ok(())
    }
    fn read_until(&mut self, dst: &mut [u8], _: Option<std::time::Instant>,
        _: &mut dyn plx_base::checkpoint::Checkpoint) -> c_int {
        let n = dst.len().min(self.body.len());
        for byte in &mut dst[..n] { *byte = self.body.pop_front().unwrap(); }
        n as c_int
    }
    fn size(&self) -> i64 { self.size }
    fn status(&self) -> c_int { 200 }
    fn body_complete(&self) -> bool { self.body.is_empty() }
    fn body_failure(&self) -> Option<range_io::BodyFailure> { None }
    fn body_receipt(&mut self) -> plx_net::stream::BodyReceipt {
        plx_net::stream::BodyReceipt { ahead: self.body.len() as i64, finished: true, stepped: false }
    }
    fn drain_available(&mut self, _: &mut [u8]) -> c_int { unreachable!() }
    fn seek(&mut self, _: i64) -> bool { unreachable!() }
    fn abort(&self) { unreachable!() }
}

struct NoDial;
impl RangeFactory for NoDial {
    fn reserve_open(&self) -> Result<range_io::OpenReservation, range_io::OpenErr> { unreachable!() }
    fn open_reserved(&self, _: &str, _: i64, _: range_io::OpenReservation)
        -> Result<Box<dyn RangeReader>, range_io::OpenErr> { unreachable!() }
    fn open_reserved_checked(&self, _: &str, _: i64, _: range_io::OpenReservation,
        _: Option<std::time::Instant>, _: &mut dyn plx_base::checkpoint::Checkpoint)
        -> Result<Box<dyn RangeReader>, range_io::OpenErr> { unreachable!() }
}

#[derive(Default)]
struct Observer { durations: Mutex<Vec<i64>> }
impl DemuxObserver for Observer {
    fn attempt(&self) -> plx_playback_contract::AttemptId { plx_playback_contract::AttemptId::new(0, 0) }
    fn playpos_ns(&self) -> Option<i64> { Some(0) }
    fn caption_retention(&self) -> Option<plx_playback_contract::subtitle::CaptionRetention> { None }
    fn hold_epoch(&self) -> Option<u64> { Some(0) }
    fn rebuffering(&self) -> Option<bool> { Some(false) }
    fn audio_expected(&self) -> Option<bool> { Some(true) }
    fn duration_ns(&self) -> Option<i64> { Some(0) }
    fn declared_video_code(&self) -> Option<u8> { Some(0) }
    fn declared_audio_code(&self) -> Option<u8> { Some(0) }
    fn paused(&self) -> Option<bool> { Some(false) }
    fn pause_clock(&self) -> Option<plx_playback_contract::demux::PauseClock> { None }
    fn hls_buffer_ms(&self) -> Option<Option<i64>> { Some(None) }
    fn hls_buffer_clocks(&self) -> Option<HlsBufferClocks> { None }
    fn remaining_playback_ms(&self) -> Option<i64> { Some(0) }
    fn user_sequence(&self) -> Option<u64> { Some(0) }
    fn with_active(&self, f: &mut dyn FnMut()) -> bool { f(); true }
    fn publish(&self, effect: DemuxCallbackEffect) -> bool {
        if let DemuxCallbackEffect::DurationNs(ns) = effect { self.durations.lock().unwrap().push(ns); }
        true
    }
}

fn media(count: u64, kind: &str, end_list: bool) -> String {
    let mut text = format!("#EXTM3U\n#EXT-X-TARGETDURATION:1\n#EXT-X-MEDIA-SEQUENCE:0\n{kind}");
    for n in 0..count { text.push_str(&format!("#EXTINF:1,\n{n}.ts\n")); }
    if end_list { text.push_str("#EXT-X-ENDLIST\n"); }
    text
}

fn net(replies: Vec<(&'static str, String)>, observer: Arc<Observer>) -> HlsNet {
    HlsNet {
        hs: std::ptr::null_mut(),
        curl: Some(Box::new(Playlists { replies: replies.into(), body: VecDeque::new(), size: 0 })),
        range: Arc::new(NoDial),
        credential_gate: |_, _, _| true,
        observer,
        carry: None,
    }
}

fn open(aq: &mut AuQueue, net: &mut HlsNet, path: &str) -> HlsCursor {
    hls_cursor_open(&Origin::parse("https://192.0.2.10:32400").unwrap(),
        path, aq, net, true, None).unwrap()
}

#[test]
fn a_master_live_playlist_starts_and_reopens_at_the_edge_without_publishing_duration() {
    let observer = Arc::new(Observer::default());
    let mut net = net(vec![
        ("start.m3u8", MASTER.into()), ("media.m3u8", media(12, "", false)),
        ("media.m3u8", media(13, "", false)),
        ("start.m3u8", MASTER.into()), ("media.m3u8", media(20, "", false)),
    ], observer.clone());
    let mut aq = crate::aq::aq_new(1024);
    let mut cursor = open(&mut aq, &mut net, LIVE_ENTRY);
    assert_eq!(cursor.declared_bps, 1_000_000);
    for sequence in 9..13 {
        assert_eq!(hls_cursor_next(&mut cursor, &mut *aq, &mut net, None).unwrap().unwrap().sequence,
            sequence, "initial open must start three seconds behind the edge, then follow growth");
    }
    assert!(cursor.live);
    assert!(net.carry.is_none(), "transcodes do not inherit a tuner's segment state");
    let mut reopened = open(&mut aq, &mut net, LIVE_ENTRY);
    assert_eq!(hls_cursor_next(&mut reopened, &mut *aq, &mut net, None).unwrap().unwrap().sequence, 17);
    assert!(observer.durations.lock().unwrap().is_empty(), "playlist growth is no content duration");
    crate::aq::aq_destroy(&mut *aq);
}

#[test]
fn library_playlists_keep_their_start_offset_and_duration_even_while_growing() {
    for (kind, end_list) in [("", true), ("#EXT-X-PLAYLIST-TYPE:EVENT\n", true),
        ("#EXT-X-PLAYLIST-TYPE:VOD\n", true), ("#EXT-X-PLAYLIST-TYPE:VOD\n", false),
        ("", false)] {
        let observer = Arc::new(Observer::default());
        let text = media(12, &format!("{kind}#EXT-X-START:TIME-OFFSET=2\n"), end_list);
        let mut net = net(vec![("start.m3u8", MASTER.into()), ("media.m3u8", text)], observer.clone());
        let mut aq = crate::aq::aq_new(1024);
        let mut cursor = open(&mut aq, &mut net, LIBRARY_ENTRY);
        assert_eq!(hls_cursor_next(&mut cursor, &mut *aq, &mut net, None).unwrap().unwrap().sequence, 2);
        assert!(!cursor.live);
        assert_eq!(*observer.durations.lock().unwrap(), [12_000_000_000]);
        crate::aq::aq_destroy(&mut *aq);
    }
}

#[test]
fn an_open_event_playlist_uses_the_live_edge() {
    let observer = Arc::new(Observer::default());
    let mut net = net(vec![("start.m3u8", MASTER.into()),
        ("media.m3u8", media(12, "#EXT-X-PLAYLIST-TYPE:EVENT\n", false))], observer.clone());
    let mut aq = crate::aq::aq_new(1024);
    let mut cursor = open(&mut aq, &mut net, LIVE_ENTRY);
    assert_eq!(hls_cursor_next(&mut cursor, &mut *aq, &mut net, None).unwrap().unwrap().sequence, 9);
    assert!(cursor.live);
    assert!(observer.durations.lock().unwrap().is_empty());
    crate::aq::aq_destroy(&mut *aq);
}

#[test]
fn a_live_source_is_named_by_the_transcoder_path_not_by_growth() {
    for live in [LIVE_ENTRY,
        "/start.m3u8?path=/livetv/sessions/9b9fe727-0000-46db-b94c-ef512e3e61d2&X-Plex-Token=fixture"] {
        assert!(is_live_session_source(live), "{live}");
    }
    for library in ["/start.m3u8?path=%2Flibrary%2Fmetadata%2F5&X-Plex-Token=t",
        "/start.m3u8?X-Plex-Token=t", "/start.m3u8", "/start.m3u8?subpath=%2Flivetv%2Fsessions%2Fx"] {
        assert!(!is_live_session_source(library), "{library}");
    }
}

fn decoded_segment() -> HlsSegmentOutput {
    HlsSegmentOutput {
        aus: Vec::new(),
        transfer: SegmentTransfer { bytes: 0, active_us: 1, total_us: 1, audio_expected: false },
        video_width: 1920, video_height: 1080, video_tail_ns: 0, audio_tail_ns: None,
    }
}

#[test]
fn a_live_cursor_skips_unprimed_segments_then_starts_and_keeps_later_failures_strict() {
    let mut net = net(vec![("start.m3u8", MASTER.into()),
        ("media.m3u8", media(12, "", false)), ("media.m3u8", media(13, "", false))],
        Arc::new(Observer::default()));
    let mut aq = crate::aq::aq_new(1024);
    let mut cursor = open(&mut aq, &mut net, LIVE_ENTRY);
    let mut read = Vec::new();
    let result = hls_read_active_with(&mut cursor, &mut net, Default::default(),
        &mut None, &mut *aq, None, |segment, _, _, _, _| {
            read.push(segment.sequence);
            match segment.sequence {
                9 => Err(HlsExit::Failed("HLS segment has invalid video dimensions")),
                10 => Err(HlsExit::Failed("HLS segment does not begin with an IDR")),
                _ => Ok(decoded_segment()),
            }
        });
    let started = result.expect("skip to the first decodable segment").unwrap();
    assert_eq!(read, [9, 10, 11]);
    assert_eq!(started.0.sequence, 11);
    let later = hls_read_active_with(&mut cursor, &mut net, Default::default(),
        &mut None, &mut *aq, None, |_, _, _, _, _| {
            Err(HlsExit::Failed("HLS segment does not begin with an IDR"))
        });
    assert!(matches!(later, Err(HlsExit::Failed("HLS segment does not begin with an IDR"))));
    crate::aq::aq_destroy(&mut *aq);
}

#[test]
fn a_fresh_tuner_cut_primes_on_its_first_valid_idr_and_discards_earlier_audio() {
    let mut start = LiveCutStart::for_tuner_segment(true, false).unwrap();
    assert!(LiveCutStart::for_tuner_segment(true, true).is_none(), "carried cuts already have a decoder anchor");
    assert!(LiveCutStart::for_tuner_segment(false, false).is_none(), "independent/VOD segments stay strict");
    assert!(!start.audio(Some(90)), "audio before video priming is discarded");

    let mut params = H264ParamSets::default();
    let mut out = Vec::new();
    let mut clock = crate::hls::SegmentTimeline::default()
        .begin(std::time::Duration::from_secs(1)).unwrap();
    clock.share_origin();
    let non_idr = [0, 0, 1, 0x41, 4];
    let key = ts_h264_access_unit(&non_idr, &mut params, &mut out).unwrap();
    assert!(!key);
    let first_video = clock.normalize_video(10_000_000_000);
    assert!(!start.video(key, 100), "leading non-IDR access units are not fed");
    assert!(!start.primed());
    let first_audio = clock.normalize_audio(9_980_000_000);
    assert_eq!(first_audio.0, 0, "a discarded audio packet timed before the cut's origin sits on the cut");

    let idr = [0, 0, 1, 0x67, 1, 0, 0, 1, 0x68, 2, 0, 0, 1, 0x65, 3];
    let key = ts_h264_access_unit(&idr, &mut params, &mut out).unwrap();
    assert!(key, "the existing parser requires a real IDR and valid SPS/PPS");
    let idr_at = clock.normalize_video(10_500_000_000);
    assert!(start.video(key, 200));
    assert_eq!(first_video.0, 0);
    assert_eq!(idr_at.0, 500, "the discarded prefix is not rebased into free playable buffer");
    assert_eq!(clock.end().0 - idr_at.0, 500, "only the portion after the IDR contributes reserve");
    assert!(start.primed());
    assert!(!start.audio(Some(199)), "late packet order cannot leak pre-IDR audio");
    assert!(start.audio(Some(200)));
    assert!(start.audio(Some(220)));
    assert!(start.video(false, 240), "later pictures belong to the now-primed stream");
}

#[test]
fn a_fresh_tuner_cut_without_an_idr_remains_unprimed_and_uses_the_existing_error() {
    let mut start = LiveCutStart::for_tuner_segment(true, false).unwrap();
    let mut params = H264ParamSets::default();
    let mut out = Vec::new();
    for pts in [100, 140, 180] {
        let key = ts_h264_access_unit(&[0, 0, 1, 0x41, 4], &mut params, &mut out).unwrap();
        assert!(!key);
        assert!(!start.video(key, pts));
    }
    assert!(!start.primed(), "a segment without a valid IDR cannot start playback");
}

#[test]
fn a_library_cursor_still_refuses_an_unprimed_first_segment() {
    for error in ["HLS segment has invalid video dimensions", "HLS segment does not begin with an IDR"] {
        let mut net = net(vec![("start.m3u8", MASTER.into()), ("media.m3u8", media(12, "", true))],
            Arc::new(Observer::default()));
        let mut aq = crate::aq::aq_new(1024);
        let mut cursor = open(&mut aq, &mut net, LIBRARY_ENTRY);
        let mut reads = 0;
        let result = hls_read_active_with(&mut cursor, &mut net, Default::default(),
            &mut None, &mut *aq, None, |_, _, _, _, _| { reads += 1; Err(HlsExit::Failed(error)) });
        assert!(matches!(result, Err(HlsExit::Failed(e)) if e == error));
        assert_eq!(reads, 1);
        crate::aq::aq_destroy(&mut *aq);
    }
}

#[test]
fn live_start_skipping_is_bounded_and_does_not_hide_other_failures() {
    for (error, expected_reads) in [("HLS segment does not begin with an IDR", 4),
        ("HLS segment has invalid video dimensions", 4), ("HLS segment audio is not AAC", 1)] {
        let mut net = net(vec![("start.m3u8", MASTER.into()),
            ("media.m3u8", media(12, "", false)), ("media.m3u8", media(13, "", false))],
            Arc::new(Observer::default()));
        let mut aq = crate::aq::aq_new(1024);
        let mut cursor = open(&mut aq, &mut net, LIVE_ENTRY);
        let mut reads = 0;
        let result = hls_read_active_with(&mut cursor, &mut net, Default::default(),
            &mut None, &mut *aq, None, |_, _, _, _, _| { reads += 1; Err(HlsExit::Failed(error)) });
        assert!(matches!(result, Err(HlsExit::Failed(e)) if e == error));
        assert_eq!(reads, expected_reads, "{error}");
        crate::aq::aq_destroy(&mut *aq);
    }
}

#[test]
fn carried_live_state_keeps_segment_failures_strict() {
    let mut net = net(vec![("start.m3u8", media(12, "", false)),
        ("start.m3u8", media(12, "", false))], Arc::new(Observer::default()));
    let mut aq = crate::aq::aq_new(1024);
    let mut cursor = open(&mut aq, &mut net, LIVE_ENTRY);
    let segment = hls_cursor_next(&mut cursor, &mut *aq, &mut net, None).unwrap().unwrap();
    net.carry = Some(SegmentCarry { scope: carry_scope(&segment).to_owned(),
        raster: (1920, 1080), ..Default::default() });
    assert!(!cursor.skip_live_start(&segment, &net, HlsExit::Failed(HLS_MISSING_IDR)));
    assert_eq!(cursor.live_start_skips_left, LIVE_START_SKIP_SEGMENTS);
    crate::aq::aq_destroy(&mut *aq);
}

/// **A live playlist that stops advancing fails the read instead of freezing the picture.** The
/// maintainer's webOS 4.5 run (PR #993): a Threadfin channel's playlist kept answering with the
/// same segments and no end marker from 4:19 on; the reader re-read it 53 times and would have
/// kept going, the picture frozen and nothing said. Past the stall limit the read fails with
/// [`HLS_LIVE_STALLED`], which is named to the player as a stall and reads out: the recording has no
/// input, so it is not converted ([`a_stalled_live_read_is_named_before_its_failure_flag`]).
#[test]
fn a_live_playlist_that_stops_advancing_fails_the_read_instead_of_polling_forever() {
    let mut net = net(vec![("start.m3u8", MASTER.into()), ("media.m3u8", media(12, "", false))],
        Arc::new(Observer::default()));
    let mut aq = crate::aq::aq_new(1024);
    let mut cursor = open(&mut aq, &mut net, LIVE_ENTRY);
    for sequence in 9..12 {
        assert_eq!(hls_cursor_next(&mut cursor, &mut *aq, &mut net, None).unwrap().unwrap().sequence, sequence);
    }
    // The playlist answers unchanged from here on; the clock says it has for longer than the limit.
    let unchanged = (0..4).map(|_| ("media.m3u8", media(12, "", false)));
    net.curl = Some(Box::new(Playlists { replies: unchanged.collect(), body: VecDeque::new(), size: 0 }));
    cursor.live_unchanged_since = Some(std::time::Instant::now()
        - live_stall_limit(cursor.target_duration_secs) - std::time::Duration::from_secs(1));
    let read = hls_cursor_next(&mut cursor, &mut *aq, &mut net, None);
    assert!(matches!(read, Err(HlsExit::Failed(HLS_LIVE_STALLED))), "{read:?}");
    let timeline = crate::hls::SegmentTimeline::default();
    let n_clock = timeline.begin(std::time::Duration::from_secs(2)).unwrap();
    let lookahead = unsafe { hls_prefetch_same_encoder(
        &mut cursor, &mut *aq, &mut net, "aac", timeline, n_clock, None,
    ) };
    assert!(matches!(lookahead, Err(HlsExit::Failed(HLS_LIVE_STALLED))),
        "fixed HLS lookahead must pass the terminal stall to recovery");
    crate::aq::aq_destroy(&mut *aq);
}

/// **A 30 s gap in the recording's input is waited out on the same session.** The maintainer's webOS
/// 4.5 runs (PR #993 review): on two Pluto channels Plex waited 29.8 s and 30.3 s for DVR input and
/// then produced segments again on the same encoder, 14-15 s after the 15 s floor had failed the
/// read. The cursor keeps polling the one playlist it opened (the fake refuses any other request, so
/// a reopen or a second session would panic here), and delivers the segment the gap ended with.
#[test]
fn a_live_playlist_unchanged_for_thirty_seconds_resumes_on_the_same_session() {
    let observer = Arc::new(Observer::default());
    let mut net = net(vec![("start.m3u8", MASTER.into()), ("media.m3u8", media(12, "", false))], observer.clone());
    let mut aq = crate::aq::aq_new(1024);
    let mut cursor = open(&mut aq, &mut net, LIVE_ENTRY);
    for sequence in 9..12 {
        assert_eq!(hls_cursor_next(&mut cursor, &mut *aq, &mut net, None).unwrap().unwrap().sequence, sequence);
    }
    // Unchanged for 30 s already, unchanged once more, then the recording's input is back.
    net.curl = Some(Box::new(Playlists { replies: vec![("media.m3u8", media(12, "", false)),
        ("media.m3u8", media(13, "", false))].into(), body: VecDeque::new(), size: 0 }));
    cursor.live_unchanged_since = Some(std::time::Instant::now() - std::time::Duration::from_secs(30));
    let read = hls_cursor_next(&mut cursor, &mut *aq, &mut net, None);
    assert_eq!(read.expect("a 30 s gap is not a failure").expect("nor an end").sequence, 12);
    assert_eq!(cursor.live_unchanged_since, None, "growth clears the stall clock");
    assert!(!cursor.ended);
    crate::aq::aq_destroy(&mut *aq);
}

/// **A stall is named before the flag that ends the session**, so the pump reads both from one
/// sample and does not take it for a decode failure (which converts the channel). Every other
/// failure keeps the two flags it had: the transport's once anything was delivered, the open's before.
#[test]
fn a_stalled_live_read_is_named_before_its_failure_flag() {
    let effects = |why, pushed| hls_failure_effects(why, pushed).collect::<Vec<_>>();
    assert!(matches!(effects(HLS_LIVE_STALLED, true)[..], [DemuxEffect::LiveStalled, DemuxEffect::IoFailed]));
    assert!(matches!(effects(HLS_LIVE_STALLED, false)[..],
        [DemuxEffect::LiveStalled, DemuxEffect::FailFault(DemuxFault::HlsFailed)]));
    assert!(matches!(effects(HLS_MISSING_IDR, true)[..], [DemuxEffect::IoFailed]));
    assert!(matches!(effects(H264_IDR_WITHOUT_SPS, false)[..], [DemuxEffect::FailFault(DemuxFault::HlsFailed)]));
}

#[test]
fn the_live_stall_clock_starts_on_the_first_empty_read_and_growth_clears_it() {
    let limit = live_stall_limit(1);
    // Was 15 s and 30 s: the floor is now the provisional 45 s (PR #993 review), and a longer
    // target duration keeps its three-target-durations allowance.
    assert_eq!(limit, std::time::Duration::from_secs(45), "a short target duration keeps the 45 s floor");
    assert_eq!(live_stall_limit(10), std::time::Duration::from_secs(45));
    assert_eq!(live_stall_limit(20), std::time::Duration::from_secs(60), "three target durations past the floor");
    let t0 = std::time::Instant::now();
    let mut since = None;
    assert!(!live_playlist_stalled(&mut since, 0, t0, limit), "the first empty read only starts the clock");
    assert_eq!(since, Some(t0));
    assert!(!live_playlist_stalled(&mut since, 0, t0 + limit, limit), "exactly the limit is still waiting");
    assert!(live_playlist_stalled(&mut since, 0, t0 + limit + std::time::Duration::from_millis(1), limit));
    assert!(!live_playlist_stalled(&mut since, 2, t0 + limit * 2, limit), "a new segment is progress");
    assert_eq!(since, None);
}

/// A cut whose first IDR arrives without its parameter sets (the PPS, or the SPS, was in the
/// part of the recording before the cut) is the same undecodable start as one with no IDR at all:
/// skipped inside the same bounded budget, not the end of the startup read (PR #993 review).
#[test]
fn a_live_start_skips_a_cut_whose_idr_has_no_parameter_sets() {
    let mut params = H264ParamSets::default();
    let mut out = Vec::new();
    let no_sps = ts_h264_access_unit(&[0, 0, 1, 0x68, 2, 0, 0, 1, 0x65, 3], &mut params, &mut out).unwrap_err();
    let mut params = H264ParamSets::default();
    let no_pps = ts_h264_access_unit(&[0, 0, 1, 0x67, 1, 0, 0, 1, 0x65, 3], &mut params, &mut out).unwrap_err();
    assert_ne!(no_sps, no_pps);

    let mut net = net(vec![("start.m3u8", MASTER.into()),
        ("media.m3u8", media(12, "", false)), ("media.m3u8", media(13, "", false))],
        Arc::new(Observer::default()));
    let mut aq = crate::aq::aq_new(1024);
    let mut cursor = open(&mut aq, &mut net, LIVE_ENTRY);
    let mut read = Vec::new();
    let result = hls_read_active_with(&mut cursor, &mut net, Default::default(),
        &mut None, &mut *aq, None, |segment, _, _, _, _| {
            read.push(segment.sequence);
            match segment.sequence {
                9 => Err(HlsExit::Failed(no_pps)),
                10 => Err(HlsExit::Failed(no_sps)),
                _ => Ok(decoded_segment()),
            }
        });
    assert_eq!(result.expect("skip to the first decodable segment").unwrap().0.sequence, 11);
    assert_eq!(read, [9, 10, 11]);
    crate::aq::aq_destroy(&mut *aq);

    // Bounded by the same budget as the other undecodable starts, and a library cursor stays strict.
    for (entry, expected_reads) in [(LIVE_ENTRY, 4), (LIBRARY_ENTRY, 1)] {
        let mut net = self::net(vec![("start.m3u8", MASTER.into()),
            ("media.m3u8", media(12, "", entry == LIBRARY_ENTRY)), ("media.m3u8", media(13, "", false))],
            Arc::new(Observer::default()));
        let mut aq = crate::aq::aq_new(1024);
        let mut cursor = open(&mut aq, &mut net, entry);
        let mut reads = 0;
        let result = hls_read_active_with(&mut cursor, &mut net, Default::default(),
            &mut None, &mut *aq, None, |_, _, _, _, _| { reads += 1; Err(HlsExit::Failed(no_pps)) });
        assert!(matches!(result, Err(HlsExit::Failed(e)) if e == no_pps));
        assert_eq!(reads, expected_reads, "{entry}");
        crate::aq::aq_destroy(&mut *aq);
    }
}

/// A recording that ended with less than the start cushion in its playlist can never grow into
/// it: its ENDLIST bypasses the hold, what it lists is delivered, then the read ends (PR #993
/// review). It used to hold for the cushion on every read and poll for ever.
#[test]
fn an_ended_live_playlist_shorter_than_the_cushion_drains_and_ends() {
    let mut net = net(vec![("start.m3u8", media(2, "", true)), ("start.m3u8", media(2, "", true))],
        Arc::new(Observer::default()));
    let mut aq = crate::aq::aq_new(1024);
    let mut cursor = open(&mut aq, &mut net, LIVE_ENTRY);
    for sequence in 0..2 {
        assert_eq!(hls_cursor_next(&mut cursor, &mut *aq, &mut net, None).unwrap().unwrap().sequence, sequence);
    }
    assert!(hls_cursor_next(&mut cursor, &mut *aq, &mut net, None).unwrap().is_none(), "end of stream");
    crate::aq::aq_destroy(&mut *aq);
}

/// The stall rule covers the hold as well: a playlist frozen below the cushion with no end marker
/// fails the read like one frozen after the start, instead of holding for a cushion that never comes.
#[test]
fn a_live_playlist_frozen_below_its_cushion_fails_the_read() {
    let frozen = || ("start.m3u8", media(2, "", false));
    let mut net = net(vec![frozen(), frozen(), frozen(), frozen()], Arc::new(Observer::default()));
    let mut aq = crate::aq::aq_new(1024);
    let mut cursor = open(&mut aq, &mut net, LIVE_ENTRY);
    assert!(cursor.pending.is_empty());
    // The playlist held these two segments at the last read, longer ago than the limit.
    cursor.live_held_next = 2;
    cursor.live_unchanged_since = Some(std::time::Instant::now()
        - live_stall_limit(cursor.target_duration_secs) - std::time::Duration::from_secs(1));
    let read = hls_cursor_next(&mut cursor, &mut *aq, &mut net, None);
    assert!(matches!(read, Err(HlsExit::Failed(HLS_LIVE_STALLED))), "{read:?}");
    crate::aq::aq_destroy(&mut *aq);
}

/// **A conversion's playlist frozen below the cushion fails the read too.** Only a tuner's own
/// playlist was judged during the hold, so a transcoder playlist that stopped below the cushion with
/// no end marker was polled every 500 ms for ever (PR #993 review). It gets a longer limit, because
/// its clock runs from before the encoder's first segment: past the tuner's limit it still waits.
#[test]
fn a_conversion_playlist_frozen_below_its_cushion_fails_the_read_after_a_longer_wait() {
    let tuner = live_hold_stall_limit(true, 1);
    let conversion = live_hold_stall_limit(false, 1);
    assert_eq!(tuner, live_stall_limit(1));
    assert_eq!(conversion, std::time::Duration::from_secs(60));
    // The hold is never shorter than it was under the 15 s floor, and always leaves the encoder's
    // start 15 s past the in-play allowance.
    for (target, was) in [(1, 60), (2, 60), (6, 72), (10, 120), (20, 240)] {
        let hold = live_hold_stall_limit(false, target);
        assert_eq!(hold, std::time::Duration::from_secs(was), "target {target}");
        assert!(hold >= live_stall_limit(target) + std::time::Duration::from_secs(15), "target {target}");
        assert!(live_hold_stall_limit(true, target) >= std::time::Duration::from_secs(45), "target {target}");
    }
    let second = std::time::Duration::from_secs(1);
    let frozen = || ("media.m3u8", media(2, "", false));

    let mut net = net(vec![("start.m3u8", MASTER.into()), frozen()], Arc::new(Observer::default()));
    let mut aq = crate::aq::aq_new(1024);
    let mut cursor = open(&mut aq, &mut net, LIVE_ENTRY);
    assert!(cursor.live && net.carry.is_none(), "a conversion: live, and no tuner carry");
    cursor.live_held_next = 2;
    cursor.live_unchanged_since = Some(std::time::Instant::now() - conversion - second);
    let read = hls_cursor_next(&mut cursor, &mut *aq, &mut net, None);
    assert!(matches!(read, Err(HlsExit::Failed(HLS_LIVE_STALLED))), "{read:?}");
    crate::aq::aq_destroy(&mut *aq);

    // Past the tuner's limit a slow encoder is still waited for, and growth starts the read.
    let mut net = self::net(vec![("start.m3u8", MASTER.into()), frozen(), ("media.m3u8", media(4, "", false))],
        Arc::new(Observer::default()));
    let mut aq = crate::aq::aq_new(1024);
    let mut cursor = open(&mut aq, &mut net, LIVE_ENTRY);
    cursor.live_held_next = 2;
    cursor.live_unchanged_since = Some(std::time::Instant::now() - tuner - second);
    assert_eq!(hls_cursor_next(&mut cursor, &mut *aq, &mut net, None).unwrap().unwrap().sequence, 1);
    assert_eq!(cursor.live_unchanged_since, None, "growth cleared the clock");
    crate::aq::aq_destroy(&mut *aq);
}

/// A tune's playlist read before the recording cut its first segment has the media tags and no
/// EXTINF. It is the session's media playlist, not a master with no variant: the cursor opens on
/// it and starts once the cushion exists (PR #993 review).
#[test]
fn a_live_entry_with_no_segment_yet_opens_as_media_and_starts_when_it_grows() {
    let mut net = net(vec![("start.m3u8", media(0, "", false)), ("start.m3u8", media(0, "", false)),
        ("start.m3u8", media(4, "", false))], Arc::new(Observer::default()));
    let mut aq = crate::aq::aq_new(1024);
    let mut cursor = open(&mut aq, &mut net, LIVE_ENTRY);
    assert!(cursor.live && net.carry.is_some(), "a tuner's own playlist");
    assert_eq!(hls_cursor_next(&mut cursor, &mut *aq, &mut net, None).unwrap().unwrap().sequence, 1,
        "three seconds behind the edge of the first read that holds the cushion");
    crate::aq::aq_destroy(&mut *aq);
}
