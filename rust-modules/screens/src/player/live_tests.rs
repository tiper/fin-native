//! **The live player's keys, zap, art ground and banner** (Live TV PR4), graded through `PlayerScreen::step` the way
//! the dispatcher drives it: a key is an `Input` event, a frame is a `Tick`, and what the page asks of the app is the
//! `PlayerReq`s it pushes. The session is the only collaborator, owned by the case.
use super::*;
use crate::player::overlay_tests::{derive_state, set_lineup, TestHost};
use plx_data::livetv::Channel;
use plx_machine::machine::{Cx, Edge, Effects, Fx, Handled, InputEvent, InputKind, InputOwner, MachineId, Source, Tick};
use plx_media::player::PlaybackState;
use plx_ui::consts::{
    SDLK_DOWN, SDLK_LEFT, SDLK_PAGEDOWN, SDLK_PAGEUP, SDLK_RETURN, SDLK_RIGHT, SDLK_UP, WCODE_BACK, WCODE_CH_DOWN_KEY,
    WCODE_CH_UP_KEY, WCODE_FASTFORWARD, WCODE_REWIND, WCODE_STOP,
};
use plx_ui::fixture::FixtureMeasure;
use zap::{Art, Banner, Zap};

const ENTRY: EntryId = EntryId(9);
const INST: InstanceId = InstanceId(3);

fn ch(n: u32) -> Channel {
    Channel {
        sid: plx_plex::plex::ServerId::from_raw(0),
        dvr: "7".into(),
        channel: format!("C{n}"),
        vcn: n.to_string(),
        name: format!("Station {n}"),
        thumb: String::new(),
        now: format!("Show {n}"),
        now_thumb: format!("/art/{n}"),
        now_summary: String::new(),
        begins_at: 0,
        ends_at: 3_600,
    }
}

/// A live page over a four-channel lineup (101..104) playing `n`, in `state`.
fn live(n: u32, state: PlaybackState) -> (plx_media::route::PlaybackSession, PlayerScreen) {
    set_lineup((101..=104).map(ch).collect());
    let mut ps = plx_media::route::PlaybackSession::IDLE;
    plx_media::route::mark_live_channel_for_test(&mut ps, ch(n));
    derive_state(&mut ps, state);
    (ps, PlayerScreen::new(ENTRY, plx_appkit::player_hud::loaded::classic_for_test()))
}

fn cx_over(ps: &plx_media::route::PlaybackSession, ms: u32) -> Cx<'_, TestHost> {
    Cx {
        views: ps,
        tick: Tick { ms, dt_us: 16_000 },
        measure: &FixtureMeasure,
        focus: Default::default(),
        press: Default::default(),
        modal: Default::default(), page_image: Default::default(), hold_hint: Default::default(),
        owner: InputOwner::Entry(ENTRY),
    }
}

fn run(ps: &plx_media::route::PlaybackSession, page: &mut PlayerScreen, ms: u32, ev: ScreenEvent<TestHost>) -> (Handled, Vec<PlayerReq>) {
    crate::player::overlay_tests::publish_presentation_fixture(crate::player::overlay_tests::presentation_fixture(ps));
    let mut out = Vec::new();
    let mut present = plx_machine::present::Present::new();
    let handled = page.step(&ev, &cx_over(ps, ms), &mut Effects::new(&mut out, MachineId::Instance(INST), &mut present));
    let reqs = out.into_iter().filter_map(|e| match e.fx { Fx::App(AppFx::Player(req)) => Some(req), _ => None }).collect();
    (handled, reqs)
}

/// A key press of the given edge at `ms`.
fn key(ps: &plx_media::route::PlaybackSession, page: &mut PlayerScreen, sym: u32, wcode: u32, edge: Edge, ms: u32) -> (Handled, Vec<PlayerReq>) {
    let ev = ScreenEvent::<TestHost>::Input(InputEvent {
        kind: InputKind::Key { key: plx_machine::machine::Key::Other, sym, wcode, edge, at_edge: false },
        at: Tick { ms, dt_us: 0 },
        source: Source::Sdl,
    });
    run(ps, page, ms, ev)
}

fn down(ps: &plx_media::route::PlaybackSession, page: &mut PlayerScreen, sym: u32, wcode: u32, ms: u32) -> (Handled, Vec<PlayerReq>) {
    key(ps, page, sym, wcode, Edge::Down, ms)
}

fn tick(ps: &plx_media::route::PlaybackSession, page: &mut PlayerScreen, ms: u32) -> Vec<PlayerReq> {
    run(ps, page, ms, ScreenEvent::Tick(Tick { ms, dt_us: 16_000 })).1
}

/// Frames from `from` to `to` (inclusive, 16 ms apart), returning every request they asked.
fn frames(ps: &plx_media::route::PlaybackSession, page: &mut PlayerScreen, from: u32, to: u32) -> Vec<PlayerReq> {
    (from..=to).step_by(16).flat_map(|ms| tick(ps, page, ms)).collect()
}

/// Frames from `from` until one asks for something, returning the time of that frame and what it asked. The app
/// performs a `Tune` before the next frame (it swaps the session to the target), so a case that goes on from it
/// hands the page the landed session itself.
fn until_asked(ps: &plx_media::route::PlaybackSession, page: &mut PlayerScreen, from: u32) -> (u32, Vec<PlayerReq>) {
    (from..from + 5_000).step_by(16).find_map(|ms| Some(tick(ps, page, ms)).filter(|r| !r.is_empty()).map(|r| (ms, r))).expect("a request")
}

fn tune_of(n: u32) -> PlayerReq {
    let c = ch(n);
    PlayerReq::Tune { sid: c.sid, dvr: c.dvr, channel: c.channel, title: c.name }
}

fn shown(ps: &plx_media::route::PlaybackSession, page: &PlayerScreen) -> Option<String> {
    page.live_shown(ps, <TestHost as PlayerLike>::lineup(&cx_over(ps, 0))).map(|c| c.vcn.clone())
}

/// The banner names the channel being stepped to at every press, before the stream has been asked for anything.
#[test]
fn the_banner_follows_each_press_before_any_tune() {
    let _g = plx_base::testlock::serial();
    let (ps, mut page) = live(101, PlaybackState::Playing);
    frames(&ps, &mut page, 0, 64);
    assert_eq!(shown(&ps, &page).as_deref(), Some("101"));
    for (ms, sym, want) in [(1_000, SDLK_UP, "102"), (1_100, SDLK_UP, "103"), (1_200, SDLK_DOWN, "102")] {
        let (handled, reqs) = down(&ps, &mut page, sym, 0, ms);
        assert_eq!((handled, reqs), (Handled::Yes, Vec::new()), "a press asks the app for nothing at once");
        assert_eq!(shown(&ps, &page).as_deref(), Some(want), "the banner follows the press");
        assert_eq!(page.live.banner_alpha(ms), 1.0);
    }
    assert!(matches!(page.live.zap, Zap::Pending { .. }), "still inside the window");
}

/// Up and CH+ are the next higher number, Down and CH- the next lower, whichever key spelled them.
#[test]
fn up_and_ch_plus_step_higher_down_and_ch_minus_lower() {
    let _g = plx_base::testlock::serial();
    for (sym, wcode, want) in [
        (SDLK_UP, 0, "102"), (SDLK_PAGEUP, 0, "102"), (0, WCODE_CH_UP_KEY, "102"),
        (SDLK_DOWN, 0, "104"), (SDLK_PAGEDOWN, 0, "104"), (0, WCODE_CH_DOWN_KEY, "104"),
    ] {
        let (ps, mut page) = live(101, PlaybackState::Playing);
        let (handled, reqs) = down(&ps, &mut page, sym, wcode, 1_000);
        assert_eq!((handled, reqs), (Handled::Yes, Vec::new()), "sym {sym} wcode {wcode}");
        assert_eq!(shown(&ps, &page).as_deref(), Some(want), "sym {sym} wcode {wcode}");
        // a held key or its release moves nothing further
        for edge in [Edge::Repeat, Edge::Up] {
            assert_eq!(key(&ps, &mut page, sym, wcode, edge, 1_050), (Handled::Yes, Vec::new()));
            assert_eq!(shown(&ps, &page).as_deref(), Some(want));
        }
    }
}

/// A run of presses is one tune: the third target, asked once, 600 ms after the last press.
#[test]
fn three_presses_inside_600_ms_give_one_tune_for_the_third_target() {
    let _g = plx_base::testlock::serial();
    let (ps, mut page) = live(101, PlaybackState::Playing);
    frames(&ps, &mut page, 0, 64);
    let mut asked = Vec::new();
    for ms in [1_000, 1_200, 1_400] {
        asked.extend(down(&ps, &mut page, SDLK_UP, 0, ms).1);
        asked.extend(frames(&ps, &mut page, ms, ms + 560));
    }
    assert_eq!(asked, Vec::<PlayerReq>::new(), "nothing is asked while presses keep arriving inside the window");
    asked.extend(frames(&ps, &mut page, 1_400 + 560, 2_400));
    assert_eq!(asked, vec![tune_of(104)], "one Tune, for the third target");
}

/// The window is not one-shot: a press after the tune was asked opens a new one from the tuned target.
#[test]
fn a_press_after_the_tune_starts_a_new_window() {
    let _g = plx_base::testlock::serial();
    let (ps, mut page) = live(101, PlaybackState::Playing);
    frames(&ps, &mut page, 0, 64);
    down(&ps, &mut page, SDLK_UP, 0, 1_000);
    let (asked_at, reqs) = until_asked(&ps, &mut page, 1_000);
    assert_eq!((reqs, asked_at >= 1_600), (vec![tune_of(102)], true));
    assert!(matches!(page.live.zap, Zap::Tuning { .. }));
    // the session has not landed on 102 yet: the next press steps from the TARGET, and a new window opens
    down(&ps, &mut page, SDLK_UP, 0, asked_at + 8);
    assert_eq!(shown(&ps, &page).as_deref(), Some("103"));
    assert!(matches!(page.live.zap, Zap::Pending { .. }));
    let (asked_again, reqs) = until_asked(&ps, &mut page, asked_at + 16);
    assert_eq!((reqs, asked_again >= asked_at + 8 + 600), (vec![tune_of(103)], true));
}

/// The art stands from the first press through the debounce, and in Buffering; Playing with nothing pressed has none.
#[test]
fn the_art_is_drawn_in_buffering_and_through_the_debounce_but_not_while_playing() {
    let _g = plx_base::testlock::serial();
    // Buffering: loaded, no frame yet
    let (ps, mut page) = live(101, PlaybackState::Buffering);
    tick(&ps, &mut page, 0);
    assert_eq!((page.live.art, page.live.art_alpha(0)), (Art::Covering, 1.0));
    // Playing: the first frame is up, the art fades and is gone
    let (ps, mut page) = live(101, PlaybackState::Playing);
    frames(&ps, &mut page, 0, 16);
    assert_eq!(page.live.art, Art::Settled, "a channel already showing a frame never draws the art");
    assert_eq!(page.live.art_alpha(16), 0.0);
    // A press while Playing covers at once, before the Tune is asked, and holds through the window
    down(&ps, &mut page, SDLK_UP, 0, 1_000);
    assert_eq!((page.live.art, page.live.art_alpha(1_000)), (Art::Covering, 1.0));
    assert_eq!(frames(&ps, &mut page, 1_000, 1_560), Vec::<PlayerReq>::new());
    assert_eq!(page.live.art_alpha(1_560), 1.0, "still covering during the debounce");
    let (asked_at, reqs) = until_asked(&ps, &mut page, 1_576);
    assert_eq!(reqs, vec![tune_of(102)]);
    // the new channel is loaded and buffering: no frame, the art stays whole
    let (buffering, _) = live(102, PlaybackState::Buffering);
    frames(&buffering, &mut page, asked_at + 16, asked_at + 800);
    assert_eq!((page.live.art, page.live.art_alpha(asked_at + 800)), (Art::Covering, 1.0));
    // its first frame is up: the art fades out and is gone
    let (landed, _) = live(102, PlaybackState::Playing);
    frames(&landed, &mut page, asked_at + 816, asked_at + 900);
    assert!(matches!(page.live.art, Art::Fading { .. } | Art::Settled), "{:?}", page.live.art);
    frames(&landed, &mut page, asked_at + 916, asked_at + 1_500);
    assert_eq!(page.live.art, Art::Settled);
    assert!(page.live.zap.is_idle());
}

/// **A rebuffer is not a start-up.** A channel that has shown its frame and then stalls (the session reports Buffering
/// again) keeps the frozen picture: the art does not come back over it and the banner is not raised a second time.
#[test]
fn a_stall_after_the_first_frame_draws_no_art_and_no_banner() {
    let _g = plx_base::testlock::serial();
    let (mut ps, mut page) = live(101, PlaybackState::Playing);
    frames(&ps, &mut page, 0, 64);
    let late = 20_000;
    frames(&ps, &mut page, late - 32, late);
    assert_eq!((page.live.art, page.live.banner_alpha(late)), (Art::Settled, 0.0), "settled and quiet");
    derive_state(&mut ps, PlaybackState::Buffering);
    frames(&ps, &mut page, late + 16, late + 800);
    assert_eq!((page.live.art, page.live.art_alpha(late + 800)), (Art::Settled, 0.0), "the stall draws no art");
    derive_state(&mut ps, PlaybackState::Playing);
    frames(&ps, &mut page, late + 816, late + 1_200);
    assert_eq!(page.live.banner_alpha(late + 1_200), 0.0, "the stall's end raises no banner");
}

/// **A single BACK leaves, whatever the phase** (seen on the television: the first BACK after a tune did not).
#[test]
fn a_single_back_leaves_at_every_phase() {
    let _g = plx_base::testlock::serial();
    for wcode in [WCODE_BACK, WCODE_STOP] {
        // buffering, the transport up from the tune (the old path hid it on this first press)
        let (ps, mut page) = live(101, PlaybackState::Buffering);
        tick(&ps, &mut page, 0);
        page.hud.extend(0, input::HUD_LINGER_MS);
        assert_eq!(down(&ps, &mut page, 0, wcode, 100), (Handled::Yes, vec![PlayerReq::Exit]), "buffering, wcode {wcode}");
        // playing
        let (ps, mut page) = live(101, PlaybackState::Playing);
        frames(&ps, &mut page, 0, 64);
        assert_eq!(down(&ps, &mut page, 0, wcode, 100), (Handled::Yes, vec![PlayerReq::Exit]), "playing, wcode {wcode}");
        // the debounce
        down(&ps, &mut page, SDLK_UP, 0, 1_000);
        assert_eq!(down(&ps, &mut page, 0, wcode, 1_100), (Handled::Yes, vec![PlayerReq::Exit]), "debounce, wcode {wcode}");
        // …and the debounce the BACK interrupted closes on nothing: no tune follows the exit
        assert_eq!(frames(&ps, &mut page, 1_116, 2_500), Vec::<PlayerReq>::new(), "no tune after a BACK, wcode {wcode}");
        assert!(page.live.zap.is_idle());
        // the tune asked, not landed
        let (ps, mut page) = live(101, PlaybackState::Playing);
        frames(&ps, &mut page, 0, 64);
        down(&ps, &mut page, SDLK_UP, 0, 1_000);
        let (asked_at, _) = until_asked(&ps, &mut page, 1_016);
        assert!(matches!(page.live.zap, Zap::Tuning { .. }));
        assert_eq!(down(&ps, &mut page, 0, wcode, asked_at + 8), (Handled::Yes, vec![PlayerReq::Exit]), "tuning, wcode {wcode}");
        // …and the release of that press is swallowed, not a second exit
        assert_eq!(key(&ps, &mut page, 0, wcode, Edge::Up, asked_at + 24), (Handled::Yes, Vec::new()));
    }
}

/// Left, Right and OK only raise the banner: no HUD, no section, no request, no zap.
#[test]
fn left_right_and_ok_show_the_banner_and_open_nothing() {
    let _g = plx_base::testlock::serial();
    for (sym, wcode) in [(SDLK_LEFT, 0), (SDLK_RIGHT, 0), (SDLK_RETURN, 0), (0, WCODE_FASTFORWARD), (0, WCODE_REWIND)] {
        let (ps, mut page) = live(101, PlaybackState::Playing);
        frames(&ps, &mut page, 0, 64);
        // long after the banner's time is up and it has faded
        let late = 20_000;
        frames(&ps, &mut page, late - 32, late);
        assert_eq!(page.live.banner_alpha(late), 0.0, "faded");
        let (handled, reqs) = down(&ps, &mut page, sym, wcode, late + 16);
        assert_eq!((handled, reqs), (Handled::Yes, Vec::new()), "sym {sym} wcode {wcode}");
        assert_eq!(page.live.banner_alpha(late + 16), 1.0, "sym {sym} wcode {wcode}");
        assert!(!page.hud.sections.any() && page.live.zap.is_idle(), "sym {sym} wcode {wcode}");
        assert!(!page.hud_drawn(&ps, late + 16), "a live channel draws no transport");
        // 4 s on it is going, and after the fade it is gone
        assert_eq!(page.live.banner_alpha(late + 16 + 4_000), 1.0);
        assert!(page.live.banner_alpha(late + 16 + 4_175) < 1.0);
        assert_eq!(page.live.banner_alpha(late + 16 + 4_350), 0.0);
    }
}

/// The banner rises on every tune, not only on a press: with the page, and on each stream the app starts (`PlayerStart`).
#[test]
fn the_banner_rises_on_every_tune() {
    let _g = plx_base::testlock::serial();
    let (ps, mut page) = live(101, PlaybackState::Playing);
    assert_eq!(page.live.banner, Banner::Entering, "raised with the page, before the screen has a clock");
    tick(&ps, &mut page, 100);
    assert_eq!(page.live.banner, Banner::Shown { until: 100 + plx_appkit::player_hud::live::SHOW_MS });
    // the app starts another stream later: its word, not a poll of the session, raises the banner
    let (other, _) = live(103, PlaybackState::Playing);
    tick(&other, &mut page, 9_000);
    assert_eq!(page.live.banner, Banner::Shown { until: 100 + plx_appkit::player_hud::live::SHOW_MS }, "a channel change nobody announced raises nothing");
    run(&other, &mut page, 9_016, ScreenEvent::App(AppMsg::PlayerStart { now: 9_016, hud_ms: input::HUD_LINGER_MS }));
    assert_eq!(page.live.banner, Banner::Shown { until: 9_016 + plx_appkit::player_hud::live::SHOW_MS });
}

/// **Up then Down inside the window is no zap at all**: the target is back on the channel playing, so the window
/// closes on nothing, and the session is not stopped and restarted.
#[test]
fn up_then_down_inside_the_window_asks_for_no_tune() {
    let _g = plx_base::testlock::serial();
    let (ps, mut page) = live(101, PlaybackState::Playing);
    frames(&ps, &mut page, 0, 64);
    down(&ps, &mut page, SDLK_UP, 0, 1_000);
    down(&ps, &mut page, SDLK_DOWN, 0, 1_100);
    assert_eq!(shown(&ps, &page).as_deref(), Some("101"), "the banner is back on the channel that plays");
    assert_eq!(frames(&ps, &mut page, 1_116, 2_500), Vec::<PlayerReq>::new(), "no tune for the channel already playing");
    assert!(page.live.zap.is_idle());
}

/// A tune the app took no request for ends the zap on the app's word (`AppMsg::LiveTuneRefused`), whatever the timing.
#[test]
fn a_refused_tune_ends_the_zap_on_the_apps_word() {
    let _g = plx_base::testlock::serial();
    let (ps, mut page) = live(101, PlaybackState::Playing);
    frames(&ps, &mut page, 0, 64);
    down(&ps, &mut page, SDLK_UP, 0, 1_000);
    let (asked_at, reqs) = until_asked(&ps, &mut page, 1_016);
    assert_eq!(reqs, vec![tune_of(102)]);
    // the session still plays 101 with a frame, frame after frame: still tuning
    frames(&ps, &mut page, asked_at + 16, asked_at + 400);
    assert!(matches!(page.live.zap, Zap::Tuning { .. }), "no inference from timing");
    run(&ps, &mut page, asked_at + 416, ScreenEvent::App(AppMsg::LiveTuneRefused));
    assert!(page.live.zap.is_idle());
}

/// What the live page does not take stays the page's: the transport's play/pause.
#[test]
fn play_pause_keeps_its_meaning_on_a_live_channel() {
    let _g = plx_base::testlock::serial();
    let (ps, mut page) = live(101, PlaybackState::Playing);
    frames(&ps, &mut page, 0, 64);
    let (handled, reqs) = down(&ps, &mut page, 0, plx_ui::consts::WCODE_PLAYPAUSE, 1_100);
    assert_eq!(handled, Handled::Yes);
    assert!(!reqs.is_empty() && !reqs.contains(&PlayerReq::Exit), "{reqs:?}");
}

/// The request that opens number entry over channel `n`, which is the one on screen.
fn open_entry(n: u32, first: u8) -> PlayerReq {
    let c = ch(n);
    PlayerReq::OpenChannelEntry { first, playing: Some((c.sid, c.dvr, c.channel)) }
}

fn picked(n: u32) -> ScreenEvent<TestHost> {
    let c = ch(n);
    ScreenEvent::App(AppMsg::LiveChannelPicked { sid: c.sid, dvr: c.dvr, channel: c.channel })
}

/// **A digit over a live channel opens number entry** (Live TV PR5): one request naming the digit and the channel on
/// screen, nothing tuned, no banner. The digit's own kinds (the keypad's `sym` or the remote's `wcode`) both count, and
/// a held digit or its release asks for nothing more.
#[test]
fn a_digit_opens_number_entry_over_a_live_channel() {
    let _g = plx_base::testlock::serial();
    for (sym, wcode, digit) in [(53, 0, 5), (0, 53, 5), (48, 0, 0), (57, 0, 9)] {
        let (ps, mut page) = live(101, PlaybackState::Playing);
        frames(&ps, &mut page, 0, 64);
        let (handled, reqs) = down(&ps, &mut page, sym, wcode, 1_000);
        assert_eq!((handled, reqs), (Handled::Yes, vec![open_entry(101, digit)]), "sym {sym} wcode {wcode}");
        assert!(page.live.zap.is_idle());
        for edge in [Edge::Repeat, Edge::Up] {
            assert_eq!(key(&ps, &mut page, sym, wcode, edge, 1_050), (Handled::Yes, Vec::new()), "{edge:?}");
        }
    }
}

/// **A pending zap is cancelled when number entry opens**: a run of presses whose window had not closed must not
/// tune under the panel, and the entry's own pick is the only tune that follows.
#[test]
fn opening_number_entry_cancels_a_pending_zap() {
    let _g = plx_base::testlock::serial();
    let (ps, mut page) = live(101, PlaybackState::Playing);
    frames(&ps, &mut page, 0, 64);
    down(&ps, &mut page, SDLK_UP, 0, 1_000);
    assert!(matches!(page.live.zap, Zap::Pending { .. }));
    let (handled, reqs) = down(&ps, &mut page, 53, 0, 1_100);
    assert_eq!((handled, reqs), (Handled::Yes, vec![open_entry(101, 5)]), "the panel names the channel ON SCREEN, not the zap's target");
    assert!(page.live.zap.is_idle(), "the pending zap is gone");
    assert_eq!(frames(&ps, &mut page, 1_116, 2_500), Vec::<PlayerReq>::new(), "the window closes on nothing");
}

/// **A pick from number entry is one `PlayerReq::Tune`**, through the same zap path (no second tune path): the
/// banner and the art follow the channel at once, the tune is asked for on the next frame, once, and the session
/// landing on it settles everything.
#[test]
fn a_pick_from_number_entry_is_one_tune_through_the_zap() {
    let _g = plx_base::testlock::serial();
    let (ps, mut page) = live(101, PlaybackState::Playing);
    frames(&ps, &mut page, 0, 64);
    let (handled, reqs) = run(&ps, &mut page, 1_000, picked(103));
    assert_eq!((handled, reqs), (Handled::Yes, Vec::new()), "the pick asks for nothing by itself: the frame does");
    assert_eq!(shown(&ps, &page).as_deref(), Some("103"), "the banner follows the pick");
    assert_eq!(page.live.banner_alpha(1_000), 1.0);
    assert_eq!((page.live.art, page.live.art_alpha(1_000)), (Art::Covering, 1.0), "the target's art covers the picture");
    let (asked_at, reqs) = until_asked(&ps, &mut page, 1_016);
    assert_eq!((reqs, asked_at), (vec![tune_of(103)], 1_016), "one Tune, on the next frame: the viewer already waited");
    assert_eq!(frames(&ps, &mut page, asked_at + 16, asked_at + 2_000), Vec::<PlayerReq>::new(), "and no second");
    assert!(matches!(page.live.zap, Zap::Tuning { .. }));
    // the channel lands: the art fades, the zap ends
    let (landed, _) = live(103, PlaybackState::Playing);
    frames(&landed, &mut page, asked_at + 2_016, asked_at + 3_000);
    assert!(page.live.zap.is_idle());
    assert_eq!(page.live.art, Art::Settled);
    assert_eq!(shown(&landed, &page).as_deref(), Some("103"));
}

/// A channel the lineup no longer lists is not tuned, and a tune the app refuses ends the pick on the app's word.
#[test]
fn a_pick_of_a_vanished_channel_tunes_nothing_and_a_refused_one_ends() {
    let _g = plx_base::testlock::serial();
    let (ps, mut page) = live(101, PlaybackState::Playing);
    frames(&ps, &mut page, 0, 64);
    run(&ps, &mut page, 1_000, picked(900));
    assert!(page.live.zap.is_idle());
    assert_eq!(frames(&ps, &mut page, 1_016, 2_000), Vec::<PlayerReq>::new());
    run(&ps, &mut page, 3_000, picked(104));
    let (asked_at, reqs) = until_asked(&ps, &mut page, 3_016);
    assert_eq!(reqs, vec![tune_of(104)]);
    run(&ps, &mut page, asked_at + 16, ScreenEvent::App(AppMsg::LiveTuneRefused));
    assert!(page.live.zap.is_idle());
}

/// A lineup with nothing to step to leaves the press a no-op that still swallows it.
#[test]
fn an_empty_lineup_steps_nowhere() {
    let _g = plx_base::testlock::serial();
    let (ps, mut page) = live(101, PlaybackState::Playing);
    set_lineup(Vec::new());
    assert_eq!(down(&ps, &mut page, SDLK_UP, 0, 1_000), (Handled::Yes, Vec::new()));
    assert!(page.live.zap.is_idle());
}
