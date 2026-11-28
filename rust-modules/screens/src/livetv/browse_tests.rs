use super::*;
use plx_data::livetv::{Airings, LiveAiring};
use plx_plex::plex::ServerId;

const NOW: i64 = 1_795_638_600; // 2026-11-24 20:30:00 UTC
const MIN: i64 = 60;

fn airing(title: &str, from: i64, to: i64) -> LiveAiring {
    LiveAiring::new(title, NOW + from * MIN, NOW + to * MIN)
}

/// A channel airing `now` from `from` to `to` minutes around the instant (`None`: nothing on), then `later`.
fn channel(vcn: &str, now: Option<(&str, i64, i64)>, later: Vec<LiveAiring>) -> Channel {
    let airings = match now {
        Some((title, from, to)) => Airings::on_then(airing(title, from, to), later),
        None => Airings::default().with_later(later),
    };
    Channel {
        sid: ServerId::from_raw(1), dvr: "7".into(), channel: format!("C{vcn}"), vcn: vcn.into(), name: format!("Channel {vcn}"),
        thumb: String::new(), airings,
    }
}

/// `channel` with the programme on now given other times.
fn retimed(channel: Channel, begins_at: i64, ends_at: i64) -> Channel {
    Channel { airings: channel.airings.clone().retimed(begins_at, ends_at), ..channel }
}

fn titles(channels: &[Channel], picks: &[Pick]) -> Vec<String> {
    picks.iter().map(|p| channels[p.channel].airings.all()[p.airing].title.clone()).collect()
}

fn on(from: i64, to: i64) -> Vec<Channel> {
    vec![channel("1", Some(("A", from, to)), vec![])]
}

fn shows(filter: Filter, channels: &[Channel]) -> bool {
    !shown(channels, filter, NOW, day_end(NOW, Some(0))).is_empty()
}

// ---- each filter's predicate at its edges ------------------------------------------------------------

#[test]
fn on_now_is_begun_and_not_over_at_both_edges() {
    assert!(shows(Filter::OnNow, &on(0, 30)), "begins this very second: on");
    assert!(shows(Filter::OnNow, &on(-30, 1)), "one minute left");
    assert!(!shows(Filter::OnNow, &on(-30, 0)), "ends this very second: over");
    assert!(!shows(Filter::OnNow, &[channel("1", None, vec![airing("Soon", 1, 30)])]), "not begun is not on");
}

#[test]
fn a_programme_with_no_known_end_is_on_once_begun_but_never_ending_soon() {
    let open = retimed(channel("1", Some(("A", -5, 0)), vec![]), NOW - 5 * MIN, 0);
    assert!(shows(Filter::OnNow, std::slice::from_ref(&open)));
    assert!(shows(Filter::JustStarted, std::slice::from_ref(&open)), "five minutes in");
    assert!(!shows(Filter::EndingSoon, std::slice::from_ref(&open)), "an end nobody knows is not soon");
}

#[test]
fn just_started_is_ten_whole_minutes_or_less_in() {
    let started = |seconds_ago: i64| vec![retimed(channel("1", Some(("A", 0, 60)), vec![]), NOW - seconds_ago, NOW + 3_600)];
    assert!(shows(Filter::JustStarted, &started(0)), "begins this second");
    assert!(shows(Filter::JustStarted, &started(10 * MIN)), "ten minutes ago");
    assert!(shows(Filter::JustStarted, &started(11 * MIN - 1)), "the tenth minute lasts until the eleventh begins");
    assert!(!shows(Filter::JustStarted, &started(11 * MIN)), "eleven minutes ago");
    assert!(shows(Filter::OnNow, &started(11 * MIN)), "still on");
}

#[test]
fn ending_soon_is_fifteen_minutes_or_less_left() {
    let ends_in = |seconds: i64| vec![retimed(channel("1", Some(("A", -60, 0)), vec![]), NOW - 3_600, NOW + seconds)];
    assert!(shows(Filter::EndingSoon, &ends_in(1)), "one second left");
    assert!(shows(Filter::EndingSoon, &ends_in(15 * MIN)), "fifteen minutes left");
    assert!(!shows(Filter::EndingSoon, &ends_in(15 * MIN + 1)), "the sixteenth minute");
    assert!(!shows(Filter::EndingSoon, &ends_in(0)), "ended: over, not ending");
}

#[test]
fn later_today_begins_after_now_and_before_the_end_of_the_day() {
    let end = day_end(NOW, Some(0));
    let later = |begin: i64| vec![channel("1", None, vec![LiveAiring { begins_at: begin, ends_at: begin + 1_800, ..airing("L", 0, 0) }])];
    let in_day = |c: &[Channel]| !shown(c, Filter::LaterToday, NOW, end).is_empty();
    assert!(in_day(&later(NOW + 1)), "one second from now");
    assert!(!in_day(&later(NOW)), "begins now: it is on, not later");
    assert!(in_day(&later(end - 1)), "the last second of the day");
    assert!(!in_day(&later(end)), "midnight is tomorrow");
}

#[test]
fn a_programme_is_judged_by_the_clock_not_by_the_list_it_came_from() {
    // read earlier: A was on and B was next. Now A is over and B has begun.
    let channels = vec![channel("1", Some(("A", -40, -10)), vec![airing("B", -10, 20)])];
    let picks = shown(&channels, Filter::OnNow, NOW, day_end(NOW, Some(0)));
    assert_eq!(titles(&channels, &picks), ["B"], "the later entry that has begun is on, the airing that ended is not");
    assert!(shown(&channels, Filter::LaterToday, NOW, day_end(NOW, Some(0))).is_empty());
}

// ---- the day ----------------------------------------------------------------------------------------

#[test]
fn the_day_ends_at_the_next_local_midnight() {
    let three_east = 3 * 3_600;
    // 20:30 UTC is 23:30 at +3: the day ends in half an hour
    assert_eq!(day_end(NOW, Some(three_east)), NOW + 30 * MIN);
    // 00:00:00 local is the first second of a day, which ends 24 h on
    let midnight = NOW + 30 * MIN;
    assert_eq!(day_end(midnight, Some(three_east)), midnight + 86_400);
    assert_eq!(day_end(midnight - 1, Some(three_east)), midnight);
    // west of Greenwich: 20:30 UTC is 15:30 at -5
    assert_eq!(day_end(NOW, Some(-5 * 3_600)), NOW + 8 * 3_600 + 30 * MIN);
    // no zone: the day is the UTC day
    assert_eq!(day_end(NOW, None), NOW + 3 * 3_600 + 30 * MIN);
    assert!(day_end(NOW, Some(0)) > NOW, "a day always ends after the instant");
}

// ---- order, one card a programme, nothing capped ---------------------------------------------------------

#[test]
fn cards_are_one_a_programme_in_channel_then_start_order() {
    let channels = vec![
        channel("1", Some(("1-now", -10, 20)), vec![airing("1-b", 20, 50), airing("1-c", 50, 80)]),
        channel("2", None, vec![airing("2-a", 5, 35)]),
        channel("3", Some(("3-now", -60, 5)), vec![airing("3-b", 5, 65)]),
    ];
    let end = day_end(NOW, Some(0));
    assert_eq!(titles(&channels, &shown(&channels, Filter::OnNow, NOW, end)), ["1-now", "3-now"]);
    assert_eq!(titles(&channels, &shown(&channels, Filter::LaterToday, NOW, end)), ["1-b", "1-c", "2-a", "3-b"]);
}

#[test]
fn every_programme_of_a_long_lineup_is_a_card() {
    let channels: Vec<Channel> = (0..700).map(|n| channel(&n.to_string(), Some(("A", -5, 25)), vec![])).collect();
    assert_eq!(shown(&channels, Filter::OnNow, NOW, day_end(NOW, Some(0))).len(), 700);
}

// ---- the count -----------------------------------------------------------------------------------------

#[test]
fn only_the_filters_that_narrow_on_now_read_a_count_and_it_never_passes_the_total() {
    let channels = vec![
        channel("1", Some(("fresh", -3, 57)), vec![airing("later", 57, 90)]),
        channel("2", Some(("ending", -50, 10)), vec![]),
        channel("3", Some(("middle", -30, 30)), vec![]),
    ];
    let end = day_end(NOW, Some(0));
    let total = on_now(&channels, NOW);
    assert_eq!(total, 3, "a later programme is not on");
    let read = |f: Filter| count(f, shown(&channels, f, NOW, end).len(), total);
    assert_eq!(read(Filter::OnNow), None, "the whole reads no count");
    assert_eq!(read(Filter::JustStarted), Some((1, 3)));
    assert_eq!(read(Filter::EndingSoon), Some((1, 3)));
    assert_eq!(read(Filter::LaterToday), None, "a different set, which can be larger than the channel count");
    for f in Filter::ALL {
        if let Some((shown, total)) = read(f) { assert!(shown <= total, "{f:?}"); }
    }
}

#[test]
fn an_empty_filter_still_counts_zero_of_the_total() {
    let channels = on(-30, 30);
    let none = shown(&channels, Filter::JustStarted, NOW, day_end(NOW, Some(0)));
    assert!(none.is_empty());
    assert_eq!(count(Filter::JustStarted, none.len(), on_now(&channels, NOW)), Some((0, 1)));
}

// ---- when the answer changes by itself -----------------------------------------------------------------

#[test]
fn the_shown_set_changes_by_itself_exactly_at_the_next_boundary() {
    let channels = vec![channel("1", Some(("A", -5, 40)), vec![airing("B", 40, 70)])];
    let end = day_end(NOW, Some(0));
    // A began 5 min ago: just started until it is 11 whole minutes in (in 6 minutes), ending soon from 15 minutes left (in 25)
    let next = next_change(&channels, NOW, end);
    assert_eq!(next, NOW + 6 * MIN, "A stops being just started");
    let before = |f| shown(&channels, f, next - 1, end);
    let after = |f| shown(&channels, f, next, end);
    assert_eq!(before(Filter::JustStarted).len(), 1);
    assert!(after(Filter::JustStarted).is_empty());
    // walking the boundaries: every set is constant between them and every boundary changes some filter
    let mut at = NOW;
    let mut seen = vec![];
    while at < NOW + 100 * MIN {
        let next = next_change(&channels, at, end);
        if next > NOW + 100 * MIN { break; }
        for f in Filter::ALL {
            assert_eq!(shown(&channels, f, at, end), shown(&channels, f, next - 1, end), "{f:?} is constant from {at} to {next}");
        }
        seen.push(next - NOW);
        at = next;
    }
    // A: +11 min in, 15 min left, its end; B: its start (A's end), +11 min in, 15 min left, its end
    assert_eq!(seen, [6 * MIN, 25 * MIN, 40 * MIN, 51 * MIN, 55 * MIN, 70 * MIN].to_vec());
}

#[test]
fn the_end_of_the_day_is_a_boundary() {
    let channels = vec![channel("1", None, vec![airing("L", 60, 90)])];
    let end = NOW + 30 * MIN;
    assert_eq!(next_change(&channels, NOW, end), end, "the day turns before the programme begins");
    assert_eq!(next_change(&[], NOW, end), end);
    assert_eq!(next_change(&[], NOW, NOW), i64::MAX, "a day that has ended has no boundary ahead");
}

// ---- a seat whose card has left ------------------------------------------------------------------------

#[test]
fn a_seat_lands_on_its_channels_card_airing_now_else_where_the_channel_would_stand() {
    let channels = vec![
        channel("1", Some(("1", -10, 20)), vec![]),
        channel("2", Some(("2", -10, 20)), vec![airing("2-b", 20, 50)]),
        channel("3", None, vec![]),
        channel("4", Some(("4", -10, 20)), vec![]),
    ];
    let end = day_end(NOW, Some(0));
    let on_now = shown(&channels, Filter::OnNow, NOW, end);
    assert_eq!(seat_for(&on_now, 1, NOW), Some(1), "channel 2's airing");
    assert_eq!(seat_for(&on_now, 2, NOW), Some(2), "channel 3 shows nothing: the next channel's card takes its place");
    assert_eq!(seat_for(&on_now, 9, NOW), Some(2), "past the last channel: the last card");
    let later = shown(&channels, Filter::LaterToday, NOW, end);
    assert_eq!(seat_for(&later, 1, NOW), Some(0), "only a later card of the channel: that one");
    assert_eq!(seat_for(&later, 0, NOW), Some(0), "an earlier channel with none: the first card after it");
    assert_eq!(seat_for(&[], 1, NOW), None, "nothing shown, nothing to seat");
}

#[test]
fn a_channel_with_an_airing_and_a_later_card_seats_the_airing() {
    let channels = vec![channel("1", Some(("on", -10, 20)), vec![airing("next", 20, 50)])];
    let end = day_end(NOW, Some(0));
    let (airing, later) = (shown(&channels, Filter::OnNow, NOW, end)[0], shown(&channels, Filter::LaterToday, NOW, end)[0]);
    assert_eq!((airing.airing, later.airing), (0, 1));
    assert_eq!(seat_for(&[later, airing], 0, NOW), Some(1));
}

#[test]
fn filter_codes_round_trip_and_the_default_is_on_now() {
    for (row, f) in Filter::ALL.into_iter().enumerate() { assert_eq!(f.code() as usize, row); }
    assert_eq!(Filter::default(), Filter::OnNow);
}

/// **A programme that has aired is no card under any filter**, and the ones that are on or to come are what they were.
#[test]
fn a_programme_that_has_aired_is_no_card_under_any_filter() {
    let aired = Channel {
        airings: Airings::of(vec![airing("Aired", -50, -5), airing("On", -5, 30), airing("Next", 30, 60)], NOW),
        ..channel("1", None, vec![])
    };
    let end = day_end(NOW, Some(0));
    let all = [aired];
    for filter in Filter::ALL {
        let titles = titles(&all, &shown(&all, filter, NOW, end));
        assert!(!titles.iter().any(|t| t == "Aired"), "{filter:?}: {titles:?}");
    }
    assert_eq!(titles(&all, &shown(&all, Filter::OnNow, NOW, end)), ["On"]);
    assert_eq!(titles(&all, &shown(&all, Filter::LaterToday, NOW, end)), ["Next"]);
    assert_eq!(on_now(&all, NOW), 1);
}
