//! The player's side of the hosted contract: which epochs are live, and the queue the host's player events
//! wait in until the loop thread drains them.
//!
//! The host runs the platform player on a video thread of its own and answers every request as an event
//! (`include/plx_host.h`, PLAYER). Those events must not share the input queue: that one drops the NEWEST input
//! when it is full, which would lose a `PLAYER_COMPLETED` behind a burst of keys. This queue is lossless for
//! every kind but the two that only ever matter at their latest ([`PlayerKind::coalesces`]), and it is
//! filtered at the boundary by EPOCH: an epoch is live from [`PlayerLink::opened`] until the library accepts
//! that epoch's `PLAYER_CLOSED`, and an event of an epoch that is not live (never opened, or already
//! closed: a completion that landed after a stop) is dropped before the application can see it. The
//! application's driver still checks its own active epoch; this is the second layer.
//!
//! Single consumer: [`PlayerLink::drain`] is called from the loop thread. Producers are any thread
//! (`plx_host_post`). Nothing here blocks beyond one short mutex.
use crate::host_abi::{self, HostCue, PlayerEvent, PlayerKind};
use std::collections::VecDeque;
use std::sync::Mutex;

/// How many player events wait for the loop thread. A playing player makes a few events a second (positions and
/// buffering are coalesced to one each); its one burst is at PREPARED, when the host posts every native audio and text
/// track's language ([`host_abi::AUDIO_LANGUAGE_REPORTS`] and [`host_abi::TEXT_LANGUAGE_REPORTS`] at most) and then
/// PREPARED, VIDEO_SIZE and the answers held for it ([`PREPARED_BURST_EVENTS`]) before the loop has drained anything. The
/// bound holds that burst with room to spare (asserted below), so it is only reached when the loop is not draining; a
/// lossless event that does not fit is an overflow the driver must treat as a fault.
const PLAYER_QUEUE_LIMIT: usize = 128;

/// The events other than the languages the host can post in its PREPARED burst: a held track's and a held seek's
/// answers, PREPARED, VIDEO_SIZE, STARTED (a play held for PREPARED), and one coalesced BUFFERING and POSITION, with
/// room for an answer folded from more than one held command.
const PREPARED_BURST_EVENTS: usize = 12;

const _: () = assert!(host_abi::AUDIO_LANGUAGE_REPORTS + host_abi::TEXT_LANGUAGE_REPORTS + PREPARED_BURST_EVENTS
    < PLAYER_QUEUE_LIMIT, "the PREPARED burst must fit the player queue");

/// How many subtitle cues wait for the loop thread. The player raises about two a second at most (a cue and the empty
/// one that ends it), and the loop drains every frame, so the bound is only reached when the loop is not draining. A cue
/// that does not fit is dropped and counted: it is a subtitle missed, never a fault of the player.
const CUE_QUEUE_LIMIT: usize = 32;

/// How many epochs are tracked as live at once. The driver opens one at a time and waits for its close; the
/// bound only keeps a host that never reports `PLAYER_CLOSED` from growing the list.
const LIVE_EPOCHS: usize = 8;

/// What one drain found, besides the events.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Drained {
    /// A lossless event did not fit since the last drain: the application has lost part of the player's story.
    pub overflowed: bool,
    /// Events dropped at the boundary since the last drain because their epoch was not live.
    pub stale: usize,
}

struct Inner {
    live: Vec<u32>,
    events: VecDeque<PlayerEvent>,
    overflowed: bool,
    stale: usize,
    /// The cues of live epochs, on a queue of their own ([`CUE_QUEUE_LIMIT`]).
    cues: VecDeque<HostCue>,
    /// Cues dropped for a full cue queue since the last drain.
    cues_dropped: usize,
}

pub struct PlayerLink {
    inner: Mutex<Inner>,
}

impl PlayerLink {
    pub const fn new() -> Self {
        Self { inner: Mutex::new(Inner { live: Vec::new(), events: VecDeque::new(), overflowed: false, stale: 0,
            cues: VecDeque::new(), cues_dropped: 0 }) }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `epoch` is being opened: its events are accepted from now on (before the host is asked, so an event
    /// that races the request's return is not dropped).
    pub fn opened(&self, epoch: u32) {
        let mut inner = self.lock();
        if !inner.live.contains(&epoch) {
            if inner.live.len() >= LIVE_EPOCHS {
                inner.live.remove(0);
            }
            inner.live.push(epoch);
        }
    }

    /// Whether any epoch is live (opened, not yet closed).
    pub fn any_live(&self) -> bool {
        !self.lock().live.is_empty()
    }

    /// The host refused the open: nothing will come for `epoch`.
    pub fn forget(&self, epoch: u32) {
        self.lock().live.retain(|e| *e != epoch);
    }

    /// Queue the host's event, or say why not: [`host_abi::E_STALE`] for an epoch that is not live,
    /// [`host_abi::E_FULL`] for a queue that has no room. A `PLAYER_CLOSED` is queued and ENDS its epoch: what
    /// the host posts for it afterwards is stale.
    pub fn accept(&self, event: PlayerEvent) -> Result<(), i32> {
        let mut inner = self.lock();
        if !inner.live.contains(&event.epoch) {
            inner.stale += 1;
            return Err(host_abi::E_STALE);
        }
        if event.kind.coalesces() {
            if let Some(queued) = inner.events.iter_mut().find(|q| q.epoch == event.epoch && q.kind == event.kind) {
                queued.a = event.a;
                queued.b = event.b;
                return Ok(());
            }
        }
        if inner.events.len() >= PLAYER_QUEUE_LIMIT {
            if !event.kind.coalesces() {
                inner.overflowed = true;
            }
            return Err(host_abi::E_FULL);
        }
        inner.events.push_back(event);
        if event.kind == PlayerKind::Closed {
            inner.live.retain(|e| *e != event.epoch);
        }
        Ok(())
    }

    /// Queue the host's cue, or say why not: [`host_abi::E_STALE`] for an epoch that is not live, [`host_abi::E_FULL`]
    /// for a cue queue that has no room (the cue is dropped and counted; nothing else is touched).
    pub fn accept_cue(&self, cue: HostCue) -> Result<(), i32> {
        let mut inner = self.lock();
        if !inner.live.contains(&cue.epoch) {
            inner.stale += 1;
            return Err(host_abi::E_STALE);
        }
        if inner.cues.len() >= CUE_QUEUE_LIMIT {
            inner.cues_dropped += 1;
            return Err(host_abi::E_FULL);
        }
        inner.cues.push_back(cue);
        Ok(())
    }

    /// Hand over every queued cue, in order, and how many were dropped for a full queue since the last call. Loop thread.
    pub fn drain_cues(&self, out: &mut Vec<HostCue>) -> usize {
        let mut inner = self.lock();
        out.extend(inner.cues.drain(..));
        std::mem::take(&mut inner.cues_dropped)
    }

    /// Hand over everything queued, in order. Loop thread.
    pub fn drain(&self, out: &mut Vec<PlayerEvent>) -> Drained {
        let mut inner = self.lock();
        out.extend(inner.events.drain(..));
        Drained { overflowed: std::mem::take(&mut inner.overflowed), stale: std::mem::take(&mut inner.stale) }
    }
}

impl Default for PlayerLink {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cue(epoch: u32, text: &str) -> HostCue {
        HostCue { epoch, position_ms: 2_000, duration_ms: 1_500, text: text.to_owned() }
    }

    /// **Cues have a queue of their own**: a cue of a live epoch is kept in order, a cue of an epoch that is not live is
    /// stale, and a full cue queue drops the cue and counts it while the player's own events are untouched (a subtitle
    /// missed is never an overflow that fails the attempt).
    #[test]
    fn cues_queue_apart_from_the_player_events_and_a_full_queue_only_drops_cues() {
        let link = PlayerLink::new();
        assert_eq!(link.accept_cue(cue(1, "never opened")), Err(host_abi::E_STALE));
        link.opened(1);
        for n in 0..CUE_QUEUE_LIMIT {
            assert_eq!(link.accept_cue(cue(1, &format!("cue {n}"))), Ok(()));
        }
        assert_eq!(link.accept_cue(cue(1, "one too many")), Err(host_abi::E_FULL));
        assert_eq!(link.accept(PlayerEvent { epoch: 1, kind: PlayerKind::Completed, a: 0, b: 0 }), Ok(()));
        let mut cues = Vec::new();
        assert_eq!(link.drain_cues(&mut cues), 1, "the dropped cue is counted");
        assert_eq!(cues.len(), CUE_QUEUE_LIMIT);
        assert_eq!(cues[0].text, "cue 0");
        let mut events = Vec::new();
        assert!(!link.drain(&mut events).overflowed, "a full cue queue is no overflow of the player events");
        assert_eq!(events.len(), 1);
        assert_eq!(link.drain_cues(&mut cues), 0, "the count is reset by the drain");
        assert_eq!(link.accept(PlayerEvent { epoch: 1, kind: PlayerKind::Closed, a: 0, b: 0 }), Ok(()));
        assert_eq!(link.accept_cue(cue(1, "after the close")), Err(host_abi::E_STALE), "a closed epoch's cue is stale");
    }

    fn ev(epoch: u32, kind: PlayerKind, a: i64) -> PlayerEvent {
        PlayerEvent { epoch, kind, a, b: 0 }
    }

    /// An event of an epoch that was never opened is dropped, counted and answers `E_STALE`.
    #[test]
    fn an_epoch_that_was_never_opened_is_stale() {
        let link = PlayerLink::new();
        assert_eq!(link.accept(ev(5, PlayerKind::Prepared, 1000)), Err(host_abi::E_STALE));
        let mut out = Vec::new();
        assert_eq!(link.drain(&mut out), Drained { overflowed: false, stale: 1 });
        assert!(out.is_empty());
    }

    /// **A late completion after the close is dropped**: `PLAYER_CLOSED` ends the epoch, so what the host's
    /// callbacks post afterwards (a completion, a position) never reaches the application, and a NEW epoch is
    /// unaffected.
    #[test]
    fn events_after_the_close_of_their_epoch_are_dropped() {
        let link = PlayerLink::new();
        link.opened(1);
        assert_eq!(link.accept(ev(1, PlayerKind::Prepared, 1000)), Ok(()));
        assert_eq!(link.accept(ev(1, PlayerKind::Closed, 0)), Ok(()));
        assert_eq!(link.accept(ev(1, PlayerKind::Completed, 0)), Err(host_abi::E_STALE));
        assert_eq!(link.accept(ev(1, PlayerKind::Position, 9)), Err(host_abi::E_STALE));
        link.opened(2);
        assert_eq!(link.accept(ev(2, PlayerKind::Started, 0)), Ok(()));
        let mut out = Vec::new();
        let drained = link.drain(&mut out);
        assert_eq!(
            out.iter().map(|e| (e.epoch, e.kind)).collect::<Vec<_>>(),
            [(1, PlayerKind::Prepared), (1, PlayerKind::Closed), (2, PlayerKind::Started)]
        );
        assert_eq!(drained.stale, 2);
    }

    /// A player is live from its open until its close or the host's refusal (what the packet probe asks).
    #[test]
    fn a_player_is_live_from_its_open_until_its_close_or_refusal() {
        let link = PlayerLink::new();
        assert!(!link.any_live());
        link.opened(3);
        assert!(link.any_live());
        link.forget(3);
        assert!(!link.any_live(), "a refused open");
        link.opened(4);
        link.accept(ev(4, PlayerKind::Closed, 0)).expect("queued");
        assert!(!link.any_live(), "closed");
    }

    /// A refused open leaves nothing live; a stale epoch cannot come back.
    #[test]
    fn a_refused_open_forgets_its_epoch() {
        let link = PlayerLink::new();
        link.opened(3);
        link.forget(3);
        assert_eq!(link.accept(ev(3, PlayerKind::Error, 7)), Err(host_abi::E_STALE));
    }

    /// Position and buffering keep only the latest per epoch, in the place of the first; every other kind is
    /// kept, in order.
    #[test]
    fn position_and_buffering_coalesce_and_the_rest_is_lossless() {
        let link = PlayerLink::new();
        link.opened(1);
        for event in [
            ev(1, PlayerKind::Prepared, 100),
            ev(1, PlayerKind::Position, 10),
            ev(1, PlayerKind::Buffering, 20),
            ev(1, PlayerKind::Position, 250),
            ev(1, PlayerKind::Position, 500),
            ev(1, PlayerKind::Buffering, 100),
            ev(1, PlayerKind::SeekDone, 4),
            ev(1, PlayerKind::SeekDone, 5),
        ] {
            assert_eq!(link.accept(event), Ok(()));
        }
        let mut out = Vec::new();
        link.drain(&mut out);
        assert_eq!(
            out.iter().map(|e| (e.kind, e.a)).collect::<Vec<_>>(),
            [
                (PlayerKind::Prepared, 100),
                (PlayerKind::Position, 500),
                (PlayerKind::Buffering, 100),
                (PlayerKind::SeekDone, 4),
                (PlayerKind::SeekDone, 5)
            ]
        );
    }

    /// The PREPARED burst fits: the host posts every native audio and text track's language (up to each cap) and then PREPARED,
    /// VIDEO_SIZE and the answers held for PREPARED in one go, before the loop drains anything (review round 2 of #1566).
    #[test]
    fn the_prepared_burst_fits_the_queue() {
        let link = PlayerLink::new();
        link.opened(1);
        for index in 0..host_abi::AUDIO_LANGUAGE_REPORTS {
            assert_eq!(link.accept(ev(1, PlayerKind::AudioLanguage, index as i64)), Ok(()));
        }
        for index in 0..host_abi::TEXT_LANGUAGE_REPORTS {
            assert_eq!(link.accept(ev(1, PlayerKind::TextLanguage, index as i64)), Ok(()));
        }
        let others = [PlayerKind::Track, PlayerKind::SeekDone, PlayerKind::Prepared, PlayerKind::VideoSize,
            PlayerKind::Started, PlayerKind::Buffering, PlayerKind::Position];
        assert!(others.len() <= PREPARED_BURST_EVENTS);
        for kind in others {
            assert_eq!(link.accept(ev(1, kind, 0)), Ok(()), "{kind:?}");
        }
        let mut out = Vec::new();
        assert!(!link.drain(&mut out).overflowed);
        assert_eq!(out.len(), host_abi::AUDIO_LANGUAGE_REPORTS + host_abi::TEXT_LANGUAGE_REPORTS + others.len());
    }

    /// A lossless event that does not fit is an overflow the drain reports (never a silent drop); a full
    /// queue still takes the latest position of an epoch that already has one queued.
    #[test]
    fn a_full_queue_reports_an_overflow_of_a_lossless_event() {
        let link = PlayerLink::new();
        link.opened(1);
        assert_eq!(link.accept(ev(1, PlayerKind::Position, 1)), Ok(()));
        for n in 1..PLAYER_QUEUE_LIMIT {
            assert_eq!(link.accept(ev(1, PlayerKind::SeekDone, n as i64)), Ok(()));
        }
        assert_eq!(link.accept(ev(1, PlayerKind::Completed, 0)), Err(host_abi::E_FULL));
        assert_eq!(link.accept(ev(1, PlayerKind::Position, 2)), Ok(()), "a coalesced kind needs no room");
        let mut out = Vec::new();
        assert!(link.drain(&mut out).overflowed);
        assert!(!link.drain(&mut out).overflowed, "reported once");
        assert_eq!(out.len(), PLAYER_QUEUE_LIMIT);
    }
}
