//! The application's clock (restructure spec §4.1): ONE door to frame time, so a replay can drive
//! the loop on recorded ticks. `today()` is the same door for the calendar date: the ONE place the
//! application asks the platform what day it is (`plx_os::i18n::local_calendar_date`) and which zone
//! it is in (`plx_os::i18n::local_utc_offset`), both read together once a minute (`zone()`). Its
//! one caller is the LOOP, which publishes the answer once per iteration
//! (`run::publish_frame` -> `Bridge::publish_today`); a screen that words a date ("Today",
//! "Yesterday") RECEIVES it as `AppViews::today` (`LibraryLike::today`) and reads no clock, the way
//! a frame's time arrives on a `Tick`. `unix_ms()` is the playback estimate's wall-clock input: it
//! samples the platform at most every five seconds and advances from monotonic frame ticks. `wall()`
//! is the loop's one publication of it for the screens (`run::publish_frame` ->
//! `Bridge::publish_wall` -> `AppViews::wall`, read through `LiveTvLike::wall`, and into the player's `HudModel::wall`):
//! its Unix second and the OS zone's UTC offset at it, from the same once-a-minute zone read as
//! the date, from which every clock face and "22m left" (and the HUD's finish time) is derived by
//! the pure functions in `plx_ui::fmt`. `now()` reads
//! `SDL_GetTicks` — the only call site of it in `app/`
//! (the §15.2 gate: `SDL_GetTicks(` only in the clock and the instruments) — unless a replay has
//! set the frame's recorded tick, in which case every read in the frame answers that tick: the
//! ingest stamp, each press arm, the HUD linger, a script delay.
//!
//! Phase 2 shape: a thread-local override rather than a `Clock` value threaded through every
//! signature. The loop still reads the clock at several points in a frame (ingest, each key
//! arm, the HUD deadline after a blocking resolve); live, those differ by the microseconds
//! between them exactly as before; under a replay they are all the frame's tick, which is what
//! makes the machines that are machines today replay deterministically. Main-thread only — the
//! loop is the only reader, and a worker that needs wall time uses `Instant` (an adapter).

use std::cell::Cell;

#[cfg(not(test))]
extern "C" {
    fn SDL_GetTicks() -> u32;
}

/// The host test binary links no SDL (`diag::heartbeat` carries the same stub for the
/// performance counter); a test reads the clock only through the replay override.
#[cfg(test)]
#[allow(non_snake_case)]
unsafe fn SDL_GetTicks() -> u32 {
    0
}

thread_local! {
    /// The replay override: `Some(ms)` while a recorded frame is being replayed.
    static REPLAY: Cell<Option<u32>> = const { Cell::new(None) };
    /// The last read of the platform's zone (calendar date and UTC offset) and the minute (of the
    /// frame clock) it was read in.
    static ZONE: Cell<Option<(u32, Zone)>> = const { Cell::new(None) };
    /// The last read of the zone's UTC offset at one other instant (a projected finish) and the Unix
    /// second it was read for. Never dropped: the offset at one second is the zone rules' answer for
    /// that second, which does not change with the minute (see [`utc_offset_at`]).
    static OFFSET_AT: Cell<Option<(i64, Option<i32>)>> = const { Cell::new(None) };
    /// A wall-clock Unix sample and its monotonic frame tick; advanced between five-second refreshes.
    static UNIX: Cell<Option<(u32, Option<i64>)>> = const { Cell::new(None) };
}

/// The date every replay frame answers: a recording has no calendar of its own, and a fixed one keeps
/// a replay of a date-sorted grid identical on every run.
pub(crate) const REPLAY_DATE: [i32; 3] = [2026, 1, 1];

/// Milliseconds since SDL init — or, under a replay, the recorded tick of the frame in flight.
pub(crate) fn now() -> u32 {
    plx_base::step::assert_io_allowed("app::clock::now");
    if let Some(ms) = REPLAY.with(|c| c.get()) {
        return ms;
    }
    // SAFETY: SDL is initialised before the loop that reads the clock exists; the call takes no
    // arguments and touches no memory of ours.
    unsafe { SDL_GetTicks() }
}

/// The replay driver sets the frame's time before the frame reads it, and with it the date
/// ([`REPLAY_DATE`]).
pub(crate) fn set_replay(ms: u32) {
    REPLAY.with(|c| c.set(Some(ms)));
    UNIX.with(|c| c.set(None));
}

/// Unix wall-clock milliseconds for live player estimates. Replays deliberately have no wall clock:
/// inventing today's date for a historical frame would make the HUD nondeterministic. The platform
/// clock is sampled at most every five seconds and advanced with monotonic frame ticks between samples.
pub(crate) fn unix_ms() -> Option<i64> {
    plx_base::step::assert_io_allowed("app::clock::unix_ms");
    if REPLAY.with(|c| c.get().is_some()) {
        UNIX.with(|c| c.set(None));
        return None;
    }
    let tick = now();
    if let Some((at, sampled)) = UNIX.with(|c| c.get()) {
        if let Some(current) = advance_unix_ms(at, sampled, tick) {
            return current;
        }
    }
    let sampled = plx_os::i18n::unix_timestamp_ms();
    UNIX.with(|c| c.set(Some((tick, sampled))));
    sampled
}

/// **The platform's zone as of one read**: the calendar date and the UTC offset at the same
/// Unix second, so the two cannot disagree across a midnight or a clock change.
#[derive(Clone, Copy)]
struct Zone {
    today: Option<[i32; 3]>,
    utc_offset_s: Option<i32>,
}

/// The zone at the Unix second `unix_s`, read from the platform at most once a minute of the frame
/// clock ([`now`]) and shared by [`today`] and [`wall`]: a midnight or a clock change (daylight
/// saving) is noticed within a minute, and a frame pays one thread-local read.
fn zone(unix_s: i64) -> Zone {
    let minute = now() / 60_000;
    if let Some((at, zone)) = ZONE.with(|c| c.get()) {
        if at == minute { return zone; }
    }
    let zone = Zone {
        today: plx_os::i18n::local_calendar_date(unix_s),
        utc_offset_s: plx_os::i18n::local_utc_offset(unix_s),
    };
    ZONE.with(|c| c.set(Some((minute, zone))));
    zone
}

/// **The OS zone's UTC offset at any Unix second** (seconds east of UTC, daylight saving included), for
/// an instant that is not this frame's: the HUD's projected finish time, which may lie past a clock
/// change that [`wall`]'s offset (the one in force now) does not know of. [`zone`]'s per-minute memo
/// answers for NOW only, so this has its own single-entry memo keyed by the second asked for (a steady
/// playback asks for the same finish second frame after frame). The memo needs no expiry: the offset at
/// one Unix second is a pure function of the zone rules the C library loaded, and a changed time-zone
/// SETTING reaches none of this (nothing calls `tzset`, and a reentrant `localtime_r` does not reread
/// the zone on its own), so [`zone`]'s per-minute read exists for the calendar date and for the offset
/// NOW changing with the clock (midnight, a daylight-saving change), not for a new zone.
/// `None` where the platform cannot say. **Only the loop calls it** (through the HUD model).
pub(crate) fn utc_offset_at(unix_s: i64) -> Option<i32> {
    plx_base::step::assert_io_allowed("app::clock::utc_offset_at");
    if let Some((at, offset)) = OFFSET_AT.with(|c| c.get()) {
        if at == unix_s { return offset; }
    }
    let offset = plx_os::i18n::local_utc_offset(unix_s);
    OFFSET_AT.with(|c| c.set(Some((unix_s, offset))));
    offset
}

/// **The wall time of this frame** for the loop to publish (`run::publish_frame` ->
/// `Bridge::publish_wall` -> `AppViews::wall`): the Unix second of [`unix_ms`] (the one wall-clock
/// door) and the OS zone's UTC offset at it ([`zone`], read at most once a minute). `None` where
/// [`unix_ms`] answers none (a replay) or the zone cannot be read: a clock face is never guessed.
/// **Only the loop calls it**; a screen receives the publication and reads no clock.
pub(crate) fn wall() -> Option<plx_ui::fmt::Wall> {
    plx_base::step::assert_io_allowed("app::clock::wall");
    let unix_ms = unix_ms()?;
    Some(plx_ui::fmt::Wall { unix_ms, utc_offset_s: zone(unix_ms.div_euclid(1_000)).utc_offset_s? })
}

/// Advance a sampled wall-clock timestamp by monotonic frame time, or ask for a fresh sample once
/// the cache is five seconds old. Wrapping subtraction keeps SDL's 32-bit tick rollover safe.
fn advance_unix_ms(sampled_at: u32, sampled: Option<i64>, tick: u32) -> Option<Option<i64>> {
    let elapsed_ms = tick.wrapping_sub(sampled_at);
    if elapsed_ms >= 5_000 {
        return None;
    }
    Some(match sampled {
        Some(sampled) => Some(sampled.checked_add(i64::from(elapsed_ms))?),
        None => None,
    })
}

/// **Today's calendar date** in the OS time zone, `[year, month, day]`, from the same once-a-minute
/// zone read as [`wall`]'s offset ([`zone`]). Under a replay (or a test that set one) it is the
/// pinned date. `None` when the platform cannot say (no wall clock, an unrepresentable `time_t`): a
/// date then reads as month and year. Main-thread only, like the clock.
///
/// **Only the loop calls it** (`run::publish_frame`, once per iteration), and publishes the answer
/// into the Bridge's views. A screen or a host impl never calls it: `AppHost::today` returns the
/// frame's publication.
pub(crate) fn today() -> Option<[i32; 3]> {
    plx_base::step::assert_io_allowed("app::clock::today");
    // A recorded Library grid sorted by a date must word the same on every run and on every day.
    if REPLAY.with(|c| c.get()).is_some() {
        return Some(REPLAY_DATE);
    }
    zone(unix_ms()?.div_euclid(1_000)).today
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_machine_cannot_sample_even_the_replay_clock_or_cached_calendar() {
        super::set_replay(4242);
        let _allowed = plx_base::task::allow_blocking(const { &plx_base::task::BlockingLabel::new("fixture") });
        let step = plx_base::step::StepScope::enter();
        for read in [|| { super::now(); }, || { super::unix_ms(); }, || { super::today(); }, || { super::wall(); }, || { super::utc_offset_at(1_795_649_400); }] {
            assert!(std::panic::catch_unwind(read).is_err());
        }
        drop(step);
        assert_eq!(super::now(), 4242);
        assert_eq!(super::today(), Some(super::REPLAY_DATE));
        super::REPLAY.with(|c| c.set(None));
    }

    #[test]
    fn a_replay_tick_overrides_the_clock_for_the_thread() {
        super::set_replay(4242);
        assert_eq!(super::now(), 4242);
        super::REPLAY.with(|c| c.set(None));
    }

    #[test]
    fn unix_wall_time_advances_from_a_monotonic_sample_and_expires() {
        assert_eq!(super::advance_unix_ms(1_000, Some(10_000), 1_999), Some(Some(10_999)));
        assert_eq!(super::advance_unix_ms(1_000, Some(10_000), 2_000), Some(Some(11_000)));
        assert_eq!(super::advance_unix_ms(1_000, Some(10_000), 5_999), Some(Some(14_999)));
        assert_eq!(super::advance_unix_ms(1_000, Some(10_000), 6_000), None);
        assert_eq!(super::advance_unix_ms(1_000, None, 1_999), Some(None));
        assert_eq!(
            super::advance_unix_ms(u32::MAX - 10, Some(10_000), 5),
            Some(Some(10_016)),
            "the monotonic frame clock may wrap"
        );
    }

    #[test]
    fn a_published_date_reaches_a_screen_through_its_views_and_no_clock() {
        use crate::app::bridge::{AppHost, Bridge};
        use plx_machine::machine::{EntryId, InputOwner, Tick};
        use plx_screens::registry::LibraryLike;
        use plx_ui::dispatch::CxParts;
        let mut owner = crate::app::runtime_owners::Harness::new(|| 0);
        let mut bridge = owner.bridge();
        let parts = CxParts { modal: Default::default(), page_image: Default::default(), hold_hint: Default::default(), tick: Tick::default(), press: Default::default(), focus: Default::default(),
            owner: InputOwner::Entry(EntryId(0)) };
        // What a screen sees: a `Cx` over the Bridge's views, asked through the Library's host trait.
        let seen = |bridge: &Bridge| {
            let cx = parts.cx::<AppHost>(
                bridge.views_with(plx_media::route::idle_session_for_test()), &plx_ui::fixture::FixtureMeasure);
            <AppHost as LibraryLike>::today(&cx)
        };
        assert_eq!(bridge.views().today, None);
        assert_eq!(seen(&bridge), None, "before the loop's first publication the host has no date");
        bridge.publish_today(Some([2001, 2, 3]));
        assert_eq!(bridge.views().today, Some([2001, 2, 3]));
        assert_eq!(seen(&bridge), Some([2001, 2, 3]), "a screen receives the published date");
        // The host reads no clock for it: a replay pinning another date changes nothing it answers.
        super::set_replay(100);
        assert_ne!(super::today(), Some([2001, 2, 3]));
        assert_eq!(seen(&bridge), Some([2001, 2, 3]), "the answer is the publication, not `today()`");
        bridge.publish_today(None);
        assert_eq!(seen(&bridge), None, "a publication of nothing is nothing");
        super::REPLAY.with(|c| c.set(None));
    }

    #[test]
    fn a_published_wall_time_reaches_a_screen_through_its_views_and_no_clock() {
        use crate::app::bridge::{AppHost, Bridge};
        use plx_machine::machine::{EntryId, InputOwner, Tick};
        use plx_screens::registry::LiveTvLike;
        use plx_ui::dispatch::CxParts;
        use plx_ui::fmt::Wall;
        let mut owner = crate::app::runtime_owners::Harness::new(|| 0);
        let mut bridge = owner.bridge();
        let parts = CxParts { modal: Default::default(), page_image: Default::default(), hold_hint: Default::default(), tick: Tick::default(), press: Default::default(), focus: Default::default(),
            owner: InputOwner::Entry(EntryId(0)) };
        // What a screen sees, through the host trait that carries it.
        let seen = |bridge: &Bridge| {
            let cx = parts.cx::<AppHost>(
                bridge.views_with(plx_media::route::idle_session_for_test()), &plx_ui::fixture::FixtureMeasure);
            <AppHost as LiveTvLike>::wall(&cx)
        };
        let wall = Wall::from_secs(1_795_649_400, 3_600);
        assert_eq!(bridge.views().wall, None);
        assert_eq!(seen(&bridge), None, "before the loop's first publication the host has no time");
        bridge.publish_wall(Some(wall));
        assert_eq!(bridge.views().wall, Some(wall));
        assert_eq!(bridge.wall(), Some(wall));
        assert_eq!(seen(&bridge), Some(wall), "a screen receives the published time");
        // The host reads no clock for it: a replay (which has no wall clock) changes nothing it answers.
        super::set_replay(100);
        assert_eq!(super::unix_ms(), None);
        assert_eq!(super::wall(), None, "the door itself answers nothing under a replay");
        assert_eq!(seen(&bridge), Some(wall), "the answer is the publication, not the door");
        bridge.publish_wall(None);
        assert_eq!(seen(&bridge), None, "a publication of nothing is nothing");
        super::REPLAY.with(|c| c.set(None));
    }

    #[test]
    fn the_wall_door_pairs_the_wall_clock_with_an_offset_read_once_a_minute() {
        super::REPLAY.with(|c| c.set(None));
        super::UNIX.with(|c| c.set(Some((0, Some(1_795_649_400_999)))));
        super::ZONE.with(|c| c.set(None));
        let live = super::wall().expect("a platform clock and zone");
        assert_eq!(live.unix_ms, 1_795_649_400_999, "the frame's wall clock to the millisecond");
        assert_eq!(Some(live.utc_offset_s), plx_os::i18n::local_utc_offset(1_795_649_400));
        let zone = |today, utc_offset_s| super::Zone { today, utc_offset_s };
        super::ZONE.with(|c| c.set(Some((0, zone(None, Some(-4_500))))));
        assert_eq!(super::wall().map(|w| w.utc_offset_s), Some(-4_500), "within the minute the zone is not read again");
        super::ZONE.with(|c| c.set(Some((7, zone(None, Some(-4_500))))));
        assert_eq!(super::wall().map(|w| w.utc_offset_s), Some(live.utc_offset_s), "a new minute reads it again");
        super::ZONE.with(|c| c.set(Some((0, zone(None, None)))));
        assert_eq!(super::wall(), None, "a zone that cannot be read is no wall time");
        super::UNIX.with(|c| c.set(Some((0, None))));
        assert_eq!(super::wall(), None, "no wall clock, no wall time");
        super::ZONE.with(|c| c.set(None));
        super::UNIX.with(|c| c.set(None));
    }

    #[test]
    fn the_date_and_the_offset_are_one_zone_read_once_a_minute() {
        super::REPLAY.with(|c| c.set(None));
        super::UNIX.with(|c| c.set(Some((0, Some(1_795_649_400_999)))));
        super::ZONE.with(|c| c.set(None));
        // One read answers both; the second call, whichever door asks, is the memo.
        let wall = super::wall().expect("a platform clock and zone");
        let today = super::today();
        assert_eq!(today, plx_os::i18n::local_calendar_date(wall.unix_s()), "the date is of the same second as the offset");
        super::ZONE.with(|c| c.set(Some((0, super::Zone { today: Some([2001, 2, 3]), utc_offset_s: Some(60) }))));
        assert_eq!((super::today(), super::wall().map(|w| w.utc_offset_s)), (Some([2001, 2, 3]), Some(60)));
        super::ZONE.with(|c| c.set(None));
        super::UNIX.with(|c| c.set(None));
    }

    #[test]
    fn the_offset_at_another_instant_is_its_own_lookup_kept_for_that_second() {
        super::REPLAY.with(|c| c.set(None));
        super::OFFSET_AT.with(|c| c.set(None));
        super::ZONE.with(|c| c.set(None));
        const AT: i64 = 1_795_649_400;
        assert_eq!(super::utc_offset_at(AT), plx_os::i18n::local_utc_offset(AT), "the platform's answer for that second");
        assert_eq!(super::utc_offset_at(0), None, "an instant the zone cannot convert is no offset");
        // A second asked for again is the memo, not a new read.
        super::OFFSET_AT.with(|c| c.set(Some((AT, Some(-4_500)))));
        assert_eq!(super::utc_offset_at(AT), Some(-4_500));
        assert_eq!(super::utc_offset_at(AT + 1), plx_os::i18n::local_utc_offset(AT + 1), "another second is another lookup");
        // The minute's zone read is a different memo and leaves it alone.
        super::OFFSET_AT.with(|c| c.set(Some((AT, Some(-4_500)))));
        super::ZONE.with(|c| c.set(None));
        super::UNIX.with(|c| c.set(Some((0, Some(AT * 1_000)))));
        assert!(super::wall().is_some(), "the zone was read again");
        assert_eq!(super::OFFSET_AT.with(|c| c.get()), Some((AT, Some(-4_500))));
        super::OFFSET_AT.with(|c| c.set(None));
        super::ZONE.with(|c| c.set(None));
        super::UNIX.with(|c| c.set(None));
    }

    #[test]
    fn the_date_is_pinned_under_a_replay_and_read_once_a_minute_otherwise() {
        super::REPLAY.with(|c| c.set(None));
        super::ZONE.with(|c| c.set(None));
        super::UNIX.with(|c| c.set(None));
        // Live: the platform's date, cached for the minute (the host test's frame clock stands at 0).
        let live = super::today();
        assert!(live.is_some_and(|[year, month, day]| year >= 2026 && (1..=12).contains(&month) && (1..=31).contains(&day)));
        let zone = |today| super::Zone { today, utc_offset_s: None };
        super::ZONE.with(|c| c.set(Some((0, zone(Some([2001, 2, 3]))))));
        assert_eq!(super::today(), Some([2001, 2, 3]), "within the minute the read is not repeated");
        super::ZONE.with(|c| c.set(Some((7, zone(Some([2001, 2, 3]))))));
        assert_eq!(super::today(), live, "a new minute reads again");
        // A replay frame pins the date, whatever the wall clock says.
        super::set_replay(100);
        assert_eq!(super::today(), Some(super::REPLAY_DATE));
        assert_eq!(super::unix_ms(), None, "a replay has no present-day wall-clock finish");
        super::REPLAY.with(|c| c.set(None));
        super::ZONE.with(|c| c.set(None));
    }
}
