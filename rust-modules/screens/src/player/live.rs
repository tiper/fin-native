//! **The live half of the player screen** (Live TV PR4; handoff SPEC-4): the keys of a live channel, the
//! frame's step of [`zap::Live`], and the paint of the art ground and the banner.
//!
//! A live channel has no transport. Everything the VOD page does with its HUD (the scrubber, the control
//! row, Info, Chapters, the track menus) is unreachable here by construction: [`PlayerScreen::hud_drawn`]
//! is `false` on a live session, and [`PlayerScreen::live_key`] answers every key the page used to route to
//! that HUD. What a key means on a live channel:
//!
//! | key | meaning |
//! |---|---|
//! | Up, CH+ | step to the next higher number ([`zap::Live::press`]) |
//! | Down, CH- | step to the next lower number |
//! | Left, Right, OK | raise the banner, nothing else |
//! | BACK, STOP | leave: a single press at every phase (the debounce, buffering, playing) |
//! | 0-9 | open number entry (`channel_entry_panel`, `PlayerReq::OpenChannelEntry`); a pending zap is dropped |
//! | everything else | where it was: play/pause, EXIT |
use super::*;
use plx_data::livetv::Channel;
use zap::{Observed, ZapEffect};

/// The channel the session is tuned to: the lineup's entry for the request (so its guide facts are the
/// store's freshest), else what the app published for it.
fn playing<'a>(ps: &'a plx_media::route::PlaybackSession, lineup: &'a [Channel]) -> Option<&'a Channel> {
    let (sid, tune) = plx_media::route::live_request(ps)?;
    lineup
        .iter()
        .find(|c| c.sid == sid && c.dvr == tune.dvr && c.channel == tune.channel)
        .or_else(|| plx_media::route::live_info(ps).map(|info| &info.channel))
}

/// No frame is on the panel yet: no engine, or one that is resolving, connecting or buffering.
fn no_frame(ps: &plx_media::route::PlaybackSession) -> bool {
    use plx_media::player::PlaybackState as S;
    matches!(plx_media::player::state(ps), S::Idle | S::Resolving | S::Connecting | S::Buffering)
}

/// The picture a channel's art ground draws: its programme's still, else its logo.
fn art_key(ch: &Channel) -> &str {
    if ch.now_thumb.is_empty() { &ch.thumb } else { &ch.now_thumb }
}

impl PlayerScreen {
    fn observed<'a>(ps: &'a plx_media::route::PlaybackSession, lineup: &'a [Channel], now: u32) -> Observed<'a> {
        Observed { now, playing: playing(ps, lineup), lineup, no_frame: no_frame(ps) }
    }

    /// A live channel's keys. `None` leaves the press to the rest of the page (play/pause, EXIT).
    pub(super) fn live_key<H: PlayerLike>(
        &mut self,
        ps: &plx_media::route::PlaybackSession,
        lineup: &[Channel],
        key: consts::Key,
        sym: u32,
        wcode: u32,
        edge: Edge,
        at_edge: bool,
        now: u32,
        fx: &mut Effects<'_, H>,
    ) -> Option<Handled> {
        use consts::Key;
        let fresh = edge == Edge::Down && !at_edge;
        // CH+ pages up the screen (-1) but means the next higher channel (+1).
        let step = consts::page_dir(sym, wcode).map(|dir| -dir).or(match key {
            Key::Up => Some(1),
            Key::Down => Some(-1),
            _ => None,
        });
        if let Some(dir) = step {
            if fresh {
                self.live.press(dir, &Self::observed(ps, lineup, now));
                fx.invalidate(plx_machine::present::Provenance::Input);
            }
            return Some(Handled::Yes);
        }
        // A digit opens number entry (PR5). A run of presses whose window has not closed is dropped first, so it cannot
        // tune under the panel; the panel names the channel ON SCREEN (what it tags "Playing"), never the zap's target.
        if let Some(digit) = consts::pin_digit(sym, wcode) {
            if fresh {
                self.live.cancel_zap();
                let playing = playing(ps, lineup).map(|c| (c.sid, c.dvr.clone(), c.channel.clone()));
                Self::ask(fx, PlayerReq::OpenChannelEntry { first: digit - b'0', playing });
                fx.invalidate(plx_machine::present::Provenance::Input);
            }
            return Some(Handled::Yes);
        }
        match key {
            Key::Left { .. } | Key::Right { .. } | Key::Ok => {
                if fresh {
                    self.live.reveal(now);
                    fx.invalidate(plx_machine::present::Provenance::Input);
                }
                Some(Handled::Yes)
            }
            Key::Back | Key::Stop => {
                if fresh {
                    self.live.leave();
                    Self::ask(fx, PlayerReq::Exit);
                }
                Some(Handled::Yes)
            }
            _ => None,
        }
    }

    /// One frame of a live channel: step [`Live`] and ask for the tune its debounce closed on.
    pub(super) fn live_tick<H: PlayerLike>(
        &mut self,
        ps: &plx_media::route::PlaybackSession,
        lineup: &[Channel],
        now: u32,
        fx: &mut Effects<'_, H>,
    ) {
        if !plx_media::route::is_live(ps) {
            return;
        }
        if let ZapEffect::Tune(ch) = self.live.tick(&Self::observed(ps, lineup, now)) {
            Self::ask(fx, PlayerReq::Tune { sid: ch.sid, dvr: ch.dvr, channel: ch.channel, title: ch.name });
        }
    }

    /// **Number entry chose a channel** (`AppMsg::LiveChannelPicked`): stand the zap on it ([`zap::Live::jump`]), so the
    /// next frame asks for the one `PlayerReq::Tune`, with the banner and the art following. A channel the lineup no
    /// longer lists, or a session that is no longer live, is no pick.
    pub(super) fn live_pick(
        &mut self,
        ps: &plx_media::route::PlaybackSession,
        lineup: &[Channel],
        pick: (plx_plex::plex::ServerId, &str, &str),
        now: u32,
    ) {
        let (sid, dvr, channel) = pick;
        if !plx_media::route::is_live(ps) {
            return;
        }
        if let Some(target) = lineup.iter().find(|c| c.sid == sid && c.dvr == dvr && c.channel == channel) {
            self.live.jump(target, &Self::observed(ps, lineup, now));
        }
    }

    /// The channel the banner and the art follow this frame, for the frame gate and the paint.
    pub(super) fn live_shown<'a>(&'a self, ps: &'a plx_media::route::PlaybackSession, lineup: &'a [Channel]) -> Option<&'a Channel> {
        self.live.shown(playing(ps, lineup))
    }

    /// Paint the art ground (the lowest layer: under the captions and the banner).
    pub(super) fn draw_live_art<H: PlayerLike>(
        &self,
        ps: &plx_media::route::PlaybackSession,
        lineup: &[Channel],
        now: u32,
        f: &DrawFrame<'_, '_, H>,
    ) {
        let art = self.live.art_alpha(now);
        if art <= 0.0 {
            return;
        }
        match self.live_shown(ps, lineup) {
            Some(ch) => plx_ui::art_ground::draw(f.painter, ch.sid.raw(), art_key(ch), art),
            None => plx_ui::art_ground::draw(f.painter, 0, "", art),
        };
    }

    /// Paint the banner.
    pub(super) fn draw_live_banner<H: PlayerLike>(
        &self,
        ps: &plx_media::route::PlaybackSession,
        lineup: &[Channel],
        now: u32,
        f: &DrawFrame<'_, '_, H>,
    ) {
        let alpha = self.live.banner_alpha(now);
        if alpha <= 0.0 {
            return;
        }
        let words = player_hud::live::words(self.live_shown(ps, lineup), "", self.model.wall);
        player_hud::live::draw(f.painter, &self.compiled.live(), &words, alpha, f.measure);
    }
}
