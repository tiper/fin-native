//! The remote's keys: Ecore's key NAME to the application's [`Action`], and the press edges.
//!
//! A platform that is not SDL maps its native names to [`Action`] and nothing else (`docs/platform-ports.md`,
//! Target shape 2): it builds no raw pair, and the loop derives the pair its readers want (`raw_pair`). So
//! [`TABLE`] is one name to one `Option<Action>`, and a key with no action (the colour buttons, Info) is a
//! row with `None`: known, and dropped.
//!
//! **Emulator codes only; a retail remote is unmeasured.** The rows marked [`Basis::Measured`] are the
//! emulator's, observed with a key injector and read off the diagnostics panel (`docs/platform-ports.md`,
//! E7 and E11): Right (keycode 114), Left (113), Up (111), Down (116), Return (36) and the colour button
//! `XF86Color` (385). Every other row is [`Basis::Unverified`]: the name is what Tizen's TV key names are
//! documented as, or inferred from the Samsung remote's layout, and no run has pressed it. The first
//! retail key dump replaces this table, and a row that turns out wrong is a wrong row here and nowhere else.
//!
//! The table's only reader is [`Decoder::decode`]; the unit test below states each name's expected action
//! as a literal of its own, so a swapped pair (Play and Pause, Rewind and Left, the two pagers) fails it,
//! which a test that read the table back through itself could not.
use plxnative_modules::host::{Action, Edge};

/// How a row got into the table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Basis {
    /// Observed on the emulator (E7, E11).
    Measured,
    /// Documented or inferred; not pressed on any device.
    Unverified,
}

/// Ecore key name, the action it presses (`None`: a known key the application has no action for), and why
/// the row is believed. The measured rows are first, in the order they were measured.
pub const TABLE: &[(&str, Option<Action>, Basis)] = &[
    ("Right", Some(Action::Right), Basis::Measured),
    ("Left", Some(Action::Left), Basis::Measured),
    ("Up", Some(Action::Up), Basis::Measured),
    ("Down", Some(Action::Down), Basis::Measured),
    ("Return", Some(Action::Ok), Basis::Measured),
    ("XF86Color", None, Basis::Measured),
    // Back and leave.
    ("XF86Back", Some(Action::Back), Basis::Unverified),
    ("Escape", Some(Action::Back), Basis::Unverified),
    ("XF86Exit", Some(Action::Exit), Basis::Unverified),
    // Transport. Rewind is `Action::Rewind`, never plain `Left`: the player's scrub and Home's navigation
    // must stay two keys.
    ("XF86AudioPlay", Some(Action::Play), Basis::Unverified),
    ("XF86AudioPause", Some(Action::Pause), Basis::Unverified),
    ("XF86PlayBack", Some(Action::PlayPause), Basis::Unverified),
    ("XF86AudioStop", Some(Action::Stop), Basis::Unverified),
    ("XF86AudioRewind", Some(Action::Rewind), Basis::Unverified),
    ("XF86AudioNext", Some(Action::FastForward), Basis::Unverified),
    ("XF86AudioForward", Some(Action::FastForward), Basis::Unverified),
    // The channel rocker pages, as the LG one does.
    ("XF86ChannelUp", Some(Action::PageUp), Basis::Unverified),
    ("XF86ChannelDown", Some(Action::PageDown), Basis::Unverified),
    ("XF86RaiseChannel", Some(Action::PageUp), Basis::Unverified),
    ("XF86LowerChannel", Some(Action::PageDown), Basis::Unverified),
    // The colour buttons and Info have no action.
    ("XF86Red", None, Basis::Unverified),
    ("XF86Green", None, Basis::Unverified),
    ("XF86Yellow", None, Basis::Unverified),
    ("XF86Blue", None, Basis::Unverified),
    ("XF86Info", None, Basis::Unverified),
    // The number keys.
    ("0", Some(Action::Digit(0)), Basis::Unverified),
    ("1", Some(Action::Digit(1)), Basis::Unverified),
    ("2", Some(Action::Digit(2)), Basis::Unverified),
    ("3", Some(Action::Digit(3)), Basis::Unverified),
    ("4", Some(Action::Digit(4)), Basis::Unverified),
    ("5", Some(Action::Digit(5)), Basis::Unverified),
    ("6", Some(Action::Digit(6)), Basis::Unverified),
    ("7", Some(Action::Digit(7)), Basis::Unverified),
    ("8", Some(Action::Digit(8)), Basis::Unverified),
    ("9", Some(Action::Digit(9)), Basis::Unverified),
];

/// What `name` presses: `Some(Some(action))` for a bound key, `Some(None)` for a known key with no action,
/// `None` for a name the table does not list.
pub fn lookup(name: &str) -> Option<Option<Action>> {
    TABLE.iter().find(|(n, _, _)| *n == name).map(|(_, action, _)| *action)
}

/// How many keys can be held at once before the oldest is forgotten (a remote has one key down; a wedged
/// handler must not grow this without bound).
const HELD_LIMIT: usize = 16;

/// Turns Ecore's down and up events into the application's press edges. The first down of a key is
/// [`Edge::Down`], a down of a key that is still held is the auto-repeat ([`Edge::Repeat`]: Ecore has no
/// repeat flag, the key simply goes down again), and an up is [`Edge::Up`]. Host-thread state: the Ecore
/// handlers are its only callers.
#[derive(Default)]
pub struct Decoder {
    held: Vec<String>,
}

impl Decoder {
    pub const fn new() -> Self {
        Self { held: Vec::new() }
    }

    /// Every held key released at once (the window lost focus): the up of each key that has an action, in the
    /// order they went down. The held set is empty afterwards.
    pub fn release_all(&mut self) -> Vec<(Action, Edge)> {
        std::mem::take(&mut self.held)
            .iter()
            .filter_map(|name| lookup(name).flatten().map(|action| (action, Edge::Up)))
            .collect()
    }

    /// The press `name` makes, or `None` for a key with no action or a name the table does not list.
    pub fn decode(&mut self, name: &str, down: bool) -> Option<(Action, Edge)> {
        let edge = if down {
            if self.held.iter().any(|h| h == name) {
                Edge::Repeat
            } else {
                if self.held.len() >= HELD_LIMIT {
                    self.held.remove(0);
                }
                self.held.push(name.to_string());
                Edge::Down
            }
        } else {
            self.held.retain(|h| h != name);
            Edge::Up
        };
        lookup(name).flatten().map(|action| (action, edge))
    }
}

/// One Ecore key event (host thread): decode it, log it, and hand the press it makes to `push`.
///
/// Every edge but an auto-repeat is logged with the name and the keycode as Ecore gave them, whether or not
/// the table knows the key: the log is how a retail remote's real names and codes get collected, and the
/// table is replaced from it. The line never names the table's verdict as more than what it is.
pub fn feed(decoder: &mut Decoder, name: &str, code: u32, down: bool, push: impl FnOnce(Action, Edge)) {
    let press = decoder.decode(name, down);
    if !matches!(press, Some((_, Edge::Repeat))) {
        let verdict = match (press, lookup(name)) {
            (Some((action, edge)), _) => format!("{action:?} {edge:?}"),
            (None, Some(_)) => "known key, no action".to_string(),
            (None, None) => "unbound".to_string(),
        };
        plx_base::eventlog::log(&format!(
            "key: {} name={name:?} code={code} -> {verdict}",
            if down { "down" } else { "up" }
        ));
    }
    if let Some((action, edge)) = press {
        push(action, edge);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A key event is logged with the name and the code as Ecore gave them and pushed as the press it
    /// makes; an unbound key is logged (that is the key dump) and pushes nothing; a repeat pushes and does
    /// not log.
    #[test]
    fn feed_logs_the_dump_and_pushes_only_bound_presses() {
        plx_base::eventlog::with_private_log(|| {
            let mut d = Decoder::new();
            let mut pushed = Vec::new();
            feed(&mut d, "Right", 114, true, |a, e| pushed.push((a, e)));
            feed(&mut d, "Right", 114, true, |a, e| pushed.push((a, e)));
            feed(&mut d, "Right", 114, false, |a, e| pushed.push((a, e)));
            feed(&mut d, "XF86Color", 385, true, |a, e| pushed.push((a, e)));
            feed(&mut d, "XF86Mystery", 999, true, |a, e| pushed.push((a, e)));
            assert_eq!(
                pushed,
                [(Action::Right, Edge::Down), (Action::Right, Edge::Repeat), (Action::Right, Edge::Up)]
            );
            let log = std::fs::read_to_string(plx_base::eventlog::events_log()).unwrap_or_default();
            let lines: Vec<&str> = log.lines().collect();
            assert_eq!(
                lines,
                [
                    "key: down name=\"Right\" code=114 -> Right Down",
                    "key: up name=\"Right\" code=114 -> Right Up",
                    "key: down name=\"XF86Color\" code=385 -> known key, no action",
                    "key: down name=\"XF86Mystery\" code=999 -> unbound",
                ]
            );
        });
    }

    /// **Each name presses exactly the action the remote's key means**, stated here as literals and not read
    /// back from [`TABLE`]: a Play row that says Pause, a Rewind that says Left, a PageUp that says
    /// PageDown, or two digits swapped fails on its own line. (The application-side half of the guard,
    /// native name to effect, is the application's; a platform's table cannot be named from there, so this
    /// is the Tizen table's half, ending at what the loop is handed.)
    #[test]
    fn each_name_presses_its_own_action() {
        let expected: &[(&str, Option<Action>)] = &[
            ("Right", Some(Action::Right)),
            ("Left", Some(Action::Left)),
            ("Up", Some(Action::Up)),
            ("Down", Some(Action::Down)),
            ("Return", Some(Action::Ok)),
            ("XF86Color", None),
            ("XF86Back", Some(Action::Back)),
            ("Escape", Some(Action::Back)),
            ("XF86Exit", Some(Action::Exit)),
            ("XF86AudioPlay", Some(Action::Play)),
            ("XF86AudioPause", Some(Action::Pause)),
            ("XF86PlayBack", Some(Action::PlayPause)),
            ("XF86AudioStop", Some(Action::Stop)),
            ("XF86AudioRewind", Some(Action::Rewind)),
            ("XF86AudioNext", Some(Action::FastForward)),
            ("XF86AudioForward", Some(Action::FastForward)),
            ("XF86ChannelUp", Some(Action::PageUp)),
            ("XF86ChannelDown", Some(Action::PageDown)),
            ("XF86RaiseChannel", Some(Action::PageUp)),
            ("XF86LowerChannel", Some(Action::PageDown)),
            ("XF86Red", None),
            ("XF86Green", None),
            ("XF86Yellow", None),
            ("XF86Blue", None),
            ("XF86Info", None),
            ("0", Some(Action::Digit(0))),
            ("1", Some(Action::Digit(1))),
            ("2", Some(Action::Digit(2))),
            ("3", Some(Action::Digit(3))),
            ("4", Some(Action::Digit(4))),
            ("5", Some(Action::Digit(5))),
            ("6", Some(Action::Digit(6))),
            ("7", Some(Action::Digit(7))),
            ("8", Some(Action::Digit(8))),
            ("9", Some(Action::Digit(9))),
        ];
        for (name, action) in expected {
            assert_eq!(lookup(name), Some(*action), "{name}");
        }
        assert_eq!(TABLE.len(), expected.len(), "a row is added here and in the table together");
        let mut names: Vec<_> = TABLE.iter().map(|(n, _, _)| *n).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), TABLE.len(), "no name is listed twice");
        assert_ne!(lookup("XF86AudioRewind"), lookup("Left"), "Rewind is not plain Left");
        assert_ne!(lookup("XF86AudioPlay"), lookup("XF86AudioPause"));
    }

    /// The emulator's measured rows come first and are marked so, and every other row says it is not.
    #[test]
    fn the_measured_rows_are_the_emulators_six_and_the_rest_say_unverified() {
        let measured: Vec<_> = TABLE.iter().filter(|r| r.2 == Basis::Measured).map(|r| r.0).collect();
        assert_eq!(measured, ["Right", "Left", "Up", "Down", "Return", "XF86Color"]);
        assert!(TABLE[..6].iter().all(|r| r.2 == Basis::Measured), "the measured rows are the first six");
        assert!(TABLE[6..].iter().all(|r| r.2 == Basis::Unverified));
    }

    #[test]
    fn an_unlisted_name_is_unknown_and_a_listed_unbound_one_is_known() {
        assert_eq!(lookup("XF86Mystery"), None);
        assert_eq!(lookup("right"), None, "names are case-sensitive, as Ecore's are");
        assert_eq!(lookup("XF86Red"), Some(None));
    }

    #[test]
    fn the_decoder_sends_down_then_repeat_then_up() {
        let mut d = Decoder::new();
        assert_eq!(d.decode("Right", true), Some((Action::Right, Edge::Down)));
        assert_eq!(d.decode("Right", true), Some((Action::Right, Edge::Repeat)));
        assert_eq!(d.decode("Right", true), Some((Action::Right, Edge::Repeat)));
        assert_eq!(d.decode("Right", false), Some((Action::Right, Edge::Up)));
        assert_eq!(d.decode("Right", true), Some((Action::Right, Edge::Down)), "after the up it is a fresh press");
        assert_eq!(d.decode("Up", true), Some((Action::Up, Edge::Down)), "another key is its own press");
        assert_eq!(d.decode("Up", false), Some((Action::Up, Edge::Up)));
        assert_eq!(d.decode("Down", false), Some((Action::Down, Edge::Up)), "an up with no down is still an up");
    }

    #[test]
    fn a_key_with_no_action_and_an_unknown_one_press_nothing() {
        let mut d = Decoder::new();
        assert_eq!(d.decode("XF86Color", true), None);
        assert_eq!(d.decode("XF86Color", false), None);
        assert_eq!(d.decode("XF86Mystery", true), None);
        assert_eq!(d.decode("XF86Mystery", false), None);
    }

    /// **Losing focus releases every held key**: the key-up goes to whatever took the focus, so without this the
    /// first press after the focus returns would be a repeat with no down (SDL, the shared path, sends a release
    /// for every held key when its window loses focus). Keys with no action have nothing to release.
    #[test]
    fn losing_focus_releases_every_held_key_once() {
        let mut d = Decoder::new();
        d.decode("Right", true);
        d.decode("XF86Color", true);
        d.decode("Return", true);
        assert_eq!(d.release_all(), [(Action::Right, Edge::Up), (Action::Ok, Edge::Up)]);
        assert_eq!(d.release_all(), [], "nothing is held any more");
        assert_eq!(d.decode("Right", true), Some((Action::Right, Edge::Down)), "the next press is a fresh one");
    }

    #[test]
    fn a_wedged_handler_cannot_grow_the_held_set_without_bound() {
        let mut d = Decoder::new();
        for i in 0..(HELD_LIMIT * 4) {
            let _ = d.decode(&format!("k{i}"), true);
        }
        assert_eq!(d.held.len(), HELD_LIMIT);
    }
}
