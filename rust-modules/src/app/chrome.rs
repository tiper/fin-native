//! Application-owned data for the shared bar. Capture before stepping a frame; paint consumes
//! borrowed strings and never opens the session file or polls the Browse vocabulary.

use plx_ui::containers::tabs::StripMember;
use plx_machine::machine::{FocusKey, Measure};
use plx_ui::icons::Icon;
use plx_ui::fmt::Wall;
use plx_ui::widgets::{self, ChromeRead, ClockRead, ProfileChipRead, TabFace, TabLabels, TopFocus};
use plx_data::stores::browse::{DirectoryView, SecKind};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Pill {
    Home,
    Section(SecKind),
    LiveTv,
    Watchlist,
    Search,
}

/// The strip's destination keys in strip order: Home, one key per type pill, Live TV when tunable
/// channels are available, the Watchlist bookmark, Search. The one place that order is written;
/// `refresh` publishes it and the pill maps below read it, so no map assumes Search's index.
fn strip_keys(directory: DirectoryView<'_>, live_tv: bool) -> Vec<u32> {
    let mut keys = vec![widgets::TAB_KEY_HOME];
    keys.extend((0..directory.tab_count()).filter_map(|i| directory.tab_kind(i)).map(|kind| match kind {
        SecKind::Movie => widgets::TAB_KEY_MOVIES,
        SecKind::Show => widgets::TAB_KEY_SHOWS,
    }));
    if live_tv { keys.push(widgets::TAB_KEY_LIVETV); }
    keys.push(widgets::TAB_KEY_WATCHLIST);
    keys.push(widgets::TAB_KEY_SEARCH);
    keys
}

pub(crate) fn pill_at(directory: DirectoryView<'_>, live_tv: bool, i: usize) -> Pill {
    pill_in(i, &strip_keys(directory, live_tv))
}

/// The destination a strip index names, by the KEY published at that index.
fn pill_in(i: usize, keys: &[u32]) -> Pill {
    match keys.get(i).copied() {
        Some(widgets::TAB_KEY_MOVIES) => Pill::Section(SecKind::Movie),
        Some(widgets::TAB_KEY_SHOWS) => Pill::Section(SecKind::Show),
        Some(widgets::TAB_KEY_LIVETV) => Pill::LiveTv,
        Some(widgets::TAB_KEY_WATCHLIST) => Pill::Watchlist,
        Some(widgets::TAB_KEY_SEARCH) => Pill::Search,
        _ => Pill::Home,
    }
}

pub(crate) fn pill_of(directory: DirectoryView<'_>, live_tv: bool, pill: Pill) -> Option<usize> {
    pill_index(pill, &strip_keys(directory, live_tv))
}

fn pill_index(pill: Pill, keys: &[u32]) -> Option<usize> {
    let key = match pill {
        Pill::Home => widgets::TAB_KEY_HOME,
        Pill::Section(SecKind::Movie) => widgets::TAB_KEY_MOVIES,
        Pill::Section(SecKind::Show) => widgets::TAB_KEY_SHOWS,
        Pill::LiveTv => widgets::TAB_KEY_LIVETV,
        Pill::Watchlist => widgets::TAB_KEY_WATCHLIST,
        Pill::Search => widgets::TAB_KEY_SEARCH,
    };
    keys.iter().position(|&k| k == key)
}

#[derive(Default)]
pub(crate) struct ChromeSnapshot {
    tabs_generation: Option<u32>,
    live_tv: bool,
    labels_generation: u32,
    profile_generation: Option<u32>,
    session_watch: plx_plex::plex::session::VisibleSessionWatch,
    faces: Vec<TabFace>,
    pub(crate) keys: Vec<u32>,
    widths: Vec<f32>,
    thumb: String,
    /// The profile's full chip label and initial, as captured: the chip's name is ELIDED from
    /// these to the room the strip leaves ([`widgets::chip_name_budget`]), so a change of the
    /// strip's widths re-elides without re-reading the session.
    label: String,
    initial_text: String,
    initial: std::ffi::CString,
    name: std::ffi::CString,
    name_w: f32,
    /// A pulse asked for and not yet handed to the strip's renderer: the destination key.
    pulse: Option<u32>,
    /// What the clock's face was last worded FROM: the Unix minute and the zone's offset, the only
    /// two things a face is a function of. Kept apart from `clock` because a wall time whose face
    /// cannot be worded (the sum overflows) must not read as "changed" on every frame.
    clock_key: Option<(i64, i32)>,
    /// The bar's clock, worded and measured for `clock_key`; `None` without a wall time.
    clock: Option<ClockFace>,
}

/// The instant `stillclock` holds the bar's clock at: 2025-01-01 10:10:00 UTC, in UTC (the watch-shop
/// hands), so a held capture's face is the same on every machine and in every zone.
const HELD_WALL: Wall = Wall { unix_ms: 1_735_726_200_000, utc_offset_s: 0 };

/// The clock's face as captured: the television's 12/24-hour wording of the minute, and its width.
struct ClockFace {
    text: std::ffi::CString,
    w: f32,
}

impl ChromeSnapshot {
    /// **Pulse the strip destination `key`** (~1.35x scale and a glow, decorative): the Watchlist
    /// mark after a title was put on the list. The APPLICATION asks, never a screen; the bridge
    /// hands it to the strip's renderer on its next chrome update. A frame is requested so an
    /// idle app wakes for it.
    pub(crate) fn pulse(&mut self, key: u32) {
        self.pulse = Some(key);
    }

    /// The pending pulse as the strip index of its destination; `None` when nothing is pending or
    /// the destination is not on the strip (a Home with no Watchlist mark). Consumes it.
    pub(crate) fn take_pulse(&mut self) -> Option<usize> {
        let key = self.pulse.take()?;
        self.keys.iter().position(|k| *k == key)
    }

    /// Forget a pending pulse: the bar is not on screen, so it would play late on the next page.
    pub(crate) fn drop_pulse(&mut self) {
        self.pulse = None;
    }

    pub(crate) fn refresh(&mut self, measure: &dyn Measure, directory: DirectoryView<'_>, live_tv: bool) {
        self.refresh_with_profile(measure, directory, live_tv, None);
    }

    pub(crate) fn refresh_with_profile(&mut self, measure: &dyn Measure, directory: DirectoryView<'_>, live_tv: bool,
        captured: Option<(&plx_plex::plex::session::CurrentProfile, &plx_plex::plex::session::Session)>) {
        let generation = directory.tabs_gen();
        let strip_changed = self.tabs_generation != Some(generation) || self.live_tv != live_tv;
        if strip_changed {
            self.keys = strip_keys(directory, live_tv);
            self.faces = self.keys.iter().map(|&key| match key {
                widgets::TAB_KEY_HOME => TabFace::Label(plx_os::i18n::msg::browse_chrome_home().into()),
                widgets::TAB_KEY_MOVIES => TabFace::Label(plx_os::i18n::msg::browse_kind_movies().into()),
                widgets::TAB_KEY_SHOWS => TabFace::Label(plx_os::i18n::msg::browse_kind_tv_shows().into()),
                widgets::TAB_KEY_LIVETV => TabFace::Label(plx_os::i18n::msg::browse_chrome_live_tv().into()),
                widgets::TAB_KEY_WATCHLIST => TabFace::Icon(Icon::Bookmark),
                _ => TabFace::Icon(Icon::Search),
            }).collect();
            self.widths = widgets::tab_widths(&self.faces, measure);
            self.tabs_generation = Some(generation);
            self.live_tv = live_tv;
            self.labels_generation = self.labels_generation.wrapping_add(1);
        }
        let generation = captured.map_or_else(plx_plex::plex::session::current_gen, |(profile, _)| profile.generation);
        let session_changed = captured.is_none() && self.session_watch.changed();
        let mut profile_changed = false;
        if self.profile_generation != Some(generation) || session_changed {
            let current = captured.map_or_else(plx_plex::plex::session::current, |(profile, _)| profile.user.clone());
            let account = if let Some((_, saved)) = captured { saved.account(current.as_ref()) }
                else { plx_plex::plex::session::peek().account(current.as_ref()) };
            self.thumb = current.map(|user| user.thumb).unwrap_or_default();
            self.label = plx_screens::account_menu::chip_label(&account);
            self.initial_text = account.name.as_deref().and_then(|name| name.chars().next())
                .map(|c| c.to_uppercase().to_string()).unwrap_or_default();
            self.profile_generation = Some(generation);
            profile_changed = true;
        }
        if profile_changed || strip_changed {
            self.elide_chip_name(measure);
        }
    }

    /// Elide the chip's name to the room the strip's CURRENT widths leave it. Called when the
    /// profile changes and when the strip does (a library kind appearing or leaving changes the
    /// track), because the budget is a function of both and drawing measures nothing.
    fn elide_chip_name(&mut self, measure: &dyn Measure) {
        let budget = widgets::chip_name_budget(widgets::tab_track_w(&self.widths));
        (self.initial, self.name, self.name_w) =
            widgets::profile_chip_text(&self.label, &self.initial_text, budget, measure);
    }

    pub(crate) fn labels(&self) -> TabLabels<'_> {
        TabLabels { generation: self.labels_generation, faces: &self.faces }
    }

    /// The strip key published at `selected` (an index into [`Self::keys`], as `members` takes it):
    /// the page's own pill, for the landing rule `TabContainer::strip_selected` states.
    pub(crate) fn key_at(&self, selected: i32) -> Option<u32> {
        self.keys.get(selected.max(0) as usize).copied()
    }

    pub(crate) fn profile(&self) -> ProfileChipRead<'_> {
        // The avatar is drawn against the BROWSED server, which is read where the chrome is
        // published: the library does not ask which server is current.
        ProfileChipRead { src: plx_plex::plex::current_server().raw(), thumb: &self.thumb,
            initial: &self.initial, name: &self.name, name_w: self.name_w }
    }

    pub(crate) fn read(&self, chip_expand: f32) -> ChromeRead<'_> {
        ChromeRead { profile: self.profile(), labels: self.labels(), chip_expand, clock: self.clock_read() }
    }

    /// **Capture the bar's clock for this frame's wall time** (the loop's one clock publication,
    /// `Bridge::publish_clock`); returns whether the face CHANGED, so the loop asks for the one
    /// frame a new minute owes ([`Self::set_clock`]'s caller notes the damage). The face is a
    /// function of the Unix minute and the zone's offset, so it is worded and measured once a
    /// minute, never per frame; the same wall time again is `false` and costs two integer compares.
    ///
    /// No wall time (a replay, a zone the platform cannot read) is no clock. The screenshot pipeline's
    /// `stillclock` HOLDS the wall at [`HELD_WALL`], the idiom's meaning for every free-running clock,
    /// so a documentation shot carries the face and no two shots differ by the minute they were taken in.
    pub(crate) fn set_clock(&mut self, wall: Option<Wall>, measure: &dyn Measure) -> bool {
        let wall = wall.map(|wall| if plx_machine::motion::phase_clocks_held() { HELD_WALL } else { wall });
        let key = wall.map(|wall| (wall.unix_s().div_euclid(60), wall.utc_offset_s));
        if key == self.clock_key { return false; }
        self.clock_key = key;
        self.clock = wall.and_then(|wall| plx_ui::fmt::clock_face(wall.unix_s(), i64::from(wall.utc_offset_s)))
            .and_then(|face| std::ffi::CString::new(face).ok())
            .map(|text| {
                let w = measure.width(&text, widgets::TOP_BAR_CLOCK_SIZE, false);
                ClockFace { text, w }
            });
        true
    }

    /// The clock as the bar draws it: the captured face, if any. The strip yields to the widest face
    /// the clock can wear (`widgets::TAB_CLOCK_CLEAR`), so there is no room rule to apply here.
    pub(crate) fn clock_read(&self) -> Option<ClockRead<'_>> {
        self.clock.as_ref().map(|face| ClockRead { text: &face.text, w: face.w })
    }

    pub(crate) fn focus(&self, focus: Option<FocusKey<u32>>) -> TopFocus {
        match focus.map(|focus| focus.elem) {
            Some(key) if key == widgets::TAB_KEY_CHIP => TopFocus::Chip,
            Some(key) => self.keys.iter().position(|&id| id == key).map(TopFocus::Pill).unwrap_or(TopFocus::Away),
            None => TopFocus::Away,
        }
    }

    /// `scroll` is the shared strip's current offset (`StripRender::scroll_pos`, owned by
    /// `app::bridge::Bridge`) — a parameter because this snapshot holds no `StripRender` of its
    /// own; the `Bridge` methods that call this are the one place both live.
    pub(crate) fn members(&self, selected: i32, focus: Option<FocusKey<u32>>, scroll: f32, out: &mut Vec<StripMember<u32>>) {
        out.clear();
        out.push(StripMember::new(widgets::TAB_KEY_CHIP, widgets::CHIP_FRAME));
        widgets::tab_members(&self.widths, &self.keys, selected, self.focus(focus), scroll, out);
    }

    #[cfg(test)]
    pub(crate) fn seed_for_test(&mut self, name: &str, initial: &str, labels: &[&str], measure: &dyn Measure) {
        self.tabs_generation = Some(1);
        self.labels_generation = 1;
        // An empty label is the Search slot: the production strip ends in the Search icon face.
        self.faces = labels.iter()
            .map(|s| if s.is_empty() { TabFace::Icon(Icon::Search) } else { TabFace::Label((*s).to_owned()) })
            .collect();
        self.keys = (0..self.faces.len()).map(|i| plx_ui::dispatch::STRIP_BASE + i as u32).collect();
        self.widths = widgets::tab_widths(&self.faces, measure);
        self.profile_generation = Some(1);
        self.thumb.clear();
        self.label = name.to_owned();
        self.initial_text = initial.to_owned();
        self.elide_chip_name(measure);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use plx_data::stores::browse::{DirectorySnapshot, SectionView, SrcRow};
    use plx_machine::machine::EntryId;

    fn directory(kinds: &[SecKind]) -> DirectorySnapshot {
        DirectorySnapshot::fixture(7, 0, kinds.iter().enumerate().map(|(i, &kind)| SectionView {
            sid: Some(plx_plex::plex::ServerId::from_raw((i / 2) as u16)),
            key: i as i64 + 1,
            kind,
            row: SrcRow { section: i, title: format!("Library {i}"), pinned: true,
                ..Default::default() },
        }).collect())
    }

    #[test]
    fn session_refresh_rebuilds_the_profile_chip_without_a_profile_switch() {
        let _serial = plx_base::testlock::serial();
        let _session = plx_plex::plex::session::TempSession::new("chrome-session-refresh");
        let mut snapshot = ChromeSnapshot::default();
        let directory = directory(&[]);
        plx_plex::plex::session::install_transient_for_test(true);
        snapshot.refresh(&plx_ui::fixture::FixtureMeasure, directory.view(), true);
        assert_eq!(snapshot.name.to_str().unwrap(), "Sign in");
        plx_plex::plex::session::save(&plx_plex::plex::session::Session {
            client_id: "synthetic-client".into(), account_token: "synthetic-token".into(),
            home_users: vec![plx_plex::plex::session::HomeUserRef {
                title: "Synthetic owner".into(), admin: true, ..Default::default()
            }], ..Default::default()
        });
        snapshot.refresh(&plx_ui::fixture::FixtureMeasure, directory.view(), true);
        assert_eq!(snapshot.name.to_str().unwrap(), "Synthetic owner");
    }

    /// A font (or a language) that lengthens the pills, standing in for one that fills the bar.
    struct Wide;
    impl Measure for Wide {
        fn width(&self, s: &std::ffi::CStr, sz: i32, _bold: bool) -> f32 {
            s.to_bytes().len() as f32 * sz as f32 * 3.0
        }
        fn cap_h(&self, sz: i32) -> f32 { sz as f32 * 0.7 }
        fn line_h(&self, sz: i32) -> f32 { sz as f32 * 1.2 }
    }

    /// The chip's name is elided to the room the strip leaves it, so a change of the STRIP (a
    /// library kind appearing or leaving) re-elides it with the profile untouched: the owner
    /// holds the full label and re-asks `chip_name_budget` whenever `tabs_generation` moves, and
    /// drawing measures nothing. A bar too wide to leave the name any room shows none.
    #[test]
    fn the_chip_name_is_re_elided_when_only_the_strip_changes() {
        let _serial = plx_base::testlock::serial();
        let directory = directory(&[SecKind::Movie, SecKind::Show]);
        let mut snapshot = ChromeSnapshot::default();
        // the first refresh captures whatever profile is seated; the label is then a long one, as
        // a profile with a long name would have captured, and only the STRIP moves from here on
        snapshot.refresh(&plx_ui::fixture::FixtureMeasure, directory.view(), true);
        snapshot.label = "A profile name long enough to overflow any budget the bar could leave it".into();
        snapshot.tabs_generation = None;
        snapshot.refresh(&plx_ui::fixture::FixtureMeasure, directory.view(), true);
        let roomy = snapshot.name_w;
        let budget = widgets::chip_name_budget(widgets::tab_track_w(&snapshot.widths));
        assert!(budget > 0.0, "the ordinary strip leaves the chip a name");
        assert!(roomy > 0.0 && roomy <= budget, "name {roomy} against a budget of {budget}");
        assert!(snapshot.name.to_str().unwrap().ends_with('\u{2026}'), "a long name is elided");

        // the strip is rebuilt with wider pills; the profile generation does not move
        snapshot.tabs_generation = None;
        snapshot.refresh(&Wide, directory.view(), true);
        assert!(widgets::tab_track_w(&snapshot.widths) > 1300.0, "the fixture strip must fill the bar (the viewport yields to the clock, 1404 at most)");
        assert_eq!(widgets::chip_name_budget(widgets::tab_track_w(&snapshot.widths)), 0.0);
        assert!(snapshot.name.as_bytes().is_empty() && snapshot.name_w == 0.0,
            "no room, no name: {:?} ({})", snapshot.name, snapshot.name_w);
        assert_eq!(widgets::chip_cap(1.0, snapshot.name_w), widgets::chip_cap(0.0, 0.0),
            "and the capsule stays the closed circle");

        // …and back: the name returns to the full budget of the ordinary strip
        snapshot.tabs_generation = None;
        snapshot.refresh(&plx_ui::fixture::FixtureMeasure, directory.view(), true);
        assert_eq!(snapshot.name_w, roomy);
    }

    /// 2026-11-26 19:50:00 UTC: a whole minute.
    const AT: i64 = 1_795_636_200;

    fn face_of(snapshot: &ChromeSnapshot) -> String {
        snapshot.clock.as_ref().expect("a face").text.to_str().unwrap().to_owned()
    }

    /// **The face is a function of the Unix minute and the zone's offset, worded once.** The same wall
    /// time again, or another second of the minute, is no change (so the loop asks for no frame); the
    /// next minute is one change, and so is a zone moving under the same instant. The words are
    /// `fmt::clock_face`'s, the television's 12/24-hour style the airing captions already use.
    #[test]
    fn the_clock_face_is_captured_once_a_minute_and_a_new_minute_reports_one_change() {
        let mut snapshot = ChromeSnapshot::default();
        let measure = &plx_ui::fixture::FixtureMeasure;
        assert!(snapshot.clock_read().is_none(), "no wall time yet: no clock");
        assert!(!snapshot.set_clock(None, measure), "nothing in, nothing out: no change");

        assert!(snapshot.set_clock(Some(Wall::from_secs(AT, 3_600)), measure), "the first face");
        assert_eq!(face_of(&snapshot), plx_ui::fmt::clock_face(AT, 3_600).unwrap());
        let first = face_of(&snapshot);
        assert_eq!(snapshot.clock.as_ref().unwrap().w, 28.0 * 0.5 * first.len() as f32, "measured at BODY");
        for second in [0, 1, 30, 59] {
            let wall = Wall { unix_ms: (AT + second) * 1_000 + 999, utc_offset_s: 3_600 };
            assert!(!snapshot.set_clock(Some(wall), measure), "second {second} of the same minute is no change");
        }
        assert!(snapshot.set_clock(Some(Wall::from_secs(AT + 60, 3_600)), measure), "the next minute");
        assert_eq!(face_of(&snapshot), plx_ui::fmt::clock_face(AT + 60, 3_600).unwrap());
        assert_ne!(face_of(&snapshot), first, "and it reads a minute later");
        assert!(!snapshot.set_clock(Some(Wall::from_secs(AT + 61, 3_600)), measure));

        assert!(snapshot.set_clock(Some(Wall::from_secs(AT + 61, 7_200)), measure), "the same instant in another zone");
        assert_eq!(face_of(&snapshot), plx_ui::fmt::clock_face(AT + 61, 7_200).unwrap());
        assert!(snapshot.set_clock(None, measure), "a wall time lost is a change");
        assert!(snapshot.clock.is_none() && snapshot.clock_read().is_none());
        assert!(!snapshot.set_clock(None, measure), "and staying lost is none");
    }

    /// The bar's focus stops are the chip and the strip's pills, whatever the clock holds: it adds no member.
    #[test]
    fn the_clock_adds_no_focus_stop_to_the_bar() {
        let directory = directory(&[SecKind::Movie, SecKind::Show]);
        let mut snapshot = ChromeSnapshot::default();
        snapshot.refresh(&plx_ui::fixture::FixtureMeasure, directory.view(), true);
        let mut without = Vec::new();
        snapshot.members(0, None, 0.0, &mut without);
        snapshot.set_clock(Some(Wall::from_secs(AT, 3_600)), &plx_ui::fixture::FixtureMeasure);
        let mut with = Vec::new();
        snapshot.members(0, None, 0.0, &mut with);
        assert_eq!(with.iter().map(|m| m.elem).collect::<Vec<_>>(), without.iter().map(|m| m.elem).collect::<Vec<_>>());
        assert!(snapshot.clock_read().is_some(), "premise: the clock is there");
    }

    /// The screenshot pipeline holds every free-running clock (`stillclock`), and the bar's clock is
    /// one: it HOLDS the wall at a fixed instant (the face a capture carries), whatever the live time
    /// and zone are, so each documentation shot is the same picture. No wall time (a replay) stays none.
    #[cfg(feature = "devtriggers")]
    #[test]
    fn the_screenshot_pipelines_pinned_clocks_hold_the_clock_at_a_fixed_instant() {
        let mut snapshot = ChromeSnapshot::default();
        let measure = &plx_ui::fixture::FixtureMeasure;
        plx_machine::motion::hold_phase_clocks(Some(4_000));
        let held = snapshot.set_clock(Some(Wall::from_secs(AT, 3_600)), measure);
        let held_face = snapshot.clock.as_ref().map(|face| face.text.to_str().unwrap().to_owned());
        let again = snapshot.set_clock(Some(Wall::from_secs(AT + 7_200, -18_000)), measure);
        let none = snapshot.set_clock(None, measure);
        plx_machine::motion::hold_phase_clocks(None);
        assert!(held, "the held face shows");
        assert_eq!(held_face.as_deref(), plx_ui::fmt::clock_face(1_735_726_200, 0).as_deref(), "10:10 in UTC");
        assert!(!again, "another instant and zone while held: the same face, no change");
        assert!(none, "and no wall time still means no clock");
    }

    #[test]
    fn published_bar_focus_uses_destination_identity_not_position() {
        let snapshot = ChromeSnapshot { faces: vec![TabFace::Label("Home".into()),
                TabFace::Label("TV Shows".into()), TabFace::Icon(Icon::Search)],
            keys: vec![widgets::TAB_KEY_HOME, widgets::TAB_KEY_SHOWS, widgets::TAB_KEY_SEARCH], ..Default::default() };
        let at = |elem| Some(FocusKey { entry: EntryId(1), elem });
        assert_eq!(snapshot.focus(at(widgets::TAB_KEY_SHOWS)), TopFocus::Pill(1));
        assert_eq!(snapshot.focus(at(widgets::TAB_KEY_SEARCH)), TopFocus::Pill(2));
        assert_eq!(snapshot.focus(at(widgets::TAB_KEY_MOVIES)), TopFocus::Away);
        assert_eq!(snapshot.focus(at(widgets::TAB_KEY_CHIP)), TopFocus::Chip);
        for elem in [0, 1] {
            assert_eq!(snapshot.focus(Some(FocusKey { entry: EntryId(1), elem })), TopFocus::Away);
        }
    }

    #[test]
    fn four_libraries_on_two_servers_publish_two_type_destinations() {
        let directory = directory(&[SecKind::Movie, SecKind::Show, SecKind::Movie, SecKind::Show]);
        let mut snapshot = ChromeSnapshot {
            profile_generation: Some(plx_plex::plex::session::current_gen()), ..Default::default()
        };
        snapshot.refresh(&plx_ui::fixture::FixtureMeasure, directory.view(), true);
        assert_eq!(snapshot.keys, vec![widgets::TAB_KEY_HOME, widgets::TAB_KEY_MOVIES,
            widgets::TAB_KEY_SHOWS, widgets::TAB_KEY_LIVETV, widgets::TAB_KEY_WATCHLIST, widgets::TAB_KEY_SEARCH]);
        let words: Vec<&str> = snapshot.faces.iter().filter_map(|face| match face {
            TabFace::Label(word) => Some(word.as_str()),
            TabFace::Icon(_) => None,
        }).collect();
        assert_eq!(&words[..4], &["Home", "Movies", "TV Shows", "Live TV"]);
        assert_eq!(snapshot.faces.last(), Some(&TabFace::Icon(Icon::Search)));
        assert_eq!(snapshot.faces[snapshot.faces.len() - 2], TabFace::Icon(Icon::Bookmark),
            "the Watchlist bookmark sits just before Search");
        let mut members = Vec::new();
        snapshot.members(0, None, 0.0, &mut members);
        assert_eq!(members.iter().map(|member| member.elem).collect::<Vec<_>>(),
            vec![widgets::TAB_KEY_CHIP, widgets::TAB_KEY_HOME, widgets::TAB_KEY_MOVIES,
                widgets::TAB_KEY_SHOWS, widgets::TAB_KEY_LIVETV, widgets::TAB_KEY_WATCHLIST, widgets::TAB_KEY_SEARCH]);
    }

    #[test]
    fn every_projected_pill_round_trips_by_stable_section_identity() {
        use plx_data::stores::browse::SecKind::{Movie, Show};
        let keys = [widgets::TAB_KEY_HOME, widgets::TAB_KEY_MOVIES, widgets::TAB_KEY_SHOWS,
            widgets::TAB_KEY_LIVETV, widgets::TAB_KEY_WATCHLIST, widgets::TAB_KEY_SEARCH];
        assert_eq!(pill_in(0, &keys), Pill::Home);
        assert_eq!(pill_in(1, &keys), Pill::Section(Movie));
        assert_eq!(pill_in(2, &keys), Pill::Section(Show));
        assert_eq!(pill_in(3, &keys), Pill::LiveTv);
        assert_eq!(pill_in(4, &keys), Pill::Watchlist);
        assert_eq!(pill_in(5, &keys), Pill::Search);
        assert_eq!(pill_in(6, &keys), Pill::Home);
        for i in 0..keys.len() {
            assert_eq!(pill_index(pill_in(i, &keys), &keys), Some(i));
        }
        assert_eq!(pill_index(Pill::Section(Show), &[widgets::TAB_KEY_HOME, widgets::TAB_KEY_SEARCH]), None,
            "a type the captured strip no longer contains borrows no other position");
    }

    #[test]
    fn an_icon_pill_before_search_does_not_shift_the_other_destinations() {
        use plx_data::stores::browse::SecKind::{Movie, Show};
        // A further icon destination (a key none of the maps names) sits before Search.
        let extra = plx_ui::dispatch::STRIP_BASE + 9;
        let keys = [widgets::TAB_KEY_HOME, widgets::TAB_KEY_MOVIES, widgets::TAB_KEY_SHOWS, extra,
            widgets::TAB_KEY_SEARCH];
        assert_eq!(pill_in(3, &keys), Pill::Home, "an unknown key is not Search and not a section");
        assert_eq!(pill_in(4, &keys), Pill::Search);
        assert_eq!(pill_in(2, &keys), Pill::Section(Show));
        assert_eq!(pill_index(Pill::Search, &keys), Some(4));
        assert_eq!(pill_index(Pill::Section(Movie), &keys), Some(1));
    }

    #[test]
    fn pill_maps_follow_the_published_strip_keys() {
        let directory = directory(&[SecKind::Movie, SecKind::Show]);
        let view = directory.view();
        assert_eq!(strip_keys(view, true), vec![widgets::TAB_KEY_HOME, widgets::TAB_KEY_MOVIES,
            widgets::TAB_KEY_SHOWS, widgets::TAB_KEY_LIVETV, widgets::TAB_KEY_WATCHLIST, widgets::TAB_KEY_SEARCH]);
        assert_eq!(pill_of(view, true, Pill::LiveTv), Some(3));
        assert_eq!(pill_of(view, true, Pill::Watchlist), Some(4));
        assert_eq!(pill_of(view, true, Pill::Search), Some(5));
        assert_eq!(pill_at(view, true, 3), Pill::LiveTv);
        assert_eq!(pill_at(view, true, 4), Pill::Watchlist);
        assert_eq!(pill_at(view, true, 5), Pill::Search);
    }

    #[test]
    fn unavailable_live_tv_leaves_watchlist_and_search_on_the_published_strip() {
        let directory = directory(&[SecKind::Movie, SecKind::Show]);
        let view = directory.view();
        assert_eq!(strip_keys(view, false), vec![widgets::TAB_KEY_HOME, widgets::TAB_KEY_MOVIES,
            widgets::TAB_KEY_SHOWS, widgets::TAB_KEY_WATCHLIST, widgets::TAB_KEY_SEARCH]);
        assert_eq!(pill_of(view, false, Pill::LiveTv), None);
        assert_eq!(pill_of(view, false, Pill::Watchlist), Some(3));
        assert_eq!(pill_of(view, false, Pill::Search), Some(4));
        assert_eq!(pill_at(view, false, 3), Pill::Watchlist);
        assert_eq!(pill_at(view, false, 4), Pill::Search);
        for live_tv in [false, true] {
            for i in 0..strip_keys(view, live_tv).len() {
                assert_eq!(pill_of(view, live_tv, pill_at(view, live_tv, i)), Some(i));
            }
        }
    }

    #[test]
    fn live_tv_availability_rebuilds_chrome_without_a_directory_change() {
        let directory = directory(&[SecKind::Movie, SecKind::Show]);
        let profile = plx_plex::plex::session::CurrentProfile {
            user: None, generation: 23, stamp: String::new(),
        };
        let saved = plx_plex::plex::session::Session::default();
        let captured = Some((&profile, &saved));
        let measure = &plx_ui::fixture::FixtureMeasure;
        let mut snapshot = ChromeSnapshot::default();
        snapshot.refresh_with_profile(measure, directory.view(), false, captured);
        let no_tv_keys = snapshot.keys.clone();
        let no_tv_faces = snapshot.faces.clone();
        let no_tv_widths = snapshot.widths.clone();
        let initial_generation = snapshot.labels().generation;
        assert!(!snapshot.keys.contains(&widgets::TAB_KEY_LIVETV));
        assert_eq!(snapshot.tabs_generation, Some(directory.view().tabs_gen()));

        snapshot.refresh_with_profile(measure, directory.view(), true, captured);
        let tv_generation = snapshot.labels().generation;
        assert_eq!(snapshot.keys, strip_keys(directory.view(), true));
        assert_eq!(snapshot.faces[3], TabFace::Label(plx_os::i18n::msg::browse_chrome_live_tv().into()));
        assert_eq!(snapshot.faces.len(), no_tv_faces.len() + 1);
        assert_eq!(snapshot.widths.len(), no_tv_widths.len() + 1);
        assert_eq!(tv_generation, initial_generation.wrapping_add(1), "the renderer must refresh its cached labels");
        assert_eq!(snapshot.tabs_generation, Some(directory.view().tabs_gen()));
        assert_eq!(snapshot.profile_generation, Some(profile.generation));

        snapshot.refresh_with_profile(measure, directory.view(), true, captured);
        assert_eq!(snapshot.labels().generation, tv_generation, "unchanged inputs keep the renderer's cache");
        snapshot.refresh_with_profile(measure, directory.view(), false, captured);
        assert_eq!(snapshot.keys, no_tv_keys);
        assert_eq!(snapshot.faces, no_tv_faces);
        assert_eq!(snapshot.widths, no_tv_widths);
        assert_eq!(snapshot.labels().generation, tv_generation.wrapping_add(1));
        assert_eq!(snapshot.tabs_generation, Some(directory.view().tabs_gen()));
    }

    #[test]
    fn chrome_production_reads_only_its_retained_directory() {
        let src = include_str!("chrome.rs");
        let production = src.split("#[cfg(test)]").next().unwrap();
        assert!(!production.contains("plx_data::browse::"),
            "Chrome must read only its retained Browse directory");
    }

    #[test]
    fn shared_widgets_read_no_live_application_vocabulary() {
        let src = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/src/widgets.rs"),
        ).expect("read widgets.rs");
        let live = src.lines().filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>().join("\n");
        for forbidden in ["plx_data::browse::", "plx_plex::plex::session::", "plx_screens::"] {
            assert!(!live.contains(forbidden),
                "shared widgets must consume captured app projections, found {forbidden}");
        }
    }
}
