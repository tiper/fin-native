//! **Live TV's Browse, as pure decisions** (PR7; handoff `SPEC-6`): which programmes the Browse grid shows under a
//! [`Filter`], in what order, how many of how many the header reads, and where a seat lands when the card it named is
//! gone. Nothing here reads a clock, a focus or a store: the page hands in the lineup, the instant and the end of the
//! local day, and the same input gives the same answer.
//!
//! **A card is a programme**, not a channel: one for each programme of a channel's guide row the store has read
//! (`Channel::airings`: the ones that have aired, the one on now and the later ones), in channel order and then start
//! order. What a programme IS right now is decided by the clock and its own times alone ([`stand`]), never by where
//! the read put it: a programme that has begun since the read is on now, and one that has ended since is over, exactly as
//! the Guide's `Board::tunes` reads them. One that is over is no card under any filter.
//!
//! ```text
//!   OnNow         begun and not over                       (an airing with no known end is on while it has begun)
//!   JustStarted   OnNow, at most 10 whole minutes in        (`fmt::phase`'s JustStarted: the tenth minute lasts to the eleventh)
//!   EndingSoon    OnNow, at most 15 minutes left            (whole minutes, rounded up: 15:00 is in, 15:01 is out)
//!   LaterToday    not begun, and begins before the end of the local day
//! ```
//!
//! **The count.** JustStarted and EndingSoon are subsets of OnNow, so they read "shown of on now" ([`count`]) and the shown
//! count can never pass the total. OnNow is the grid's whole and LaterToday a different set (it can hold more cards than
//! there are channels), so neither reads a count.
//!
//! **What changes the answer** with no new data: the clock crossing one of a programme's boundaries ([`next_change`]: a start,
//! ten minutes after it, fifteen before the end, the end) or the day's end. The page rebuilds when the instant reaches it.
use plx_data::livetv::Channel;
use plx_ui::fmt::{self, AiringPhase};

/// How many minutes before its end a programme is "ending soon" (SPEC-6 default: 15).
pub const ENDING_SOON_MINUTES: i64 = 15;
/// Seconds in a day: the length of the day an instant with no zone is read in.
pub(super) const DAY: i64 = 86_400;

/// **Which programmes Browse shows.** The page's one filter; it is the scope's (`Scope::Browse(Filter)`), so a scope that
/// is not Browse has no filter to disagree with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Filter {
    /// Every programme on air. The default, and the grid's whole.
    #[default]
    OnNow,
    /// What began 10 minutes ago or less.
    JustStarted,
    /// What ends in 15 minutes or less.
    EndingSoon,
    /// What begins after now and before the end of the local day.
    LaterToday,
}

impl Filter {
    /// The menu's rows, top to bottom.
    pub const ALL: [Filter; 4] = [Filter::OnNow, Filter::JustStarted, Filter::EndingSoon, Filter::LaterToday];

    /// The filter's place in [`ALL`](Self::ALL): its menu row and its canonical code.
    pub const fn code(self) -> u32 {
        match self {
            Filter::OnNow => 0,
            Filter::JustStarted => 1,
            Filter::EndingSoon => 2,
            Filter::LaterToday => 3,
        }
    }

    /// Does the filter narrow the grid's whole (what is on now)? Only then does the header read "shown of total".
    pub const fn narrows(self) -> bool {
        matches!(self, Filter::JustStarted | Filter::EndingSoon)
    }

    /// Does a programme that stands at `stand` belong to the filter, with the day ending at `day_end`?
    pub fn admits(self, stand: Stand, begins_at: i64, day_end: i64) -> bool {
        match (self, stand) {
            (Filter::OnNow, Stand::OnAir { .. }) => true,
            (Filter::JustStarted, Stand::OnAir { minutes_in, .. }) => minutes_in <= fmt::JUST_STARTED_MINUTES,
            (Filter::EndingSoon, Stand::OnAir { left: Some(left), .. }) => left <= ENDING_SOON_MINUTES,
            (Filter::LaterToday, Stand::Upcoming) => begins_at < day_end,
            _ => false,
        }
    }
}

/// **Where a programme stands against the clock.** Made by [`stand`]; the one decision every filter reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stand {
    /// Not begun.
    Upcoming,
    /// On air: whole minutes since the start, and whole minutes left (rounded up, at least one) when the guide knows the end.
    OnAir { minutes_in: i64, left: Option<i64> },
    /// Over: the clock has reached its end.
    Over,
}

/// Where a programme from `begin` to `end` (Unix seconds; `end` 0 when the guide has none) stands at `now`.
pub fn stand(begin: i64, end: i64, now: i64) -> Stand {
    if now < begin {
        return Stand::Upcoming;
    }
    if end == 0 {
        return Stand::OnAir { minutes_in: (now - begin) / 60, left: None };
    }
    match fmt::phase(begin, end, now) {
        AiringPhase::JustStarted { minutes, left } => Stand::OnAir { minutes_in: minutes, left: Some(left) },
        AiringPhase::Running { left } => Stand::OnAir { minutes_in: (now - begin) / 60, left: Some(left) },
        AiringPhase::Over => Stand::Over,
        // a guide with an end that is not after the start has no airing: nothing to show, and nothing to tune
        AiringPhase::Unknown => Stand::Over,
        AiringPhase::Upcoming { .. } => Stand::Upcoming,
    }
}

/// **One card of the grid**: a programme of a lineup channel. Its identity is the channel and the start, never its place in
/// the shown list or in the channel's airings (both shift when the head programme ends).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pick {
    /// The channel's index in the lineup.
    pub channel: usize,
    /// Where its words come from: its place in the channel's [`Airings`](plx_data::livetv::Airings).
    pub airing: usize,
    pub begins_at: i64,
    pub ends_at: i64,
}

/// A channel's programmes in start order.
fn programmes(channel: &Channel, index: usize) -> impl Iterator<Item = Pick> + '_ {
    channel.airings.all().iter().enumerate()
        .map(move |(i, a)| Pick { channel: index, airing: i, begins_at: a.begins_at, ends_at: a.ends_at })
}

/// **The cards `filter` shows at `now`**, in channel order and then start order (the lineup is in channel order).
pub fn shown(channels: &[Channel], filter: Filter, now: i64, day_end: i64) -> Vec<Pick> {
    channels.iter().enumerate()
        .flat_map(|(i, c)| programmes(c, i))
        .filter(|p| filter.admits(stand(p.begins_at, p.ends_at, now), p.begins_at, day_end))
        .collect()
}

/// How many programmes are on air at `now`: the total [`count`] reads.
pub fn on_now(channels: &[Channel], now: i64) -> usize {
    channels.iter().enumerate().flat_map(|(i, c)| programmes(c, i))
        .filter(|p| matches!(stand(p.begins_at, p.ends_at, now), Stand::OnAir { .. })).count()
}

/// **The header's read-out**: `(shown, total)` while the filter narrows what is on now, else nothing.
pub fn count(filter: Filter, shown: usize, on_now: usize) -> Option<(usize, usize)> {
    filter.narrows().then_some((shown, on_now))
}

/// **The end of the local day** that holds `now`: the next local midnight, in Unix seconds. With no zone offset the day is read in
/// UTC (the instant alone says nothing of a local day).
pub fn day_end(now: i64, utc_offset_s: Option<i32>) -> i64 {
    let offset = i64::from(utc_offset_s.unwrap_or(0));
    (now + offset).div_euclid(DAY) * DAY + DAY - offset
}

/// **The first instant after `now` at which the shown set may change with no new data**: some programme's start, ten
/// minutes after it, fifteen minutes before its end, its end, or the end of the day. `i64::MAX` when none lies ahead.
pub fn next_change(channels: &[Channel], now: i64, day_end: i64) -> i64 {
    let minute = 60;
    let mut next = if day_end > now { day_end } else { i64::MAX };
    for p in channels.iter().enumerate().flat_map(|(i, c)| programmes(c, i)) {
        let mut edges = [p.begins_at, p.begins_at + (fmt::JUST_STARTED_MINUTES + 1) * minute, p.ends_at, p.ends_at - ENDING_SOON_MINUTES * minute];
        if p.ends_at == 0 { edges[2] = 0; edges[3] = 0; }
        for edge in edges {
            if edge > now { next = next.min(edge); }
        }
    }
    next
}

/// **Where a seat lands for `channel`** in the shown cards: its own card, the one airing now before a later one; when the filter
/// shows none of its programmes, the card at the place the channel would stand in channel order (the next channel's first, else the
/// last), so a seat whose programme ended or fell out of the filter lands beside where it was. `None` only when nothing is shown.
pub fn seat_for(picks: &[Pick], channel: usize, now: i64) -> Option<usize> {
    let own = || picks.iter().enumerate().filter(|(_, p)| p.channel == channel);
    own().find(|(_, p)| matches!(stand(p.begins_at, p.ends_at, now), Stand::OnAir { .. }))
        .or_else(|| own().next())
        .map(|(i, _)| i)
        .or_else(|| picks.iter().position(|p| p.channel > channel))
        .or_else(|| picks.len().checked_sub(1))
}

#[cfg(test)]
#[path = "browse_tests.rs"]
mod tests;
