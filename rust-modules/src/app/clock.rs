//! The application's clock (restructure spec §4.1): ONE door to frame time, so a replay can drive
//! the loop on recorded ticks. `today()` is the same door for the calendar date: the ONE place the
//! application asks the platform what day it is (`plx_os::i18n::local_calendar_today`), once a
//! minute. Its one caller is the LOOP, which publishes the answer once per iteration
//! (`run::publish_frame` -> `Bridge::publish_today`); a screen that words a date ("Today",
//! "Yesterday") RECEIVES it as `AppViews::today` (`LibraryLike::today`) and reads no clock, the way
//! a frame's time arrives on a `Tick`. `unix_ms()` is the playback estimate's wall-clock input: it
//! samples the platform at most every five seconds and advances from monotonic frame ticks. `wall()`
//! is the loop's one publication of it for the screens (`run::publish_frame` ->
//! `Bridge::publish_wall` -> `AppViews::wall`, read through `LiveTvLike::wall` / `PlayerLike::wall`):
//! its Unix second and the OS zone's UTC offset at it, read once a minute, from which every clock
//! face and "22m left" is derived by the pure functions in `plx_ui::fmt`. `now()` reads
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
    /// The last read of the platform's calendar date and the minute (of the frame clock) it was read in.
    static TODAY: Cell<Option<(u32, Option<[i32; 3]>)>> = const { Cell::new(None) };
    /// A wall-clock Unix sample and its monotonic frame tick; advanced between five-second refreshes.
    static UNIX: Cell<Option<(u32, Option<i64>)>> = const { Cell::new(None) };
    static LOCAL_END: Cell<Option<(i64, Option<[i32; 2]>)>> = const { Cell::new(None) };
    /// The last read of the OS zone's UTC offset and the minute (of the frame clock) it was read in.
    static OFFSET: Cell<Option<(u32, Option<i32>)>> = const { Cell::new(None) };
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
    LOCAL_END.with(|c| c.set(None));
    let sampled = plx_os::i18n::unix_timestamp_ms();
    UNIX.with(|c| c.set(Some((tick, sampled))));
    sampled
}

/// Cache timezone conversion until the projected second changes or the wall clock is refreshed.
pub(crate) fn local_end_time(seconds: i64) -> Option<[i32; 2]> {
    LOCAL_END.with(|cache| {
        if let Some((_, local)) = cache.get().filter(|(cached, _)| *cached == seconds) {
            return local;
        }
        let local = plx_os::i18n::local_clock_time(seconds);
        cache.set(Some((seconds, local)));
        local
    })
}

/// **The wall time of this frame** for the loop to publish (`run::publish_frame` ->
/// `Bridge::publish_wall` -> `AppViews::wall`): the Unix second of [`unix_ms`] (the one wall-clock
/// door, so this frame's value is the one the Live TV store's `now` is taken from) and the OS zone's
/// UTC offset at it, read at most once a minute of the frame clock ([`now`]) like [`today`]. A clock
/// change (daylight saving) is therefore noticed within a minute. `None` where [`unix_ms`] answers
/// none (a replay) or the zone cannot be read: a clock face is never guessed. **Only the loop calls
/// it**; a screen receives the publication and reads no clock.
pub(crate) fn wall() -> Option<plx_ui::fmt::Wall> {
    plx_base::step::assert_io_allowed("app::clock::wall");
    let unix_s = unix_ms()?.div_euclid(1_000);
    let minute = now() / 60_000;
    let utc_offset_s = match OFFSET.with(|c| c.get()) {
        Some((at, offset)) if at == minute => offset,
        _ => {
            let offset = local_utc_offset(unix_s);
            OFFSET.with(|c| c.set(Some((minute, offset))));
            offset
        }
    }?;
    Some(plx_ui::fmt::Wall { unix_s, utc_offset_s })
}

/// **The OS zone's UTC offset at a Unix second**, in seconds east of UTC (daylight saving included),
/// derived from the two reads the os layer already has: the local hour:minute
/// ([`plx_os::i18n::local_clock_time`]) minus the UTC one, plus a day when the local calendar date
/// ([`plx_os::i18n::local_calendar_date`]) is not the UTC one. The date is what tells +13 from -11
/// and +14 from -10, which an hour:minute alone cannot. Whole-minute offsets only (every zone in
/// use). Two zone reads: [`wall`] caches the answer for a minute, and nothing else calls this.
fn local_utc_offset(unix_s: i64) -> Option<i32> {
    utc_offset_between(unix_s, plx_os::i18n::local_clock_time(unix_s)?, plx_os::i18n::local_calendar_date(unix_s)?)
}

/// [`local_utc_offset`] from the reads it is made of, so the arithmetic is testable without a zone.
/// The seconds of the minute are the same in the local and the UTC face, so they cancel. An answer
/// beyond a day's travel from UTC is a face that cannot belong to `unix_s`: `None`.
fn utc_offset_between(unix_s: i64, [hour, minute]: [i32; 2], [year, month, day]: [i32; 3]) -> Option<i32> {
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) { return None; }
    let utc_second = unix_s.rem_euclid(86_400);
    let local_second = i64::from(hour) * 3_600 + i64::from(minute) * 60 + utc_second % 60;
    let local_day = days_from_civil(year, month, day);
    let offset = (local_day - unix_s.div_euclid(86_400)) * 86_400 + local_second - utc_second;
    (offset.abs() <= 15 * 3_600).then(|| i32::try_from(offset).ok()).flatten()
}

/// Days since 1970-01-01 of a proleptic Gregorian civil date (Howard Hinnant's `days_from_civil`).
fn days_from_civil(year: i32, month: i32, day: i32) -> i64 {
    let y = i64::from(year) - i64::from(month <= 2);
    let era = y.div_euclid(400);
    let year_of_era = y - era * 400;
    let day_of_year = (153 * (i64::from(month) + if month > 2 { -3 } else { 9 }) + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
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

/// **Today's calendar date** in the OS time zone, `[year, month, day]`: read from the platform at
/// most once a minute of the frame clock ([`now`]), so a midnight is noticed within a minute and a
/// frame pays one thread-local read. Under a replay (or a test that set one) it is the pinned date.
/// `None` when the platform cannot say (an unrepresentable `time_t`): a date then reads as month
/// and year. Main-thread only, like the clock.
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
    let minute = now() / 60_000;
    if let Some((at, date)) = TODAY.with(|c| c.get()) {
        if at == minute { return date; }
    }
    let date = plx_os::i18n::local_calendar_today();
    TODAY.with(|c| c.set(Some((minute, date))));
    date
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_machine_cannot_sample_even_the_replay_clock_or_cached_calendar() {
        super::set_replay(4242);
        let _allowed = plx_base::task::allow_blocking(const { &plx_base::task::BlockingLabel::new("fixture") });
        let step = plx_base::step::StepScope::enter();
        for read in [|| { super::now(); }, || { super::unix_ms(); }, || { super::today(); }, || { super::wall(); }] {
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
        use plx_screens::registry::{LiveTvLike, PlayerLike};
        use plx_ui::dispatch::CxParts;
        use plx_ui::fmt::Wall;
        let mut owner = crate::app::runtime_owners::Harness::new(|| 0);
        let mut bridge = owner.bridge();
        let parts = CxParts { modal: Default::default(), page_image: Default::default(), hold_hint: Default::default(), tick: Tick::default(), press: Default::default(), focus: Default::default(),
            owner: InputOwner::Entry(EntryId(0)) };
        // What a screen sees, through both host traits that carry it.
        let seen = |bridge: &Bridge| {
            let cx = parts.cx::<AppHost>(
                bridge.views_with(plx_media::route::idle_session_for_test()), &plx_ui::fixture::FixtureMeasure);
            (<AppHost as LiveTvLike>::wall(&cx), <AppHost as PlayerLike>::wall(&cx))
        };
        let wall = Wall { unix_s: 1_795_649_400, utc_offset_s: 3_600 };
        assert_eq!(bridge.views().wall, None);
        assert_eq!(seen(&bridge), (None, None), "before the loop's first publication the host has no time");
        bridge.publish_wall(Some(wall));
        assert_eq!(bridge.views().wall, Some(wall));
        assert_eq!(bridge.wall(), Some(wall));
        assert_eq!(seen(&bridge), (Some(wall), Some(wall)), "a screen receives the published time");
        // The host reads no clock for it: a replay (which has no wall clock) changes nothing it answers.
        super::set_replay(100);
        assert_eq!(super::unix_ms(), None);
        assert_eq!(super::wall(), None, "the door itself answers nothing under a replay");
        assert_eq!(seen(&bridge), (Some(wall), Some(wall)), "the answer is the publication, not the door");
        bridge.publish_wall(None);
        assert_eq!(seen(&bridge), (None, None), "a publication of nothing is nothing");
        super::REPLAY.with(|c| c.set(None));
    }

    #[test]
    fn the_utc_offset_is_the_local_face_minus_the_utc_face_with_the_day_difference_kept() {
        use super::utc_offset_between as offset;
        // 2026-11-25 23:30:00 UTC.
        const T: i64 = 1_795_649_400;
        assert_eq!(offset(T, [23, 30], [2026, 11, 25]), Some(0));
        assert_eq!(offset(T, [0, 30], [2026, 11, 26]), Some(3_600), "past UTC midnight");
        assert_eq!(offset(T, [17, 30], [2026, 11, 25]), Some(-6 * 3_600));
        assert_eq!(offset(T, [5, 0], [2026, 11, 26]), Some(19_800), "a half-hour zone");
        assert_eq!(offset(T + 15, [5, 0], [2026, 11, 26]), Some(19_800), "the seconds are the same in both");
        // 2026-11-26 00:10 UTC is still the 25th for the Americas.
        assert_eq!(offset(1_795_651_800, [19, 10], [2026, 11, 25]), Some(-5 * 3_600));
        // 2026-01-01 00:00 UTC is the evening of the year's last day there.
        assert_eq!(offset(1_767_225_600, [19, 0], [2025, 12, 31]), Some(-5 * 3_600));
        // The zones a day apart from their opposites: +13 is not -11, +14 is not -10.
        assert_eq!(offset(1_767_268_800, [1, 0], [2026, 1, 2]), Some(13 * 3_600), "2026-01-01 12:00 UTC");
        assert_eq!(offset(1_767_312_000, [13, 0], [2026, 1, 1]), Some(-11 * 3_600), "2026-01-02 00:00 UTC");
        assert_eq!(offset(1_767_268_800, [2, 0], [2026, 1, 2]), Some(14 * 3_600));
        // A face the calendar cannot have, or a day no zone is, is no offset.
        assert_eq!(offset(T, [0, 30], [2026, 13, 26]), None);
        assert_eq!(offset(T, [0, 30], [2026, 11, 29]), None, "four days ahead is not a zone");
    }

    #[test]
    fn the_utc_offset_follows_the_os_timezone_including_its_clock_changes() {
        const CHILD: &str = "PLX_LOCAL_UTC_OFFSET_CASE";
        if let Ok(zone) = std::env::var(CHILD) {
            let hours = |h: i32| h * 3_600;
            let cases: &[(i64, i32)] = match zone.as_str() {
                "UTC" => &[(1_795_649_400, 0)],
                // Spring-forward: 01:59:59 CET (+1) is followed by 03:00 CEST (+2); fall-back at 01:00 UTC.
                "Europe/Madrid" => &[(1_774_745_999, hours(1)), (1_774_746_000, hours(2)), (1_792_889_999, hours(2)), (1_792_890_000, hours(1))],
                "Asia/Kolkata" => &[(1_795_649_400, 19_800)],
                "America/Los_Angeles" => &[(1_795_651_800, hours(-8))],
                "Pacific/Auckland" => &[(1_767_268_800, hours(13))],
                "Pacific/Pago_Pago" => &[(1_767_312_000, hours(-11))],
                "Pacific/Kiritimati" => &[(1_767_268_800, hours(14))],
                other => panic!("no cases for {other}"),
            };
            for &(at, want) in cases {
                assert_eq!(super::local_utc_offset(at), Some(want), "{zone} at {at}");
            }
            assert_eq!(super::local_utc_offset(0), None);
            assert_eq!(super::local_utc_offset(i64::MAX), None);
            return;
        }
        let name = format!("{}::the_utc_offset_follows_the_os_timezone_including_its_clock_changes",
            module_path!().split_once("::").unwrap().1);
        for zone in ["UTC", "Europe/Madrid", "Asia/Kolkata", "America/Los_Angeles", "Pacific/Auckland", "Pacific/Pago_Pago", "Pacific/Kiritimati"] {
            let out = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", &name, "--test-threads=1"])
                .env("TZ", zone).env(CHILD, zone).output().unwrap();
            let stdout = String::from_utf8_lossy(&out.stdout);
            assert!(out.status.success() && stdout.contains("1 passed"),
                "the UTC offset failed under {zone}:\n{stdout}\n{}", String::from_utf8_lossy(&out.stderr));
        }
    }

    #[test]
    fn the_wall_door_pairs_the_wall_second_with_an_offset_read_once_a_minute() {
        super::REPLAY.with(|c| c.set(None));
        super::UNIX.with(|c| c.set(Some((0, Some(1_795_649_400_999)))));
        super::OFFSET.with(|c| c.set(None));
        let live = super::wall().expect("a platform clock and zone");
        assert_eq!(live.unix_s, 1_795_649_400, "whole seconds of the frame's wall clock");
        assert_eq!(live.utc_offset_s, super::local_utc_offset(1_795_649_400).unwrap());
        super::OFFSET.with(|c| c.set(Some((0, Some(-4_500)))));
        assert_eq!(super::wall().map(|w| w.utc_offset_s), Some(-4_500), "within the minute the zone is not read again");
        super::OFFSET.with(|c| c.set(Some((7, Some(-4_500)))));
        assert_eq!(super::wall().map(|w| w.utc_offset_s), Some(live.utc_offset_s), "a new minute reads it again");
        super::UNIX.with(|c| c.set(Some((0, None))));
        assert_eq!(super::wall(), None, "no wall clock, no wall time");
        super::OFFSET.with(|c| c.set(None));
        super::UNIX.with(|c| c.set(None));
    }

    #[test]
    fn the_date_is_pinned_under_a_replay_and_read_once_a_minute_otherwise() {
        super::REPLAY.with(|c| c.set(None));
        super::TODAY.with(|c| c.set(None));
        // Live: the platform's date, cached for the minute (the host test's frame clock stands at 0).
        let live = super::today();
        assert!(live.is_some_and(|[year, month, day]| year >= 2026 && (1..=12).contains(&month) && (1..=31).contains(&day)));
        super::TODAY.with(|c| c.set(Some((0, Some([2001, 2, 3])))));
        assert_eq!(super::today(), Some([2001, 2, 3]), "within the minute the read is not repeated");
        super::TODAY.with(|c| c.set(Some((7, Some([2001, 2, 3])))));
        assert_eq!(super::today(), live, "a new minute reads again");
        // A replay frame pins the date, whatever the wall clock says.
        super::set_replay(100);
        assert_eq!(super::today(), Some(super::REPLAY_DATE));
        assert_eq!(super::unix_ms(), None, "a replay has no present-day wall-clock finish");
        super::REPLAY.with(|c| c.set(None));
        super::TODAY.with(|c| c.set(None));
    }
}
