//! **Audio & Subtitles before playback** (#1532): the panel a viewer opens to choose the tracks a
//! title starts with. A `Style::Compact` surface anchored to what opened it (the hero's disc, the
//! `…` menu's row, the failure screen's entry), `versions_panel`'s shape. The panel's whole state is
//! `plx_appkit::preplay_tracks::PickerState`; this screen only hosts it (focus stops, input, draw).
//!
//! **A pick is two writes**, both emitted from here the moment OK lands on a track: on a Detail page
//! the metadata store holds the pick (`AppFx::Store(Metadata, PatchTracks)`: the plan applies it at the
//! item's next start, the pick's audio as the session audio and an Off outranking the account's subtitle
//! rule, and the start spends it), and the server gets the selection PUT plus the show's track memory
//! ([`ContentReq::CommitPreplayPick`], run by the loop). The panel stays up with the check moved; BACK
//! pops a page before it dismisses. On the failure screen the store is not written: the loop holds the
//! pair on the session for Try again and closes the panel only when that retry started. That panel is a flat card
//! over the player's video ([`host_of`]: `menu::Host::Video`), never glass or the sheet ground.
//!
//! **The glue is the menu's.** OK, BACK, LEFT, RIGHT, the hover and the pointer hold all go through
//! `plx_ui::menu::Menu::on_screen` and `menu_screen_methods!`; this screen only performs what comes back
//! (a pick commits, a dismissal closes).
//!
//! **Entry point.** Anything that wants the panel sends
//! `ContentReq::Panel(ContentPanel::PreplayTracks { anchor })` from a Detail page: the panel's
//! subject is the page's item, and the tracks are the ones of the item the hero PLAYS
//! (`Detail::play_rk`: on a show page the hero episode, never the show). The playback failure
//! screen opens the same surface with [`Source::Failed`], which reads the tracks of the item that
//! was playing instead of a page's.

use std::borrow::Cow;

use plx_appkit::preplay_tracks::PickerState;
use plx_data::metadata::PreplayTracks;
use plx_data::stores::{StoreCmd, StoreId};
use plx_machine::machine::{
    Canon, Cx, Effects, EntryId, FocusKey, Fx, GroupId, Handled, InstanceId, LogicalState, Machine, NavOp,
};
use plx_plex::plex::ServerId;
use plx_ui::a11y::{A11y, A11yPath};
use plx_ui::menu::{Host, Menu, Out};
use plx_ui::popover::Placement;
use plx_ui::page_stack;
use plx_ui::screen::{
    At, Dir, FocusSource, Focusable, GroupSpec, HitSource, Placed, RenderStrategy, Screen, ScreenEvent,
    Step,
};

use crate::registry::{AppFx, AppLike, ContentReq, MetadataLike, PageMemory};

/// The fields [`PreplayPanelScreen::write`] canonicalises, for the recorder's shape pin (§5.4).
pub const SHAPE: &str = "PreplayPanel{arg:PreplayArg{host:u32,sid:u32,rk:str,anchor:[u32;4],opener:Option<{entry:u32,elem:u32}>,source:{Page,Failed{part_id:u64}}},audio:u32,sub:Option<u32>,depth:u32}";

/// Where the panel's tracks come from.
#[derive(Clone, Copy, Debug)]
pub enum Source {
    /// The Detail page the panel is over: `PreplayTracks` of the item its hero plays.
    Page,
    /// The playback failure screen: the item that was playing (`metadata.playing()`) and the part of
    /// the failed attempt, which is the part the selection PUT addresses.
    ///
    /// `pick` seeds the rows: the session's held pick, else the attempt's standing selection
    /// (`route::failed_pick`), so a reopen shows what Try again would play. A seed like the page's
    /// store pick, not part of the surface's identity (the canon below does not write it).
    Failed { part_id: i64, pick: Option<plx_data::metadata::PreplayPick> },
}

/// **Identity is where the tracks come from, not what seeded the rows**: two failure panels over the
/// same part are the same surface whatever pick they open on (the canon writes the part only), so
/// `pick` is left out of the comparison as it is out of the shape.
impl PartialEq for Source {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Source::Page, Source::Page) => true,
            (Source::Failed { part_id: a, .. }, Source::Failed { part_id: b, .. }) => a == b,
            _ => false,
        }
    }
}
impl Eq for Source {}

/// What the container is asked to present: the host instance, the item the page stands on, the drawn
/// rect of what opened the panel (bit-preserving, like every panel argument) and where the tracks
/// are read from.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PreplayArg {
    pub host: InstanceId,
    pub sid: ServerId,
    pub rk: String,
    pub anchor: [u32; 4],
    /// The page's entry and the hero control the panel grows out of and drains back into; `None` on
    /// the failure screen, whose panel keeps its fade (nothing of the page to lift).
    pub opener: Option<(EntryId, u32)>,
    pub source: Source,
}

impl LogicalState for PreplayArg {
    fn write(&self, c: &mut Canon) {
        c.u32(self.host.0).u32(u32::from(self.sid.raw())).str(&self.rk);
        for value in self.anchor {
            c.u32(value);
        }
        c.option(self.opener, |c, (entry, elem)| { c.u32(entry.0).u32(elem); });
        match self.source {
            Source::Page => {
                c.u32(0);
            }
            Source::Failed { part_id, .. } => {
                c.u32(1).u64(part_id as u64);
            }
        }
    }
    fn probe(&self, out: &mut String) {
        out.push_str("preplay_arg");
    }
}

/// **The host the panel stands on, chosen from where its tracks come from** and never from whether an opener was
/// named: the playback failure screen's panel stands over the VIDEO PLANE (the player stays flat over video,
/// `docs/liquid-glass.md` §7), a card hung under or above the button that opened it ([`Placement::UNDER_OR_ABOVE_FIT`]),
/// whatever its argument says about an opener. A Detail page's panel grows out of the hero control that opened it
/// (glass, over the control), else hangs under or above its opener on the sheet ground.
pub(crate) fn host_of(arg: &PreplayArg) -> Host {
    match (arg.source, arg.opener) {
        (Source::Failed { .. }, _) => Host::Video { anchor: arg.anchor, placement: Placement::UNDER_OR_ABOVE_FIT },
        (Source::Page, Some(_)) => plx_ui::detail_layout::hero_host(arg.anchor),
        (Source::Page, None) => Host::Sheet { anchor: arg.anchor },
    }
}

pub struct PreplayPanelScreen {
    entry: EntryId,
    arg: PreplayArg,
    /// The tracks, the choice and the menu that shows them, on the host [`host_of`] names.
    picker: PickerState,
}

impl PreplayPanelScreen {
    /// `tracks` is what the item offers, read by the registry's mount from [`Source`]; a page that
    /// cannot say gets an empty set, which lists its two rows with nothing behind them.
    pub fn new(entry: EntryId, arg: PreplayArg, tracks: PreplayTracks) -> Self {
        let picker = PickerState::new(tracks, host_of(&arg));
        Self { entry, arg, picker }
    }

    pub fn picker(&self) -> &PickerState {
        &self.picker
    }

    fn dismiss<H: AppLike<Memory = PageMemory>>(&self, fx: &mut Effects<'_, H>) {
        fx.push(Fx::Nav(NavOp::Dismiss(self.entry)));
    }

    /// What the menu answered: a dismissal closes the surface, and a pick of a track is carried forward by the
    /// picker and committed (a drill-in is a page the menu pushed). The row is the event's own (`Glued::elem`).
    fn run<H: AppLike<Memory = PageMemory>>(&mut self, out: Out<()>, elem: Option<u32>, fx: &mut Effects<'_, H>) {
        match out {
            Out::Stay | Out::Moved => {}
            Out::Dismiss => self.dismiss(fx),
            Out::Picked(()) => {
                if let Some(sel) = elem.and_then(|key| self.picker.pick(key)) {
                    commit_pick(fx, self.picker.tracks().clone(), sel, self.arg.source);
                }
            }
        }
    }
}

/// **A pick of Audio & Subtitles, as effects: the ONE pair of every surface that lists the tracks** (this
/// panel, the hero `…` menu's pages). `tracks` is the set after the pick, `sel` what was chosen.
///
/// **The store keeps only what the viewer chose for the item's next start**, and a page's pick is that.
/// The failure screen's pair is the session's (`route::hold_pick_for_retry`, run by the loop's commit):
/// its standing half is DERIVED from the failed attempt (a `0` subtitle reads as `Off`), which is not
/// the viewer's word, so it never reaches the store. On the failure screen the loop starts the retry
/// with the commit and closes the panel only when the retry started (`content_command`), so a refused
/// retry keeps it.
pub(crate) fn commit_pick<H: AppLike>(
    fx: &mut Effects<'_, H>,
    tracks: PreplayTracks,
    sel: plx_appkit::preplay_tracks::Selection,
    source: Source,
) {
    if matches!(source, Source::Page) {
        fx.push(Fx::App(AppFx::Store(
            StoreId::Metadata,
            StoreCmd::Metadata(plx_appkit::preplay_tracks::patch_command(&tracks, &sel)),
        )));
    }
    fx.push(Fx::App(AppFx::Content(ContentReq::CommitPreplayPick { tracks, sel, source })));
}

impl<H: AppLike<Memory = PageMemory>> Machine<H> for PreplayPanelScreen {
    type Ev = ScreenEvent<H>;
    fn step(&mut self, ev: &Self::Ev, cx: &Cx<'_, H>, fx: &mut Effects<'_, H>) -> Handled {
        // all of it the menu's (`Menu::on_screen`): BACK and LEFT pop a page, RIGHT enters a drill-in row; at the
        // root LEFT is inert and BACK closes; however the surface was dismissed the glass runs its morph back
        match self.picker.on_screen(ev, cx, self.entry, fx) {
            Some(glued) => {
                self.run(glued.out, glued.elem, fx);
                glued.handled
            }
            None => Handled::No,
        }
    }
}

impl plx_ui::menu::HasMenu for PreplayPanelScreen {
    type Page = plx_appkit::preplay_tracks::PickPage;
    type Row = plx_appkit::preplay_tracks::PickRow;
    type Act = ();
    fn menu(&self) -> &Menu<plx_appkit::preplay_tracks::PickPage, plx_appkit::preplay_tracks::PickRow, ()> {
        self.picker.menu()
    }
    fn menu_entry(&self) -> EntryId {
        self.entry
    }
}

const GROUP: GroupId = GroupId(0);

impl<H: AppLike<Memory = PageMemory> + MetadataLike> Focusable<H> for PreplayPanelScreen {
    fn groups(&self, cx: &Cx<'_, H>, out: &mut Vec<GroupSpec>) {
        self.picker.menu().groups(GROUP, cx.measure, out);
    }
    fn group_of(&self, elem: &u32, _cx: &Cx<'_, H>) -> Option<GroupId> {
        self.picker.menu().group_of(elem, GROUP)
    }
    fn neighbour(&self, key: FocusKey<u32>, dir: Dir, _cx: &Cx<'_, H>) -> Step<u32> {
        self.picker.menu().neighbour(self.entry, key, dir)
    }
    fn place(&self, elem: &u32, cx: &Cx<'_, H>, _at: At) -> Option<Placed> {
        self.picker.menu().place(elem, cx.measure)
    }
    fn reconcile(&self, want: FocusKey<u32>, _cx: &Cx<'_, H>) -> FocusKey<u32> {
        self.picker.menu().reconcile(self.entry, want, GROUP)
    }
    fn seat(&self, _g: GroupId, _from: Placed, _cx: &Cx<'_, H>) -> FocusKey<u32> {
        self.picker.menu().seat(self.entry)
    }
    /// `preplay.<row>`: the two root rows by name, every track row by its index in the item's list.
    fn a11y(&self, key: &u32, _cx: &Cx<'_, H>) -> Option<A11y<'_>> {
        use plx_appkit::preplay_tracks::PickRow;
        page_stack::popover_a11y(self.picker.menu().form(), key, |id| match *id {
            PickRow::OpenAudio => A11yPath::new().name("audio"),
            PickRow::OpenSubtitles => A11yPath::new().name("subtitles"),
            PickRow::SubOff => A11yPath::new().name("subtitle").name("off"),
            PickRow::Audio(i) => A11yPath::new().name("audio").index(i),
            PickRow::Sub(i) => A11yPath::new().name("subtitle").index(i),
        })
    }
    fn a11y_elems(&self, g: GroupId, _cx: &Cx<'_, H>, out: &mut Vec<u32>) {
        page_stack::popover_a11y_elems(self.picker.menu().form(), g, GROUP, out);
    }
}

impl LogicalState for PreplayPanelScreen {
    fn write(&self, c: &mut Canon) {
        self.arg.write(c);
        let now = self.picker.now();
        c.u32(now.audio as u32);
        match now.sub {
            Some(i) => c.u32(1).u32(i as u32),
            None => c.u32(0),
        };
        c.u32(self.picker.depth() as u32);
    }
    fn probe(&self, out: &mut String) {
        out.push_str("preplay");
    }
}

impl<H: AppLike<Memory = PageMemory> + MetadataLike> Screen<H> for PreplayPanelScreen {
    fn name(&self) -> &'static str {
        "preplay"
    }
    fn state(&self) -> &dyn LogicalState {
        self
    }
    fn crumb(&self, _cx: &Cx<'_, H>) -> Option<Cow<'_, str>> {
        None
    }
    /// The hero control the glass grew out of (see `VersionsPanelScreen::opener`). The failure screen's panel has none,
    /// whatever its argument names: it stands over the video plane, with nothing of a page to lift.
    fn opener(&self) -> Option<(EntryId, Option<FocusKey<u32>>)> {
        match self.arg.source {
            Source::Failed { .. } => None,
            Source::Page => self.arg.opener.map(|(entry, elem)| (entry, Some(FocusKey { entry, elem }))),
        }
    }
    // The menu's own (`plx_ui::menu_screen_methods!`): the opener consumed by the morph (gone while the panel stands,
    // back as it drains), the hand-back, the dim (a morphing panel's FOLLOWS the morph), the prewarm, the pointer
    // hold and the draw.
    plx_ui::menu_screen_methods!(H, "dt.preplay");
    fn render(&self) -> RenderStrategy {
        RenderStrategy::Page
    }
    fn focus_source(&self) -> FocusSource {
        FocusSource::Engine
    }
    fn hit_source(&self) -> HitSource {
        HitSource::Engine
    }
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use plx_ui::screen::DrawFrame;
    use super::*;
    use crate::registry::AppMsg;
    use plx_data::metadata::Stream;
    use plx_machine::machine::{Host, MachineId};
    use plx_ui::screen::ScreenArg;

    #[derive(Clone)]
    struct Arg;
    impl LogicalState for Arg {
        fn write(&self, _: &mut Canon) {}
        fn probe(&self, _: &mut String) {}
    }
    impl ScreenArg for Arg {
        fn chrome(&self) -> plx_machine::machine::Chrome {
            plx_machine::machine::Chrome::None
        }
        fn id(&self) -> plx_machine::machine::ScreenId {
            plx_machine::machine::ScreenId(1)
        }
        fn title(&self) -> Option<&str> {
            None
        }
        fn same_instance(&self, _: &Self) -> bool {
            true
        }
    }
    struct HostFixture;
    impl Host for HostFixture {
        type Arg = Arg;
        type Fx = AppFx;
        type Msg = AppMsg;
        type Elem = u32;
        type Views<'a> = ();
        type Init = Arg;
        type Memory = PageMemory;
    }
    thread_local! {
        static TEST_METADATA: std::cell::UnsafeCell<plx_data::stores::metadata::MetadataStore> =
            std::cell::UnsafeCell::new(plx_data::stores::metadata::MetadataStore::default());
    }
    impl MetadataLike for HostFixture {
        fn metadata<'a>(_cx: &Cx<'a, Self>) -> plx_data::metadata::MetadataView<'a> {
            TEST_METADATA.with(|cell| unsafe { &mut *cell.get() }).view()
        }
    }

    fn stream(id: i64, lang: &str, code: &str) -> Stream {
        Stream { id, lang: lang.into(), lang_code: code.into(), ..Stream::default() }
    }

    fn tracks() -> PreplayTracks {
        PreplayTracks {
            sid: ServerId::from_raw(1),
            rk: "42".into(),
            kind: "movie".into(),
            show_rk: String::new(),
            part: "/library/parts/9001/1/file.mkv".into(),
            audio: vec![stream(101, "English", "eng"), stream(102, "French", "fra")],
            subs: vec![stream(201, "English", "eng")],
            pick: None,
        }
    }

    fn panel() -> PreplayPanelScreen {
        panel_from(Source::Page)
    }

    fn panel_from(source: Source) -> PreplayPanelScreen {
        let anchor = [500.0f32, 600.0, 64.0, 64.0].map(f32::to_bits);
        let arg = PreplayArg { host: InstanceId(9), sid: ServerId::from_raw(1), rk: "42".into(), anchor, opener: None, source };
        PreplayPanelScreen::new(EntryId(3), arg, tracks())
    }

    fn cx() -> Cx<'static, HostFixture> {
        use plx_machine::machine::{FocusRead, InputOwner, PressRead, Tick};
        Cx {
            views: (),
            tick: Tick::default(),
            measure: &plx_ui::fixture::FixtureMeasure,
            focus: FocusRead::default(),
            press: PressRead::default(),
            modal: Default::default(), page_image: Default::default(), hold_hint: Default::default(),
            owner: InputOwner::Entry(EntryId(3)),
        }
    }

    /// The panel's root page, run through the id conformance (`AppArg::PreplayPanel`; the coverage
    /// table calls this).
    pub(crate) fn preplay_panel_conforms() -> plx_ui::a11y::Namespaces {
        vec![plx_ui::a11y::conform::<HostFixture, _>(&panel(), &cx())]
    }

    /// A key press (down edge), re-delivered at the edge as the menu's one group hands it to its screen.
    fn key_ev(key: plx_machine::machine::Key) -> ScreenEvent<HostFixture> {
        use plx_machine::machine::{Edge, InputEvent, InputKind, Source as InputSource, Tick};
        ScreenEvent::<HostFixture>::Input(InputEvent {
            at: Tick::default(),
            source: InputSource::Script,
            kind: InputKind::Key { key, sym: 0, wcode: 0, edge: Edge::Down, at_edge: true },
        })
    }

    /// One screen event, as the dispatcher delivers it.
    fn press(p: &mut PreplayPanelScreen, ev: ScreenEvent<HostFixture>, fx: &mut Effects<'_, HostFixture>) {
        Machine::<HostFixture>::step(p, &ev, &cx(), fx);
    }

    fn drive(
        p: &mut PreplayPanelScreen,
        f: impl FnOnce(&mut PreplayPanelScreen, &mut Effects<'_, HostFixture>),
    ) -> Vec<Fx<HostFixture>> {
        let mut out = Vec::new();
        let mut present = plx_machine::present::Present::new();
        let mut fx: Effects<'_, HostFixture> = Effects::new(&mut out, MachineId::Instance(InstanceId(4)), &mut present);
        f(p, &mut fx);
        out.into_iter().map(|e| e.fx).collect()
    }

    /// OK on a drill-in opens the page and writes nothing; OK on a track emits both writes (the
    /// store patch and the addressed commit); BACK pops a page before it dismisses.
    #[test]
    fn a_pick_emits_the_patch_and_the_commit_and_back_pops_before_it_dismisses() {
        use plx_appkit::preplay_tracks::PickRow;
        let mut p = panel();
        let elem = |row: PickRow| plx_ui::form::FormId::key(&row).0;
        let fx = drive(&mut p, |p, fx| press(p, ScreenEvent::Activate(elem(PickRow::OpenSubtitles)), fx));
        assert!(fx.is_empty(), "a drill-in picks nothing");
        assert_eq!(p.picker().depth(), 1);
        let fx = drive(&mut p, |p, fx| press(p, ScreenEvent::Activate(elem(PickRow::SubOff)), fx));
        let patched = fx.iter().any(|f| {
            matches!(
                f,
                Fx::App(AppFx::Store(
                    StoreId::Metadata,
                    StoreCmd::Metadata(plx_data::stores::metadata::MetadataCmd::PatchTracks {
                        pick: plx_data::metadata::PreplayPick { sub: plx_data::metadata::SubPick::Off, .. },
                        ..
                    })
                ))
            )
        });
        let committed = fx.iter().any(|f| {
            matches!(f, Fx::App(AppFx::Content(ContentReq::CommitPreplayPick { sel, .. })) if sel.subtitle_sid == 0)
        });
        assert!(patched && committed, "Off patches the page and commits the PUT");
        let fx = drive(&mut p, |p, fx| press(p, key_ev(plx_machine::machine::Key::Back), fx));
        assert!(fx.is_empty() && p.picker().depth() == 0, "BACK pops the subtitle page first");
        let fx = drive(&mut p, |p, fx| press(p, key_ev(plx_machine::machine::Key::Back), fx));
        assert!(matches!(fx.first(), Some(Fx::Nav(NavOp::Dismiss(EntryId(3))))), "BACK at the root closes");
    }

    /// **RIGHT enters a drill-in row and LEFT pops a page, as in the player**; at the root LEFT is inert, and
    /// neither closes the panel (only BACK does). Both arrive re-delivered at the edge (`at_edge`).
    #[test]
    fn right_enters_a_drill_in_and_left_pops_without_ever_closing() {
        use plx_appkit::preplay_tracks::PickRow;
        use plx_machine::machine::{Edge, InputEvent, InputKind, Key, Source as InputSource, Tick};
        let key = |key: Key, edge: Edge| {
            ScreenEvent::<HostFixture>::Input(InputEvent {
                at: Tick::default(),
                source: InputSource::Script,
                kind: InputKind::Key { key, sym: 0, wcode: 0, edge, at_edge: true },
            })
        };
        let focus_on = |row: PickRow| {
            let mut c = cx();
            let elem = plx_ui::form::FormId::key(&row).0;
            c.focus.current = Some(FocusKey { entry: EntryId(3), elem });
            c
        };
        let mut p = panel();
        let send = |p: &mut PreplayPanelScreen, ev: ScreenEvent<HostFixture>, c: &Cx<'_, HostFixture>| {
            let mut out = Vec::new();
            let mut present = plx_machine::present::Present::new();
            let mut fx: Effects<'_, HostFixture> = Effects::new(&mut out, MachineId::Instance(InstanceId(4)), &mut present);
            let handled = Machine::<HostFixture>::step(p, &ev, c, &mut fx);
            (handled, out.len())
        };
        assert_eq!(send(&mut p, key(Key::Left, Edge::Down), &focus_on(PickRow::OpenAudio)), (Handled::Yes, 0), "LEFT at the root is inert");
        assert_eq!(p.picker().depth(), 0);
        assert_eq!(send(&mut p, key(Key::Right, Edge::Down), &focus_on(PickRow::OpenSubtitles)), (Handled::Yes, 0));
        assert_eq!(p.picker().depth(), 1, "RIGHT on a drill-in row enters it");
        assert_eq!(send(&mut p, key(Key::Right, Edge::Down), &focus_on(PickRow::SubOff)), (Handled::Yes, 0));
        assert_eq!(p.picker().depth(), 1, "RIGHT on a choice row does nothing");
        assert_eq!(send(&mut p, key(Key::Left, Edge::Up), &focus_on(PickRow::SubOff)), (Handled::Yes, 0));
        assert_eq!(p.picker().depth(), 1, "the up edge is swallowed");
        assert_eq!(send(&mut p, key(Key::Left, Edge::Down), &focus_on(PickRow::SubOff)), (Handled::Yes, 0));
        assert_eq!(p.picker().depth(), 0, "LEFT pops the page");
    }

    /// A source's identity is where the tracks come from: the seed pick rides along but is not compared.
    #[test]
    fn a_source_is_the_same_surface_whatever_seeded_its_rows() {
        let seeded = plx_data::metadata::PreplayPick { part_id: 9001, audio_sid: 11, sub: plx_data::metadata::SubPick::Off };
        let failed = |part_id, pick| Source::Failed { part_id, pick };
        assert_eq!(failed(9001, None), failed(9001, Some(seeded)));
        assert_ne!(failed(9001, None), failed(9002, None), "another part is another surface");
        assert_ne!(Source::Page, failed(9001, None));
        assert_eq!(Source::Page, Source::Page);
    }

    /// On the failure screen a pick commits with the failure source (the loop holds it AND starts the
    /// retry, as a quality pick does) and leaves closing to it; on a page the panel stays up.
    #[test]
    fn a_failure_pick_commits_for_the_retry_and_never_patches_the_store() {
        use plx_appkit::preplay_tracks::PickRow;
        let elem = |row: PickRow| plx_ui::form::FormId::key(&row).0;
        let mut p = panel_from(Source::Failed { part_id: 9001, pick: None });
        drive(&mut p, |p, fx| press(p, ScreenEvent::Activate(elem(PickRow::OpenSubtitles)), fx));
        let fx = drive(&mut p, |p, fx| press(p, ScreenEvent::Activate(elem(PickRow::SubOff)), fx));
        assert!(fx.iter().any(|f| matches!(
            f,
            Fx::App(AppFx::Content(ContentReq::CommitPreplayPick { source: Source::Failed { .. }, sel, .. })) if sel.subtitle_sid == 0
        )));
        assert!(!fx.iter().any(|f| matches!(f, Fx::Nav(_))), "the loop closes the panel, and only when the retry started");
        assert!(
            !fx.iter().any(|f| matches!(f, Fx::App(AppFx::Store(StoreId::Metadata, _)))),
            "the failure screen's pair is the session's: a derived standing Off is never stored as the viewer's pick"
        );
        let mut page = panel();
        drive(&mut page, |p, fx| press(p, ScreenEvent::Activate(elem(PickRow::OpenSubtitles)), fx));
        let fx = drive(&mut page, |p, fx| press(p, ScreenEvent::Activate(elem(PickRow::SubOff)), fx));
        assert!(!fx.iter().any(|f| matches!(f, Fx::Nav(_))), "a Detail page's panel stays up with the check moved");
    }

    // ---- the hero control's morph ------------------------------------------------------------

    fn morphing(opener: Option<(EntryId, u32)>, source: Source) -> PreplayPanelScreen {
        let anchor = [500.0f32, 700.0, 64.0, 64.0].map(f32::to_bits);
        let arg = PreplayArg { host: InstanceId(9), sid: ServerId::from_raw(1), rk: "42".into(), anchor, opener, source };
        PreplayPanelScreen::new(EntryId(3), arg, tracks())
    }

    fn step(p: &mut PreplayPanelScreen, ev: ScreenEvent<HostFixture>) {
        let mut out = Vec::new();
        let mut present = plx_machine::present::Present::new();
        let mut fx: Effects<'_, HostFixture> = Effects::new(&mut out, MachineId::Instance(InstanceId(4)), &mut present);
        Machine::<HostFixture>::step(p, &ev, &cx(), &mut fx);
    }

    fn tick(p: &mut PreplayPanelScreen, ms: &mut u32, n: usize) {
        for _ in 0..n {
            *ms += 16;
            step(p, ScreenEvent::Tick(plx_machine::machine::Tick { ms: *ms, dt_us: 16_667 }));
        }
    }

    /// The glass this frame: the panel ground (a `rect_rimmed`, tag 8, on a host with no frozen field).
    fn glass(p: &mut PreplayPanelScreen) -> plx_ui::Rect {
        plx_ui::draw_census::capture(|| {
            let cx = cx();
            let mut f = DrawFrame::new(&cx, plx_ui::Painter::recording());
            Screen::<HostFixture>::draw(p, &mut f);
        })
        .into_iter()
        .find_map(|(tag, r)| (tag == 8).then_some(r))
        .unwrap_or_default()
    }

    /// **Audio & Subtitles GROWS out of the hero control that opened it** (the `…` button or its own
    /// disc), as every header menu does: the control is the opener, the panel covers it, three frames in
    /// the glass is still the control's size or a little over, the settled panel is exactly the
    /// picker's rect, and BACK runs the morph home. The failure screen's panel names no opener and
    /// keeps its fade, hung under or above its control.
    #[test]
    fn the_panel_grows_out_of_its_control_settles_on_its_frame_and_runs_home_on_back() {
        let _guard = plx_base::testlock::serial();
        let mut p = morphing(Some((EntryId(1), 9)), Source::Page);
        assert_eq!(
            Screen::<HostFixture>::opener(&p),
            Some((EntryId(1), Some(FocusKey { entry: EntryId(1), elem: 9 }))),
        );
        step(&mut p, ScreenEvent::Mount);
        let settled = p.picker().panel_rect(&plx_ui::fixture::FixtureMeasure);
        let pill = plx_ui::detail_layout::hero_pill(p.arg.anchor);
        assert!(
            settled.x <= pill.x && settled.y <= pill.y && settled.x + settled.w >= pill.x + pill.w && settled.y + settled.h >= pill.y + pill.h,
            "the settled panel {settled:?} covers the control {pill:?}"
        );
        assert_eq!(Screen::<HostFixture>::opener_alpha(&p), 1.0);
        assert!(Screen::<HostFixture>::scrim(&p).ramp.is_some(), "the dim follows the morph");
        let mut ms = 0;
        tick(&mut p, &mut ms, 3);
        let early = glass(&mut p);
        assert!(
            early.w < settled.w - 20.0 && early.h < settled.h - 20.0,
            "three frames in the glass is still growing out of the control ({early:?}), not laid at {settled:?}"
        );
        tick(&mut p, &mut ms, 96);
        let end = glass(&mut p);
        assert!(
            (end.x - settled.x).abs() < 0.5 && (end.y - settled.y).abs() < 0.5
                && (end.w - settled.w).abs() < 0.5 && (end.h - settled.h).abs() < 0.5,
            "the settled panel is {settled:?}, drew {end:?}"
        );
        assert_eq!(Screen::<HostFixture>::opener_alpha(&p), 0.0);
        step(&mut p, ScreenEvent::Closing);
        tick(&mut p, &mut ms, 3);
        assert!(glass(&mut p).w < settled.w - 20.0, "the close narrows the glass into the control");
        tick(&mut p, &mut ms, 60);
        assert!(Screen::<HostFixture>::handed_back(&p), "the droplet has drained into the control");
        assert_eq!(Screen::<HostFixture>::opener_alpha(&p), 1.0);

        // the failure screen's panel: no opener, no morph, hung beside its control as before
        let mut failed = morphing(None, Source::Failed { part_id: 9001, pick: None });
        step(&mut failed, ScreenEvent::Mount);
        step(&mut failed, ScreenEvent::Closing);
        assert_eq!(Screen::<HostFixture>::opener(&failed), None);
        assert_eq!(Screen::<HostFixture>::opener_alpha(&failed), 1.0);
        assert!(!Screen::<HostFixture>::handed_back(&failed));
        assert!(Screen::<HostFixture>::scrim(&failed).ramp.is_none());
        let [x, y, w, h] = [500.0f32, 700.0, 64.0, 64.0];
        let m = &plx_ui::fixture::FixtureMeasure;
        let beside = plx_ui::popover::under_or_above_fit(
            plx_ui::Rect::new(x, y, w, h),
            failed.picker().menu().form().table.menu_panel_width(m),
            failed.picker().menu().form().table.measured_height(),
        );
        assert_eq!(failed.picker().panel_rect(m), beside);
    }

    /// **The host is chosen from the SOURCE, never from whether an opener was named.** The failure screen's panel stands
    /// over the video plane: flat (never the sheet ground, never glass: the player stays flat over video), the neutral
    /// dim, no opener to lift, under or above its button by the fit rule. An argument that names an opener on the failure
    /// screen is the same card, not a glass growing out of a control that is not there.
    #[test]
    fn the_failure_panel_is_a_flat_card_over_the_video_whatever_its_argument_names() {
        use plx_ui::menu::Surface;
        let failed = Source::Failed { part_id: 9001, pick: None };
        for opener in [None, Some((EntryId(1), 9))] {
            let p = morphing(opener, failed);
            assert_eq!(p.picker().menu().surface(), Surface::Flat, "{opener:?}");
            assert_eq!(Screen::<HostFixture>::opener(&p), None, "{opener:?}: nothing of a page to lift");
            let scrim = Screen::<HostFixture>::scrim(&p);
            assert_eq!(scrim.source, plx_ui::screen::UnderlaySource::Flat, "{opener:?}: the dim is neutral ink over the picture");
            assert_eq!(p.picker().menu().form().table.min_panel_w, plx_ui::theme::layout::PLAYER_MENU_MIN_W);
            assert_eq!(Screen::<HostFixture>::opener_alpha(&p), 1.0, "{opener:?}: no morph");
        }
        // a Detail page's: glass out of its control, or the sheet when it has none
        assert_eq!(morphing(Some((EntryId(1), 9)), Source::Page).picker().menu().surface(), Surface::Glass);
        assert_eq!(morphing(None, Source::Page).picker().menu().surface(), Surface::Sheet);
        assert!(matches!(host_of(&morphing(Some((EntryId(1), 9)), failed).arg), plx_ui::menu::Host::Video { .. }));
    }

    /// **The pointer waits while the card moves**, through the menu's one forwarding (`menu_screen_methods!`): a host
    /// that forgot it would act on a card still sliding between two pages.
    #[test]
    fn the_pointer_waits_while_a_page_slides() {
        use plx_appkit::preplay_tracks::PickRow;
        let mut p = panel();
        assert!(!Screen::<HostFixture>::pointer_held(&p));
        drive(&mut p, |p, fx| press(p, ScreenEvent::Activate(plx_ui::form::FormId::key(&PickRow::OpenSubtitles).0), fx));
        assert!(Screen::<HostFixture>::pointer_held(&p), "the pages slide");
        let mut ms = 0;
        tick(&mut p, &mut ms, 120);
        assert!(!Screen::<HostFixture>::pointer_held(&p), "and settle");
    }

    /// OK goes through the menu: a click on the header's back mark pops, and an OK on a key that names no row only closes
    /// (the menu's own table, not a ladder of the host's).
    #[test]
    fn ok_is_the_menus_the_back_mark_pops_and_no_row_closes() {
        use plx_appkit::preplay_tracks::PickRow;
        let mut p = panel();
        drive(&mut p, |p, fx| press(p, ScreenEvent::Activate(plx_ui::form::FormId::key(&PickRow::OpenAudio).0), fx));
        assert_eq!(p.picker().depth(), 1);
        let fx = drive(&mut p, |p, fx| press(p, ScreenEvent::Activate(plx_ui::page_stack::TITLE_KEY), fx));
        assert!(fx.is_empty() && p.picker().depth() == 0, "the back mark pops");
        let fx = drive(&mut p, |p, fx| press(p, ScreenEvent::Activate(999), fx));
        assert!(matches!(fx.first(), Some(Fx::Nav(NavOp::Dismiss(EntryId(3))))), "no row: the menu closes");
    }
}
