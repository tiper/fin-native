//! **The live player's zap, as two small machines** (Live TV PR4; handoff SPEC-4). Pure: no clock read,
//! no session read, no effect performed; the screen feeds events and acts on the one effect that comes back.
//!
//! [`Zap`] is the channel being stepped to. ▲ / CH+ steps to the next higher number and ▼ / CH- to the
//! next lower, from the channel the banner shows: a press moves the target at once and the banner follows
//! it, while the stream starts [`DEBOUNCE_MS`] after the LAST press, so a run of presses opens one Plex
//! session, not one each. The old stream keeps playing (audio) through the window and is stopped by the
//! tune itself, which is the app's: a one-tuner server frees its tuner before the new tune.
//!
//! ```text
//!   Idle --Step--> Pending{target, last} --Step--> Pending{target', now}   (the window restarts)
//!   Pending --Tick, DEBOUNCE_MS after the last press, target is not what plays--> Tuning{target}  + Effect::Tune(target)
//!   Pending --Tick, DEBOUNCE_MS after the last press, target IS what plays--> Idle   (Up then Down: nothing to retune)
//!   Idle | Pending | Tuning --Jump{target}--> Pending{target, a window already run out}   (number entry chose it: the next tick tunes)
//!   Tuning --Step--> Pending{next of target, now}     (a press after the tune fired: a new window)
//!   Tuning --Session: the session plays target and its first frame is up--> Idle
//!   Tuning --Refused: the app took no request for the tune (AppMsg::LiveTuneRefused)--> Idle
//!   Pending | Tuning --Cancel: BACK / STOP, the viewer is leaving--> Idle
//! ```
//!
//! [`Art`] is the picture drawn over the video plane's hole until a first frame exists: covering from the
//! first press (owner decision Q10) through the debounce and the tune until the first frame, fading out
//! then. It is told one [`ArtEvent`] a frame (a zap in flight, a stream with no frame, a frame); the fade is
//! its own, and once a frame has been shown a later stall never brings it back.
use plx_data::livetv::Channel;

/// How long after the last press the stream starts (SPEC-4).
pub const DEBOUNCE_MS: u32 = 600;
/// How long the art takes to fade out when the first frame arrives. SPEC-4 says only "fades out when the
/// video arrives"; the duration is this PR's default, close to the banner's own fade.
pub const ART_FADE_MS: u32 = 300;

/// A channel by identity: its server, DVR and key, never its number.
pub fn same(a: &Channel, b: &Channel) -> bool {
    a.sid == b.sid && a.dvr == b.dvr && a.channel == b.channel
}

/// The zap target, as the screen's machine.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum Zap {
    #[default]
    Idle,
    /// presses are being collected; `last` is the time of the latest
    Pending { target: Channel, last: u32 },
    /// the tune was asked for; the target stands until the session shows it (or the app refuses it)
    Tuning { target: Channel },
}

/// What happened to the machine.
#[derive(Clone, Copy, Debug)]
pub enum ZapEvent<'a> {
    /// ▲ / CH+ (`dir` +1) or ▼ / CH- (-1) at `now`, over `lineup` (in number order), the session playing `playing`
    Step { dir: i32, lineup: &'a [Channel], playing: Option<&'a Channel>, now: u32 },
    /// number entry chose `target` at `now` (Live TV PR5): a target stood on directly, not stepped to. It tunes at the
    /// next tick: the viewer already waited for it (the entry's own 2.5 s, or a number that fits one channel), so it
    /// carries no debounce window of its own, and it follows the same art, banner, session and refusal path as a step.
    Jump { target: &'a Channel, now: u32 },
    /// time passing; `playing` is what the session plays, so a window that closes on the channel already playing
    /// asks for nothing
    Tick { now: u32, playing: Option<&'a Channel> },
    /// what the session plays and whether its first frame is up
    Session { playing: Option<&'a Channel>, framed: bool },
    /// the app took no request for the tune it was asked for (`AppMsg::LiveTuneRefused`)
    Refused,
    /// the viewer is leaving (BACK / STOP): no tune may follow
    Cancel,
}

/// What the screen must do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ZapEffect {
    None,
    /// start the stream of this channel (`PlayerReq::Tune`)
    Tune(Channel),
}

impl Zap {
    /// The channel stepped to, while one is: what the banner and the art follow.
    pub fn target(&self) -> Option<&Channel> {
        match self {
            Zap::Idle => None,
            Zap::Pending { target, .. } | Zap::Tuning { target, .. } => Some(target),
        }
    }

    pub fn is_idle(&self) -> bool {
        matches!(self, Zap::Idle)
    }

    pub fn step(&mut self, ev: ZapEvent<'_>) -> ZapEffect {
        match ev {
            ZapEvent::Step { dir, lineup, playing, now } => {
                let Some(from) = self.target().or(playing) else { return ZapEffect::None };
                let Some(at) = lineup.iter().position(|c| same(c, from)) else { return ZapEffect::None };
                if lineup.len() < 2 || !matches!(dir, -1 | 1) {
                    return ZapEffect::None;
                }
                let next = if dir < 0 { at.checked_sub(1).unwrap_or(lineup.len() - 1) } else { (at + 1) % lineup.len() };
                *self = Zap::Pending { target: lineup[next].clone(), last: now };
                ZapEffect::None
            }
            ZapEvent::Jump { target, now } => {
                // a window that has already run out: the next tick starts the stream
                *self = Zap::Pending { target: target.clone(), last: now.wrapping_sub(DEBOUNCE_MS) };
                ZapEffect::None
            }
            ZapEvent::Tick { now, playing } => match self {
                Zap::Pending { target, last } if now.wrapping_sub(*last) >= DEBOUNCE_MS => {
                    if playing.is_some_and(|c| same(c, target)) {
                        // stepped away and back inside the window: the channel is the one already playing
                        *self = Zap::Idle;
                        return ZapEffect::None;
                    }
                    let target = target.clone();
                    *self = Zap::Tuning { target: target.clone() };
                    ZapEffect::Tune(target)
                }
                _ => ZapEffect::None,
            },
            ZapEvent::Session { playing, framed } => {
                if let Zap::Tuning { target } = self {
                    if framed && playing.is_some_and(|c| same(c, target)) {
                        *self = Zap::Idle;
                    }
                }
                ZapEffect::None
            }
            ZapEvent::Refused => {
                if matches!(self, Zap::Tuning { .. }) {
                    *self = Zap::Idle;
                }
                ZapEffect::None
            }
            ZapEvent::Cancel => {
                *self = Zap::Idle;
                ZapEffect::None
            }
        }
    }
}

/// The art ground's phase.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Art {
    /// the page's first state: nothing drawn, and the stream has shown no frame since the tune. A stream that stays
    /// frameless covers; one that shows a frame settles.
    #[default]
    Waiting,
    /// the picture is wanted: drawn whole
    Covering,
    /// the picture is no longer wanted: fading out since `since`
    Fading { since: u32 },
    /// a frame has been shown since the tune: nothing drawn, and a later stall (a rebuffer) never brings the art
    /// back; only a zap does
    Settled,
}

/// What the screen tells [`Art`] each frame: exactly one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArtEvent {
    /// a zap is in flight: its art stands over the hole whatever the stream says
    Zapping,
    /// the session has no picture on the panel (a stream still starting, or stalled)
    NoFrame,
    /// the session shows a frame
    Frame,
}

impl Art {
    /// One step. Only a stream that has NEVER shown a frame since the tune is covered for lacking one: a stall
    /// mid-playback ([`Art::Settled`] receiving `NoFrame`) is the frozen picture's, not the art's.
    pub fn step(&mut self, ev: ArtEvent, now: u32) {
        *self = match (*self, ev) {
            (_, ArtEvent::Zapping) => Art::Covering,
            (Art::Waiting, ArtEvent::NoFrame) => Art::Covering,
            (Art::Waiting, ArtEvent::Frame) => Art::Settled,
            (Art::Covering, ArtEvent::NoFrame) => Art::Covering,
            (Art::Covering, ArtEvent::Frame) => Art::Fading { since: now },
            (Art::Fading { since }, _) if now.wrapping_sub(since) >= ART_FADE_MS => Art::Settled,
            (other, _) => other,
        };
    }

    /// How much of the picture is drawn at `now`: 1 covering, a linear fade, 0 otherwise.
    pub fn alpha(self, now: u32) -> f32 {
        match self {
            Art::Waiting | Art::Settled => 0.0,
            Art::Covering => 1.0,
            Art::Fading { since } => (1.0 - now.wrapping_sub(since) as f32 / ART_FADE_MS as f32).clamp(0.0, 1.0),
        }
    }
}

/// The banner's phase: raised with the page (the screen has no clock yet, so the deadline is stamped by its first
/// frame), or shown until a deadline (then it fades, [`live::FADE_MS`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Banner {
    #[default]
    Entering,
    Shown { until: u32 },
}

/// What the screen reads off the session and the lineup this frame, handed to [`Live::tick`].
#[derive(Clone, Copy, Debug)]
pub struct Observed<'a> {
    pub now: u32,
    /// the channel the session is tuned to (the lineup's entry for it, else the published guide facts)
    pub playing: Option<&'a Channel>,
    pub lineup: &'a [Channel],
    /// the session has no frame on the panel yet (resolving, connecting or buffering, or no engine)
    pub no_frame: bool,
}

/// **The live player's whole decision state**: the zap, the art ground and the banner, as three named
/// phases. Stepped from keys ([`Self::press`], [`Self::reveal`]) and from the frame ([`Self::tick`]); never
/// reads a clock or the session itself.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Live {
    pub zap: Zap,
    pub art: Art,
    pub banner: Banner,
}

impl Live {
    /// ▲ / CH+ (`dir` +1) or ▼ / CH- (-1): move the target, raise the banner on it and cover with its art.
    pub fn press(&mut self, dir: i32, o: &Observed<'_>) {
        self.zap.step(ZapEvent::Step { dir, lineup: o.lineup, playing: o.playing, now: o.now });
        if !self.zap.is_idle() {
            self.art.step(ArtEvent::Zapping, o.now);
            self.reveal(o.now);
        }
    }

    /// BACK / STOP: the viewer is leaving, so no tune may follow the Exit the same frame the debounce would have closed.
    pub fn leave(&mut self) {
        self.cancel_zap();
    }

    /// Drop a pending zap: no tune may follow. BACK / STOP leave this way, and so does a digit that opens number
    /// entry, so a press run's window cannot close under the panel and tune a channel the viewer is typing past.
    pub fn cancel_zap(&mut self) {
        self.zap.step(ZapEvent::Cancel);
    }

    /// Number entry chose `target` ([`ZapEvent::Jump`]): the banner and the art follow it at once and the next tick
    /// asks for the one tune, exactly as a zap's does.
    pub fn jump(&mut self, target: &Channel, o: &Observed<'_>) {
        self.zap.step(ZapEvent::Jump { target, now: o.now });
        self.art.step(ArtEvent::Zapping, o.now);
        self.reveal(o.now);
    }

    /// The app started a stream on this page (`AppMsg::PlayerStart`: a tune, or a zap landing): the banner is raised.
    pub fn started(&mut self, now: u32) {
        self.reveal(now);
    }

    /// The app took no request for the tune it was asked for (`AppMsg::LiveTuneRefused`).
    pub fn refused(&mut self) {
        self.zap.step(ZapEvent::Refused);
    }

    /// Raise the banner for [`live::SHOW_MS`] more (Left, Right, OK, a tune, a press).
    pub fn reveal(&mut self, now: u32) {
        self.banner = Banner::Shown { until: now.wrapping_add(plx_appkit::player_hud::live::SHOW_MS) };
    }

    /// One frame: the session's answer to the zap, the art's phase, the banner's on a tune, then the
    /// debounce. Returns the tune to ask for, if the window has just closed.
    pub fn tick(&mut self, o: &Observed<'_>) -> ZapEffect {
        if self.banner == Banner::Entering {
            self.reveal(o.now);
        }
        self.zap.step(ZapEvent::Session { playing: o.playing, framed: !o.no_frame });
        let was_covering = self.art == Art::Covering;
        let art = if !self.zap.is_idle() { ArtEvent::Zapping } else if o.no_frame { ArtEvent::NoFrame } else { ArtEvent::Frame };
        self.art.step(art, o.now);
        // the first frame of a tune: the banner's four seconds start when the picture does
        if was_covering && self.art != Art::Covering {
            self.reveal(o.now);
        }
        self.zap.step(ZapEvent::Tick { now: o.now, playing: o.playing })
    }

    /// The channel the banner and the art follow: the zap's target while one stands, else the session's.
    pub fn shown<'a>(&'a self, playing: Option<&'a Channel>) -> Option<&'a Channel> {
        self.zap.target().or(playing)
    }

    /// How opaque the banner is at `now`: held while a zap is pending or the picture is not there yet.
    pub fn banner_alpha(&self, now: u32) -> f32 {
        match self.banner {
            Banner::Entering => 1.0,
            Banner::Shown { until } => {
                plx_appkit::player_hud::live::alpha(now, until, !self.zap.is_idle() || self.art == Art::Covering)
            }
        }
    }

    pub fn art_alpha(&self, now: u32) -> f32 {
        self.art.alpha(now)
    }

    /// A term for the frame gate: changes whenever what is drawn can change with nothing else moving (a
    /// phase edge, the banner's deadline passing, a fade running, the shown channel or its airing).
    pub fn fingerprint(&self, now: u32, shown: Option<&Channel>) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let mut mix = |v: u64| h = (h ^ v).wrapping_mul(0x0100_0000_01b3);
        mix(match self.zap { Zap::Idle => 0, Zap::Pending { .. } => 1, Zap::Tuning { .. } => 2 });
        mix(match self.art { Art::Waiting => 0, Art::Covering => 1, Art::Fading { .. } => 2, Art::Settled => 3 });
        let alpha = self.banner_alpha(now);
        mix(match self.banner { Banner::Entering => 0, Banner::Shown { .. } => 1 + u64::from(alpha > 0.0) });
        // a fade is a frame per tick for as long as it runs; its start is the deadline passing
        let fading = (alpha > 0.0 && alpha < 1.0) || matches!(self.art, Art::Fading { .. });
        if fading {
            mix(u64::from(now));
        }
        if let Banner::Shown { until } = self.banner {
            mix(u64::from(until));
        }
        if let Some(c) = shown {
            mix(c.channel.len() as u64);
            mix(c.airings.begins_at() as u64);
            mix(c.airings.ends_at() as u64);
        }
        h
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(n: &str) -> Channel {
        Channel { sid: plx_plex::plex::ServerId::UNSET, dvr: "7".into(), channel: format!("C{n}"), vcn: n.into(),
            name: format!("Station {n}"), thumb: String::new(),
            airings: plx_data::livetv::Airings::of(vec![plx_data::livetv::LiveAiring::new(format!("Show {n}"), 0, 0)], 0) }
    }

    fn lineup() -> Vec<Channel> { ["101", "102", "103", "104"].map(ch).to_vec() }

    #[test]
    fn three_presses_inside_the_window_tune_once_the_third_target() {
        let lineup = lineup();
        let playing = ch("101");
        let mut zap = Zap::default();
        for now in [1_000, 1_200, 1_400] {
            assert_eq!(zap.step(ZapEvent::Step { dir: 1, lineup: &lineup, playing: Some(&playing), now }), ZapEffect::None);
            // the window has not elapsed since the latest press
            assert_eq!(zap.step(ZapEvent::Tick { now: now + 599, playing: None }), ZapEffect::None, "at {now}");
        }
        assert_eq!(zap.target().map(|c| c.vcn.as_str()), Some("104"));
        assert_eq!(zap.step(ZapEvent::Tick { now: 1_400 + DEBOUNCE_MS, playing: None }), ZapEffect::Tune(ch("104")));
        // the machine tunes once: further ticks ask nothing
        assert_eq!(zap.step(ZapEvent::Tick { now: 3_000, playing: None }), ZapEffect::None);
    }

    #[test]
    fn a_press_after_the_tune_fired_starts_a_new_window_from_the_tuned_target() {
        let lineup = lineup();
        let playing = ch("101");
        let mut zap = Zap::default();
        zap.step(ZapEvent::Step { dir: 1, lineup: &lineup, playing: Some(&playing), now: 0 });
        assert!(matches!(zap.step(ZapEvent::Tick { now: 600, playing: None }), ZapEffect::Tune(_)));
        // the session still plays 101 (the tune has not landed): the next press steps from the TARGET
        zap.step(ZapEvent::Step { dir: 1, lineup: &lineup, playing: Some(&playing), now: 700 });
        assert_eq!(zap.target().map(|c| c.vcn.as_str()), Some("103"));
        assert_eq!(zap.step(ZapEvent::Tick { now: 1_299, playing: None }), ZapEffect::None);
        assert_eq!(zap.step(ZapEvent::Tick { now: 1_300, playing: None }), ZapEffect::Tune(ch("103")));
    }

    #[test]
    fn up_is_the_next_higher_number_and_both_directions_wrap() {
        let lineup = lineup();
        let mut zap = Zap::default();
        zap.step(ZapEvent::Step { dir: -1, lineup: &lineup, playing: Some(&ch("101")), now: 0 });
        assert_eq!(zap.target().map(|c| c.vcn.as_str()), Some("104"), "below the lowest wraps to the highest");
        zap.step(ZapEvent::Step { dir: 1, lineup: &lineup, playing: Some(&ch("101")), now: 1 });
        assert_eq!(zap.target().map(|c| c.vcn.as_str()), Some("101"), "above the highest wraps to the lowest");
    }

    #[test]
    fn a_channel_the_lineup_does_not_list_or_a_lineup_of_one_steps_nowhere() {
        let mut zap = Zap::default();
        zap.step(ZapEvent::Step { dir: 1, lineup: &lineup(), playing: Some(&ch("999")), now: 0 });
        assert!(zap.is_idle());
        zap.step(ZapEvent::Step { dir: 1, lineup: &[ch("101")], playing: Some(&ch("101")), now: 0 });
        assert!(zap.is_idle());
        zap.step(ZapEvent::Step { dir: 1, lineup: &lineup(), playing: None, now: 0 });
        assert!(zap.is_idle());
    }

    #[test]
    fn the_target_stands_until_the_session_shows_its_first_frame() {
        let lineup = lineup();
        let mut zap = Zap::default();
        zap.step(ZapEvent::Step { dir: 1, lineup: &lineup, playing: Some(&ch("101")), now: 0 });
        zap.step(ZapEvent::Tick { now: 600, playing: None });
        let target = ch("102");
        zap.step(ZapEvent::Session { playing: Some(&target), framed: false });
        assert_eq!(zap.target().map(|c| c.vcn.as_str()), Some("102"), "tuned but no frame yet");
        zap.step(ZapEvent::Session { playing: Some(&target), framed: true });
        assert!(zap.is_idle());
    }

    #[test]
    fn a_tune_the_app_did_not_take_ends_the_zap_on_the_refusal_not_on_timing() {
        let lineup = lineup();
        let old = ch("101");
        let mut zap = Zap::default();
        zap.step(ZapEvent::Step { dir: 1, lineup: &lineup, playing: Some(&old), now: 0 });
        zap.step(ZapEvent::Tick { now: 600, playing: Some(&old) });
        // the session still plays the old channel, frame after frame: nothing ends the zap but the app's word
        for _ in 0..5 {
            zap.step(ZapEvent::Session { playing: Some(&old), framed: true });
        }
        assert!(matches!(zap, Zap::Tuning { .. }), "no timing inference");
        zap.step(ZapEvent::Refused);
        assert!(zap.is_idle());
        // a refusal when nothing was asked changes nothing
        zap.step(ZapEvent::Step { dir: 1, lineup: &lineup, playing: Some(&old), now: 700 });
        zap.step(ZapEvent::Refused);
        assert!(matches!(zap, Zap::Pending { .. }));
    }

    #[test]
    fn up_then_down_inside_the_window_retunes_nothing() {
        let lineup = lineup();
        let playing = ch("101");
        let mut zap = Zap::default();
        zap.step(ZapEvent::Step { dir: 1, lineup: &lineup, playing: Some(&playing), now: 0 });
        zap.step(ZapEvent::Step { dir: -1, lineup: &lineup, playing: Some(&playing), now: 100 });
        assert_eq!(zap.target().map(|c| c.vcn.as_str()), Some("101"), "back on the channel that plays");
        assert_eq!(zap.step(ZapEvent::Tick { now: 100 + DEBOUNCE_MS, playing: Some(&playing) }), ZapEffect::None);
        assert!(zap.is_idle(), "the window closed on the playing channel: no tune, no new session");
        // a full lap of presses lands on it too
        for now in 1_000..1_004 {
            zap.step(ZapEvent::Step { dir: 1, lineup: &lineup, playing: Some(&playing), now });
        }
        assert_eq!(zap.step(ZapEvent::Tick { now: 1_004 + DEBOUNCE_MS, playing: Some(&playing) }), ZapEffect::None);
        assert!(zap.is_idle());
    }

    #[test]
    fn cancel_ends_a_pending_or_tuning_zap_and_no_tune_follows() {
        let lineup = lineup();
        let playing = ch("101");
        let mut zap = Zap::default();
        zap.step(ZapEvent::Step { dir: 1, lineup: &lineup, playing: Some(&playing), now: 0 });
        zap.step(ZapEvent::Cancel);
        assert_eq!(zap.step(ZapEvent::Tick { now: 5_000, playing: Some(&playing) }), ZapEffect::None);
        assert!(zap.is_idle());
        zap.step(ZapEvent::Step { dir: 1, lineup: &lineup, playing: Some(&playing), now: 6_000 });
        zap.step(ZapEvent::Tick { now: 6_600, playing: Some(&playing) });
        zap.step(ZapEvent::Cancel);
        assert!(zap.is_idle());
    }

    #[test]
    fn the_art_covers_from_the_first_press_and_fades_when_no_longer_wanted() {
        let mut art = Art::default();
        assert_eq!(art.alpha(0), 0.0);
        art.step(ArtEvent::Zapping, 10);
        assert_eq!((art, art.alpha(10)), (Art::Covering, 1.0));
        art.step(ArtEvent::Frame, 100);
        assert_eq!(art, Art::Fading { since: 100 });
        assert!(art.alpha(100 + ART_FADE_MS / 2) > 0.4 && art.alpha(100 + ART_FADE_MS / 2) < 0.6);
        art.step(ArtEvent::Frame, 100 + ART_FADE_MS);
        assert_eq!(art, Art::Settled);
        // a stall after the first frame is not the art's: no frame, no zap, still settled
        art.step(ArtEvent::NoFrame, 5_000);
        assert_eq!((art, art.alpha(5_000)), (Art::Settled, 0.0));
        // …but a zap covers again
        art.step(ArtEvent::Zapping, 6_000);
        assert_eq!(art, Art::Covering);
        // wanted again mid-fade: back to whole at once
        let mut art = Art::Fading { since: 5 };
        art.step(ArtEvent::Zapping, 50);
        assert_eq!(art, Art::Covering);
        // a stream that has shown nothing yet is covered for lacking a frame; one that shows a frame settles
        let mut art = Art::default();
        art.step(ArtEvent::NoFrame, 0);
        assert_eq!(art, Art::Covering);
        let mut art = Art::default();
        art.step(ArtEvent::Frame, 0);
        assert_eq!(art, Art::Settled);
    }
}
