//! **Number entry, as one small machine** (Live TV PR5; handoff SPEC-5). Pure: no clock read, no focus read, no
//! effect performed; the panel (`channel_entry_panel`) feeds events and acts on the one effect that comes back.
//!
//! The viewer types a channel number anywhere (on the What's on page, or over the live player). The digits narrow
//! the lineup to the channels whose number STARTS with them, and the panel lists those matches under the typed
//! number. Nothing tunes while the digits still fit more than one channel: the picture keeps playing.
//!
//! ```text
//!   Fresh{digits} --any event at `now`--> Typing{digits, ..}                    (the opening digits ride on the open)
//!   Typing --Digit--> Typing{digits + d}      the highlight restarts on the match the digits name, the timer restarts
//!   Typing --Digit, the digits now fit ONE channel--> Done + Tune(it)           (Leave when that channel is the one on)
//!   Typing --Moved{to}--> Typing{highlight: to}    (the focus engine moved the cursor: up / down)
//!   Typing --Ok--> Done + Tune(highlight)          (Leave when that channel is the one on; nothing when no channel matches)
//!   Typing --Tick, IDLE_MS after the last key--> Done + Tune(highlight)   (Leave when nothing matches, or it is the one on)
//!   Fresh | Typing --Back--> Done + Leave          (no tune at any point)
//!   Done carries the digits it finished with, so what the panel shows while it fades is the machine's own.
//! ```
//!
//! A channel is matched on its number's DIGITS ([`Cand::key`]: `6.1` is `61`), because the viewer has digit keys and no
//! dot; the row shows the real number ([`Cand::number`]).
//!
//! The highlight is a channel of the lineup, held by its index, never a row of the shown list: a digit that
//! narrows the list moves no highlight by position, and it is `None` exactly when nothing matches. It starts on the exact match when there is one, else on the
//! first match that is not the channel already playing, so OK on the opening highlight is never a silent no-op;
//! the playing channel carries a tag for the same reason.
//!
//! An exact match that is also the prefix of a longer number (`201` with `2010` in the lineup) only seats the
//! highlight and waits for [`IDLE_MS`]: tuning it at once would make `2010` unreachable.

/// How long after the last key the highlighted match tunes (SPEC-5).
pub const IDLE_MS: u32 = 2_500;
/// How many matches the panel's list shows at once; the rest scroll under it (SPEC-5: "a window of five").
pub const LIST_ROWS: usize = 5;

/// One channel of the lineup, as the machine reads it: its number, the digits of that number (what is matched) and
/// whether it is the one already on screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cand {
    /// The number as the row shows it (`6.1`).
    pub number: String,
    /// The number's digits only (`61`): what typed digits are matched against.
    pub key: String,
    pub playing: bool,
}

impl Cand {
    pub fn new(number: &str, playing: bool) -> Self {
        Self { number: number.to_string(), key: number.chars().filter(char::is_ascii_digit).collect(), playing }
    }
}

/// The number entry's phase.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NumberEntry {
    /// The panel has just opened with the digits that opened it (usually one; more when several keys arrived before
    /// it mounted); no event has carried a time yet.
    Fresh { digits: String },
    /// Digits are being collected. `highlight` is the lineup index of the highlighted match (`None` when nothing
    /// matches) and `last` the time of the latest key.
    Typing { digits: String, highlight: Option<usize>, last: u32 },
    /// The panel has asked to leave (a tune went out, or a dismissal): nothing more is asked. It keeps the digits it
    /// finished with.
    Done { digits: String },
}

/// What happened to the machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ev {
    Digit { d: u8, now: u32 },
    /// The focus engine moved the cursor to lineup index `to` (▲ ▼ on the list).
    Moved { to: usize, now: u32 },
    /// Any other key: the idle timer restarts.
    Touch { now: u32 },
    Ok { now: u32 },
    Back,
    Tick { now: u32 },
}

impl Ev {
    /// The time the event carries (none for BACK).
    pub fn now(&self) -> Option<u32> {
        match *self {
            Ev::Digit { now, .. } | Ev::Moved { now, .. } | Ev::Touch { now } | Ev::Ok { now } | Ev::Tick { now } => Some(now),
            Ev::Back => None,
        }
    }
}

/// What the panel must do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fx {
    None,
    /// put the cursor on this lineup index (the digits changed the list)
    Seat(usize),
    /// tune this lineup index, then leave
    Tune(usize),
    /// leave with no tune
    Leave,
}

/// The lineup indices whose number starts with `digits`, in lineup order.
pub fn matches<'a>(lineup: &'a [Cand], digits: &'a str) -> impl Iterator<Item = usize> + 'a {
    lineup.iter().enumerate().filter(move |(_, c)| c.key.starts_with(digits)).map(|(i, _)| i)
}

/// Where the highlight starts for `digits`: the exact match, else the first match that is not playing, else the first.
pub fn start_highlight(lineup: &[Cand], digits: &str) -> Option<usize> {
    let all: Vec<usize> = matches(lineup, digits).collect();
    all.iter().copied().find(|&i| lineup[i].key == digits)
        .or_else(|| all.iter().copied().find(|&i| !lineup[i].playing))
        .or_else(|| all.first().copied())
}

/// The longest number of the lineup, in digits: the most digits worth typing.
pub fn longest(lineup: &[Cand]) -> usize {
    lineup.iter().map(|c| c.key.len()).max().unwrap_or(0)
}

impl NumberEntry {
    /// Open with `digits` already typed (keys that arrived before the panel mounted).
    pub fn open_digits(digits: &str) -> Self {
        NumberEntry::Fresh { digits: digits.chars().filter(char::is_ascii_digit).collect() }
    }

    /// What the viewer has typed, for the panel to show. Before the first event the opening digits are already on
    /// screen; after the machine is done they are the digits it finished with.
    pub fn typed(&self) -> &str {
        match self {
            NumberEntry::Fresh { digits } | NumberEntry::Typing { digits, .. } | NumberEntry::Done { digits } => digits,
        }
    }

    pub fn is_done(&self) -> bool {
        matches!(self, NumberEntry::Done { .. })
    }

    /// The highlighted lineup index: while typing, and only when something matches.
    pub fn highlight(&self) -> Option<usize> {
        match self {
            NumberEntry::Typing { highlight, .. } => *highlight,
            NumberEntry::Fresh { .. } | NumberEntry::Done { .. } => None,
        }
    }

    /// Stop with no tune, keeping the digits.
    fn leave(&mut self) -> Fx {
        let digits = self.typed().to_string();
        *self = NumberEntry::Done { digits };
        Fx::Leave
    }

    /// Step the machine on `ev` over the lineup the panel captured when it opened.
    pub fn step(&mut self, ev: Ev, lineup: &[Cand]) -> Fx {
        // The opening digits are read at the time of the first event that carries one, so the machine never
        // reads a clock. BACK before any event is just a cancel.
        if let (NumberEntry::Fresh { digits }, Some(now)) = (&*self, ev.now()) {
            let opening = digits.clone();
            *self = NumberEntry::Typing { digits: String::new(), highlight: None, last: now };
            let mut opened = Fx::None;
            for d in opening.bytes() {
                opened = self.push(d - b'0', now, lineup);
                if self.is_done() {
                    return opened;
                }
            }
            // the opening digits answered; the event that woke them is handled as it would have been after them
            let then = self.step_typing(ev, lineup);
            return if then == Fx::None { opened } else { then };
        }
        self.step_typing(ev, lineup)
    }

    fn step_typing(&mut self, ev: Ev, lineup: &[Cand]) -> Fx {
        let NumberEntry::Typing { digits, highlight, last } = self else {
            return match (&*self, ev) {
                (NumberEntry::Fresh { .. }, Ev::Back) => self.leave(),
                _ => Fx::None,
            };
        };
        match ev {
            Ev::Back => self.leave(),
            Ev::Digit { d, now } => self.push(d, now, lineup),
            Ev::Moved { to, now } => {
                *last = now;
                if lineup.get(to).is_some_and(|c| c.key.starts_with(digits.as_str())) {
                    *highlight = Some(to);
                }
                Fx::None
            }
            Ev::Touch { now } => {
                *last = now;
                Fx::None
            }
            Ev::Ok { now } => {
                *last = now;
                // nothing matches: nothing to choose
                match *highlight {
                    Some(h) => self.finish(h, lineup),
                    None => Fx::None,
                }
            }
            Ev::Tick { now } => {
                if now.wrapping_sub(*last) < IDLE_MS {
                    return Fx::None;
                }
                match *highlight {
                    Some(h) => self.finish(h, lineup),
                    None => self.leave(),
                }
            }
        }
    }

    /// Append digit `d` at `now`, unless the digits already are as long as any number: that key is ignored, but still a key.
    fn push(&mut self, d: u8, now: u32, lineup: &[Cand]) -> Fx {
        let NumberEntry::Typing { digits, last, .. } = self else { return Fx::None };
        if digits.len() >= longest(lineup) {
            *last = now;
            return Fx::None;
        }
        self.digit(d, now, lineup)
    }

    /// Append digit `d` at `now`: one match tunes at once, several seat the highlight, none clears it.
    fn digit(&mut self, d: u8, now: u32, lineup: &[Cand]) -> Fx {
        let NumberEntry::Typing { digits, highlight, last } = self else { return Fx::None };
        digits.push(char::from(b'0' + d.min(9)));
        *last = now;
        let (a, b) = {
            let mut it = matches(lineup, digits);
            (it.next(), it.next())
        };
        match (a, b) {
            (None, _) => {
                *highlight = None;
                Fx::None
            }
            (Some(only), None) => self.finish(only, lineup),
            (Some(_), Some(_)) => {
                let h = start_highlight(lineup, digits);
                *highlight = h;
                h.map_or(Fx::None, Fx::Seat)
            }
        }
    }

    /// Leave having chosen lineup index `i`: a tune, unless that channel is the one already on screen.
    fn finish(&mut self, i: usize, lineup: &[Cand]) -> Fx {
        let digits = self.typed().to_string();
        *self = NumberEntry::Done { digits };
        if lineup.get(i).is_some_and(|c| c.playing) { Fx::Leave } else { Fx::Tune(i) }
    }
}

#[cfg(test)]
#[path = "channel_entry_tests.rs"]
mod tests;
