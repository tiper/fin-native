//! **Select Version** — the Detail hero's list of a movie's or episode's versions, opened by the
//! Select Version disc.
//!
//! A `Style::Compact` surface anchored to the disc that opened it, the *Also available* panel's
//! shape (`screens::alt_sources`): the anchor rides on [`VersionsArg`], the surface owns input
//! while it is up, and it REPORTS the choice. A row delivers [`AppMsg::SelectVersion`] to the Detail
//! instance named on its argument and the PAGE records it as the file its Play and Resume start; the
//! row plays nothing, and the check mark moves to it. So "what Play means on this page" stays in one
//! place.
//!
//! The rows are `plx_appkit::version_list`, the same builder the item menu's Play Version page
//! draws, captured when the panel opens: a refetch landing behind an open list does not move a
//! row under the cursor. It opens on the row that wears the check mark, the version Play takes now
//! (the viewer's choice, else what plays unasked).
//!
//! **It grows out of the hero control that opened it** (`menu::Host::Grows`, built by
//! `detail_layout::hero_host`, the path every header menu takes) when the argument names that control
//! ([`VersionsArg::opener`]): the page leaves the control out of its own pass and the opener lift draws it at
//! the morph's `opener_alpha`, so the panel covers it and BACK or a pick drains the glass back into it. A
//! panel with no opener is a `Host::Sheet` that keeps the fade. The panel is a one-page `ui::menu::Menu`: the
//! placement, the ground, the radius, the stops and the dim are the menu's.

use std::borrow::Cow;
use std::convert::Infallible;

use plx_data::pms::MediaVersion;
use plx_plex::plex::{MediaId, ServerId};
use crate::registry::{AppFx, AppLike, AppMsg, MetadataLike, PageMemory, PlayerReq};
use plx_ui::a11y::{A11y, A11yPath};
use plx_machine::machine::{
    Canon, Cx, Delivery, Effects, EntryId, FocusKey, Fx, GroupId, Handled, InstanceId, LogicalState, Machine,
    MachineId, NavOp,
};
use plx_ui::menu::{no_pages, Class, Event, Host, Menu, Out, Page, Shade};
use plx_ui::popover::Placement;
use plx_ui::screen::{
    At, Dir, FocusSource, Focusable, GroupSpec, HitSource, Placed, RenderStrategy, Screen, ScreenEvent,
    Step,
};
use plx_ui::form::{Form, FormId, RowKey};
use plx_ui::Rect;

/// The fields [`VersionsPanelScreen::write`] canonicalises, for the recorder's shape pin (§5.4).
pub const SHAPE: &str =
    "VersionsPanel{arg:VersionsArg{host:u32,sid:u32,rk:str,anchor:[u32;4],opener:Option<{entry:u32,elem:u32}>},versions:[i64],sel:i32,table:TableViewMotion}";

/// What the container is asked to present: the Detail instance the choice is reported to, the item
/// whose versions these are, and the drawn rect of the disc it hangs off (bit-preserving, so a
/// canonical argument holds it without float equality).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct VersionsArg {
    pub host: InstanceId,
    pub sid: ServerId,
    pub rk: String,
    pub anchor: [u32; 4],
    /// The page's entry and the hero control the panel grows out of and drains back into (the `…`
    /// button when the menu handed the row over, the disc itself otherwise): the page leaves it out
    /// of its own pass and the opener lift draws it at the morph's alpha ([`Screen::opener`]).
    pub opener: Option<(EntryId, u32)>,
    /// Whose versions these are and where a pick goes ([`Source`]). Not in the canon: `host` already tells the two
    /// apart (a Detail page's instance, or the player's), and the recorder's shape pin stays what it was.
    pub source: Source,
}

/// **Where the list comes from and where a pick goes**: a Detail page's hero (the row reports the file to the page,
/// which records it for its Play and Resume), or the playback failure screen (the row asks the one recovery door to
/// play the failed item on that file, `Recovery::PickVersion`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    Page,
    /// `attempted` is the file the failed attempt asked for (`None`: it asked for none, so the one that plays unasked
    /// wears the check mark). `refused` is the file the failed attempt landed on (`route::attempt_media`) when a refusal
    /// stands (`player::version_repeat_refused`): its row is disabled, since choosing it would repeat a request the
    /// read-out withholds *Try again* for. Kept apart from `attempted`, which only places the check mark.
    Failed { attempted: Option<MediaId>, refused: Option<MediaId> },
}

impl LogicalState for VersionsArg {
    fn write(&self, c: &mut Canon) {
        c.u32(self.host.0).u32(u32::from(self.sid.raw())).str(&self.rk);
        for value in self.anchor {
            c.u32(value);
        }
        c.option(self.opener, |c, (entry, elem)| { c.u32(entry.0).u32(elem); });
    }
    fn probe(&self, out: &mut String) {
        out.push_str("versions_arg");
    }
}

/// **The host the panel stands on, chosen from where its list comes from** (as `preplay_panel::host_of`): the failure
/// screen's panel stands over the VIDEO PLANE, a card under or above its button; a Detail page's grows out of the hero
/// control that opened it, else hangs under or above its disc on the sheet ground.
fn host_of(arg: &VersionsArg) -> Host {
    match (arg.source, arg.opener) {
        (Source::Failed { .. }, _) => Host::Video { anchor: arg.anchor, placement: Placement::UNDER_OR_ABOVE_FIT, shade: Shade::Sheet, need_h: None },
        (Source::Page, Some(_)) => plx_ui::detail_layout::hero_host(arg.anchor),
        (Source::Page, None) => Host::Sheet { anchor: arg.anchor },
    }
}

/// A row's identity: its position in the list captured at open (a dynamic list's documented key).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct VersionRow(pub u16);

impl FormId for VersionRow {
    fn key(&self) -> RowKey {
        RowKey(u32::from(self.0))
    }
}

/// A page-less menu: the version list is its only page.
type VersionsMenu = Menu<Infallible, VersionRow, MediaId>;

pub struct VersionsPanelScreen {
    entry: EntryId,
    arg: VersionsArg,
    /// The versions the rows were built from, in the server's order.
    versions: Vec<MediaId>,
    /// The panel: the glass of the hero control it grows out of (`Host::Grows`) when the argument names that
    /// control, else a sheet under or above the disc; one root page, the shared version list.
    menu: VersionsMenu,
}

impl VersionsPanelScreen {
    /// `checked` is the version Play takes now: it wears the check mark and the list opens on it
    /// (else on the first row). `can_play` is whether the platform composed any playback driver
    /// (`ScreenInputs::can_play`): without one every row is disabled and says why, and presses
    /// nothing.
    pub fn new(entry: EntryId, arg: VersionsArg, versions: &[MediaVersion], checked: Option<MediaId>, can_play: bool) -> Self {
        // over the control when it morphs out of one; under the disc when there is room and above it when
        // there is not for a panel with no opener. Rows carry a sub-line: the track menu's size class.
        let host = host_of(&arg);
        let mut menu = Menu::new(host, Class::Picker, plx_ui::table_screen::BAND_BASE);
        let unavailable = (!can_play).then(plx_os::i18n::msg::browse_detail_no_playback);
        let refused = match arg.source {
            Source::Failed { refused, .. } => refused,
            Source::Page => None,
        };
        // It opens on the check mark, but never on a disabled row: then on the first row that presses.
        let opening = checked.filter(|id| Some(*id) != refused).and_then(|id| versions.iter().position(|v| v.id == id))
            .or_else(|| versions.iter().position(|v| Some(v.id) != refused))
            .unwrap_or(0);
        menu.step(
            Event::ReplaceRoot { slide: None },
            &mut |page| {
                page.is_none().then(|| {
                    let form = Form::new().section(plx_appkit::version_list::section(plx_appkit::version_list::title(), versions, VersionRow, |v| v.id, checked, refused, unavailable));
                    Page::root(form).initial(Some(VersionRow(opening as u16)))
                })
            },
        );
        Self { entry, arg, versions: versions.iter().map(|v| v.id).collect(), menu }
    }

    /// The panel's table and cursor.
    pub fn form(&self) -> &plx_ui::form::FormTable<VersionRow, MediaId, Infallible> {
        self.menu.form()
    }

    /// Where the panel stands (the menu's one placement rule for its host).
    pub fn frame(&self, measure: &dyn plx_machine::machine::Measure) -> Rect {
        self.menu.panel_rect(measure)
    }

    fn dismiss<H: AppLike>(&self, fx: &mut Effects<'_, H>) {
        fx.push(Fx::Nav(NavOp::Dismiss(self.entry)));
    }

    /// What the menu answered: a dismissal closes the panel, and a pick closes it and reports the row's version to
    /// the page, which selects it. OK on a DISABLED row (no playback driver composed) does nothing and the panel
    /// stays up, as on every disabled row (`ui/CLAUDE.md`: "OK does nothing"); BACK still closes it.
    fn run<H: AppLike>(&mut self, out: Out<MediaId>, fx: &mut Effects<'_, H>) {
        let media = match out {
            Out::Stay | Out::Moved | Out::Edge => return,
            Out::Picked(media) => Some(media),
            Out::Dismiss => None,
        };
        self.dismiss(fx);
        let Some(media) = media else { return };
        match self.arg.source {
            Source::Page => fx.push(Fx::Deliver(
                MachineId::Instance(self.arg.host),
                Delivery::Screen(ScreenEvent::App(AppMsg::SelectVersion(media))),
            )),
            // The failed item plays on that file through the same door every failure recovery takes.
            Source::Failed { .. } => fx.push(Fx::App(AppFx::Player(PlayerReq::Recover(plx_media::player::Recovery::PickVersion(media))))),
        }
    }
}

impl<H: AppLike<Memory = PageMemory>> Machine<H> for VersionsPanelScreen {
    type Ev = ScreenEvent<H>;
    fn step(&mut self, ev: &Self::Ev, cx: &Cx<'_, H>, fx: &mut Effects<'_, H>) -> Handled {
        // all of it the menu's (`Menu::on_screen`): BACK at its one page closes it, LEFT is inert here, never a close
        match self.menu.on_screen(ev, cx, self.entry, fx, &mut no_pages) {
            Some(glued) => {
                self.run(glued.out, fx);
                glued.handled
            }
            None => Handled::No,
        }
    }
}

impl plx_ui::menu::HasMenu for VersionsPanelScreen {
    type Page = Infallible;
    type Row = VersionRow;
    type Act = MediaId;
    fn menu(&self) -> &VersionsMenu {
        &self.menu
    }
    fn menu_entry(&self) -> EntryId {
        self.entry
    }
}

/// One column of `Bare` rows over the table's own cursor, *Also available*'s `Focusable`.
impl<H: AppLike<Memory = PageMemory> + MetadataLike> Focusable<H> for VersionsPanelScreen {
    fn groups(&self, cx: &Cx<'_, H>, out: &mut Vec<GroupSpec>) {
        self.menu.groups(GroupId(0), cx.measure, out);
    }
    fn group_of(&self, elem: &u32, _cx: &Cx<'_, H>) -> Option<GroupId> {
        self.menu.group_of(elem, GroupId(0))
    }
    fn neighbour(&self, key: FocusKey<u32>, dir: Dir, _cx: &Cx<'_, H>) -> Step<u32> {
        self.menu.neighbour(self.entry, key, dir)
    }
    fn place(&self, elem: &u32, cx: &Cx<'_, H>, _at: At) -> Option<Placed> {
        self.menu.place(elem, cx.measure)
    }
    fn reconcile(&self, want: FocusKey<u32>, _cx: &Cx<'_, H>) -> FocusKey<u32> {
        self.menu.reconcile(self.entry, want, GroupId(0))
    }
    fn seat(&self, _g: GroupId, _from: Placed, _cx: &Cx<'_, H>) -> FocusKey<u32> {
        self.menu.seat(self.entry)
    }

    /// **Row `n` is `row.<n>`**, the shared table convention
    /// ([`plx_ui::geom::table_a11y`]): `n` is the row's ordinal among the focusable rows (the same
    /// count `GroupSpec::len` states), so a separator or note never shifts an id. The label is the
    /// row's own text, for the state file only. The elements and the inverse are the shared ones.
    ///
    /// **A row selects for a title, so it names that title.** OK delivers `AppMsg::SelectVersion` to the page, which
    /// records the version the row names for the hero's title, the one its Play pill plays: the subject is the Play pill's own
    /// (`registry::versions_subject`: the page's loaded item, read from the metadata store the way the mount reads the
    /// versions), so `versions.row.<n>` needs a press that names the title. The key is derived from `cx`, not kept on the
    /// screen: it is no part of the screen's recorded state (`SHAPE`).
    fn a11y(&self, elem: &u32, cx: &Cx<'_, H>) -> Option<A11y<'_>> {
        let form = self.menu.form();
        let slot = form.index_of_key(RowKey(*elem))?;
        let row = plx_ui::geom::table_a11y(&form.table, Some(form), slot)?;
        Some(match crate::registry::versions_subject(cx, &self.arg) {
            Some(rk) => row.with_subject(rk),
            None => row,
        })
    }
    fn a11y_elems(&self, g: GroupId, _cx: &Cx<'_, H>, out: &mut Vec<u32>) {
        if g == GroupId(0) {
            let form = self.menu.form();
            out.extend(
                plx_ui::geom::table_elems(&form.table, Some(form))
                    .filter_map(|i| form.key_at(i))
                    .map(|k| k.0),
            );
        }
    }
    fn a11y_find(&self, path: &A11yPath, _cx: &Cx<'_, H>) -> Option<u32> {
        let form = self.menu.form();
        let slot = plx_ui::geom::table_a11y_find(&form.table, Some(form), path)?;
        form.key_at(slot).map(|k| k.0)
    }
}

impl LogicalState for VersionsPanelScreen {
    fn write(&self, c: &mut Canon) {
        self.arg.write(c);
        c.seq(self.versions.len());
        for id in &self.versions {
            c.u64(id.0 as u64);
        }
        let table = &self.menu.form().table;
        c.u32(table.sel as u32);
        table.write_motion(c);
    }
    fn probe(&self, out: &mut String) {
        out.push_str("versions");
    }
}

impl<H: AppLike<Memory = PageMemory> + MetadataLike> Screen<H> for VersionsPanelScreen {
    fn name(&self) -> &'static str {
        "versions"
    }
    /// The hero control the glass grew out of: the page leaves it out of its pass
    /// (`DrawFrame::lifted`) and the dispatcher's lift draws it above the scrim and the panel.
    fn opener(&self) -> Option<(EntryId, Option<FocusKey<u32>>)> {
        self.arg.opener.map(|(entry, elem)| (entry, Some(FocusKey { entry, elem })))
    }
    // The menu's own (`plx_ui::menu_screen_methods!`): the opener is CONSUMED by the morph (gone while the panel
    // stands, back as the droplet drains into it; whole for a panel with no morph), the hand-back, the panel's own
    // dim weight (a morphing panel's FOLLOWS the morph), the prewarm, the pointer hold and the draw.
    plx_ui::menu_screen_methods!(H, "dt.versions");
    fn state(&self) -> &dyn LogicalState {
        self
    }
    fn crumb(&self, _cx: &Cx<'_, H>) -> Option<Cow<'_, str>> {
        None
    }
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
    use plx_machine::machine::{Edge, Host, InputKind, Key};
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
        type Fx = crate::registry::AppFx;
        type Msg = AppMsg;
        type Elem = u32;
        type Views<'a> = ();
        type Init = Arg;
        type Memory = PageMemory;
    }

    thread_local! {
        // TEST ONLY: the page a panel reads its subject off, as `screens::detail::tests`'s `TEST_METADATA`.
        static TEST_METADATA: std::cell::UnsafeCell<plx_data::stores::metadata::MetadataStore> =
            std::cell::UnsafeCell::new(plx_data::stores::metadata::MetadataStore::default());
    }
    fn test_store() -> &'static mut plx_data::stores::metadata::MetadataStore {
        TEST_METADATA.with(|cell| unsafe { &mut *cell.get() })
    }
    impl MetadataLike for HostFixture {
        fn metadata<'a>(_cx: &Cx<'a, Self>) -> plx_data::metadata::MetadataView<'a> {
            test_store().view()
        }
    }
    /// Make `d` the page the metadata store holds (or none), and keep the shared store to this test.
    fn page(d: Option<plx_data::metadata::Detail>) -> plx_base::testlock::Serial {
        let guard = plx_base::testlock::serial();
        plx_data::metadata::set_current_for_test(test_store().state_mut(), d);
        guard
    }

    fn panel() -> VersionsPanelScreen {
        let anchor = [500.0f32, 600.0, 64.0, 64.0].map(f32::to_bits);
        VersionsPanelScreen::new(
            EntryId(3),
            VersionsArg { host: InstanceId(9), sid: ServerId::from_raw(1), rk: "42".into(), anchor, opener: None, source: crate::versions_panel::Source::Page },
            &plx_appkit::version_list::sample(),
            None,
            true,
        )
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

    /// The *Play Version* list over the sample versions, run through the id conformance
    /// (`AppArg::VersionsPanel`; the coverage table calls this).
    pub(crate) fn versions_panel_conforms() -> plx_ui::a11y::Namespaces {
        vec![plx_ui::a11y::conform::<HostFixture, _>(&panel(), &cx())]
    }

    /// Every version is a row with an id, `row.<n>` in the server's order, and the id finds it
    /// again.
    #[test]
    fn every_version_answers_an_id_in_the_servers_order() {
        let p = panel();
        let cx = cx();
        plx_ui::a11y::conform::<HostFixture, _>(&p, &cx);
        let n = plx_appkit::version_list::sample().len();
        assert!(n >= 2, "the sample has several versions");
        for i in 0..n {
            let path = plx_ui::a11y::A11yPath::parse_relative(&format!("row.{i}")).expect("a well-formed path");
            assert_eq!(Focusable::<HostFixture>::a11y_find(&p, &path, &cx), Some(i as u32), "row.{i} is version {i}");
        }
        let past = plx_ui::a11y::A11yPath::parse_relative(&format!("row.{n}")).expect("a well-formed path");
        assert_eq!(Focusable::<HostFixture>::a11y_find(&p, &past, &cx), None);
    }

    fn row_subject(p: &VersionsPanelScreen, row: u32) -> Option<String> {
        let cx = cx();
        Focusable::<HostFixture>::a11y(p, &row, &cx)?.subject.map(Cow::into_owned)
    }

    /// **A row plays, so it names the title it plays** (`versions.row.<n>` is in `SUBJECT_BEARING`): the subject is
    /// what the page's Play pill plays, the key `AppMsg::SelectVersion` acts on (`DetailScreen::select_version`), and not the
    /// route's. A movie page plays itself; a show page plays the episode its hero plays, so the route's show key
    /// would name the wrong title. With no page loaded (or another one) a row names none, and the guarded doors
    /// cannot press it.
    #[test]
    fn a_row_names_the_title_its_page_plays() {
        use plx_data::metadata::{Detail, Episode};
        let sid = ServerId::from_raw(1);
        let p = panel();
        {
            let _guard = page(Some(Detail { sid, rk: "42".into(), kind: "movie".into(), ..Default::default() }));
            for row in 0..plx_appkit::version_list::sample().len() as u32 {
                assert_eq!(row_subject(&p, row).as_deref(), Some("42"), "row {row} plays the movie");
            }
        }
        {
            let episode = Episode { rk: "e1".into(), index: 1, season: 1, ..Default::default() };
            let _guard = page(Some(Detail {
                sid, rk: "42".into(), kind: "show".into(), is_show: true, episodes: vec![episode].into(), ..Default::default()
            }));
            assert_eq!(row_subject(&p, 0).as_deref(), Some("e1"), "a show's rows play the hero's episode, not the show");
        }
        {
            let _guard = page(Some(Detail { sid, rk: "another".into(), kind: "movie".into(), ..Default::default() }));
            assert_eq!(row_subject(&p, 0), None, "another page is loaded: the panel names no title");
        }
        let _guard = page(None);
        assert_eq!(row_subject(&p, 0), None, "no page loaded");
    }

    /// **The standalone Select Version panel is a picker: HEADLINE bold rows** (one size class per glass;
    /// the same list pushed into an action menu takes the menu's compact class).
    #[test]
    fn the_panel_is_headline_bold() {
        let _guard = plx_base::testlock::serial();
        let p = morphing_panel(None);
        assert_eq!(p.form().table.label_style(), (plx_ui::theme::size::HEADLINE, true));
    }

    /// The version Play takes now wears the check mark and the list opens on it; with none named
    /// the list opens on its first row and no row is marked.
    #[test]
    fn the_taken_version_wears_the_check_and_the_list_opens_on_it() {
        let versions = plx_appkit::version_list::sample();
        let anchor = [500.0f32, 600.0, 64.0, 64.0].map(f32::to_bits);
        let arg = VersionsArg { host: InstanceId(9), sid: ServerId::from_raw(1), rk: "42".into(), anchor, opener: None, source: crate::versions_panel::Source::Page };
        for (checked, marked, opening) in [(None, None, 0), (Some(versions[0].id), Some(0), 0), (Some(versions[1].id), Some(1), 1)] {
            let p = VersionsPanelScreen::new(EntryId(3), arg.clone(), &versions, checked, true);
            let rows: Vec<_> = p.form().table.sections.iter().flat_map(|s| s.rows.iter()).filter(|r| !r.is_note()).collect();
            assert_eq!(rows.iter().position(|r| r.checked), marked, "{checked:?}");
            assert_eq!(p.form().selected_key().map(|k| k.0), Some(opening), "{checked:?}: opens on the marked row");
        }
        let gone = VersionsPanelScreen::new(EntryId(3), arg, &versions, Some(MediaId(99)), true);
        assert_eq!(gone.form().selected_key().map(|k| k.0), Some(0), "a version the list lacks opens on the first row");
    }

    /// OK on a row closes the panel and reports THAT version to the page that opened it.
    #[test]
    fn a_row_reports_its_version_to_the_host_page() {
        let mut p = panel();
        let mut out = Vec::new();
        let mut present = plx_machine::present::Present::new();
        let mut fx: Effects<'_, HostFixture> =
            Effects::new(&mut out, MachineId::Instance(InstanceId(4)), &mut present);
        <VersionsPanelScreen as Machine<HostFixture>>::step(&mut p, &ScreenEvent::Activate(1), &cx(), &mut fx);
        let fx: Vec<_> = out.into_iter().map(|e| e.fx).collect();
        assert!(matches!(fx.first(), Some(Fx::Nav(NavOp::Dismiss(EntryId(3))))), "the panel closes");
        let played = fx.iter().find_map(|f| match f {
            Fx::Deliver(MachineId::Instance(InstanceId(9)), Delivery::Screen(ScreenEvent::App(AppMsg::SelectVersion(m)))) => Some(*m),
            _ => None,
        });
        assert_eq!(played, Some(plx_appkit::version_list::sample()[1].id), "the host page is told which version");
    }

    /// **On the failure screen a pick asks the one recovery door** (`Recovery::PickVersion`) and tells no page: the panel
    /// stands over the video plane, the file the failed attempt asked for wears the check mark, and OK on a row closes
    /// the panel and asks to play the failed item on THAT version.
    #[test]
    fn on_the_failure_screen_a_row_asks_the_recovery_door_for_that_version() {
        use crate::registry::PlayerReq;
        use plx_media::player::Recovery;
        let sample = plx_appkit::version_list::sample();
        let anchor = [500.0f32, 600.0, 64.0, 64.0].map(f32::to_bits);
        let arg = VersionsArg {
            host: InstanceId(9), sid: ServerId::from_raw(1), rk: "42".into(), anchor, opener: None,
            source: Source::Failed { attempted: Some(sample[1].id), refused: None },
        };
        assert!(matches!(host_of(&arg), plx_ui::menu::Host::Video { .. }), "the read-out's panel stands over the video plane");
        let mut p = VersionsPanelScreen::new(EntryId(3), arg, &sample, Some(sample[1].id), true);
        let mut out = Vec::new();
        let mut present = plx_machine::present::Present::new();
        let mut fx: Effects<'_, HostFixture> = Effects::new(&mut out, MachineId::Instance(InstanceId(4)), &mut present);
        <VersionsPanelScreen as Machine<HostFixture>>::step(&mut p, &ScreenEvent::Activate(0), &cx(), &mut fx);
        let fx: Vec<_> = out.into_iter().map(|e| e.fx).collect();
        assert!(fx.iter().any(|f| matches!(f, Fx::Nav(NavOp::Dismiss(EntryId(3))))), "the panel closes");
        assert!(!fx.iter().any(|f| matches!(f, Fx::Deliver(..))), "no page is told: the failed item is not a Detail page's");
        let asked: Vec<_> = fx.iter().filter_map(|f| match f {
            Fx::App(AppFx::Player(PlayerReq::Recover(r))) => Some(r.clone()),
            _ => None,
        }).collect();
        assert_eq!(asked, [Recovery::PickVersion(sample[0].id)], "the door is asked to play that file");
    }

    /// **On a refusal the file that just failed is a disabled row**: choosing it would repeat the request the read-out
    /// withholds *Try again* for, so it wears the check mark and answers no press (and the panel stays up), while the other
    /// file asks the recovery door. It opens on the first row that presses, never on the disabled one, while the check mark
    /// stays on the failed file. On a stall (`refused: None`) every row presses and it opens on the check mark.
    #[test]
    fn on_a_refusal_the_failed_file_is_disabled_and_another_file_still_asks() {
        use crate::registry::PlayerReq;
        let sample = plx_appkit::version_list::sample();
        let anchor = [500.0f32, 600.0, 64.0, 64.0].map(f32::to_bits);
        let panel = |refused| VersionsPanelScreen::new(
            EntryId(3),
            VersionsArg {
                host: InstanceId(9), sid: ServerId::from_raw(1), rk: "42".into(), anchor, opener: None,
                source: Source::Failed { attempted: Some(sample[1].id), refused },
            },
            &sample, Some(sample[1].id), true);
        let drive = |p: &mut VersionsPanelScreen, key: u32| {
            let mut out = Vec::new();
            let mut present = plx_machine::present::Present::new();
            let mut fx: Effects<'_, HostFixture> = Effects::new(&mut out, MachineId::Instance(InstanceId(4)), &mut present);
            <VersionsPanelScreen as Machine<HostFixture>>::step(p, &ScreenEvent::Activate(key), &cx(), &mut fx);
            out.into_iter().map(|e| e.fx).collect::<Vec<_>>()
        };
        let asked = |fx: &[Fx<HostFixture>]| fx.iter().any(|f| matches!(f, Fx::App(AppFx::Player(PlayerReq::Recover(_)))));
        let mut refused = panel(Some(sample[1].id));
        let rows: Vec<_> = refused.form().table.sections.iter().flat_map(|s| s.rows.iter()).filter(|r| !r.is_note()).collect();
        assert_eq!((rows[0].disabled, rows[1].disabled), (false, true), "only the failed file's row is disabled");
        assert_eq!(refused.form().selected_id(), Some(&VersionRow(0)), "it opens on the first row that presses, not the disabled one");
        assert!(drive(&mut refused, 1).is_empty(), "OK on it does nothing and the panel stays up");
        assert!(asked(&drive(&mut refused, 0)), "the other file asks the recovery door");
        let mut stall = panel(None);
        assert_eq!(stall.form().selected_id(), Some(&VersionRow(1)), "with nothing refused it opens on the check mark");
        assert!(asked(&drive(&mut stall, 1)), "on a stall the same file is a retry and presses");
    }

    /// A platform that composed no playback driver opens the same list with every row disabled and the
    /// one reason noted under them (`browse.detail.no_playback`, the Detail facts line's own word): OK
    /// on a row does nothing, as on every disabled row (`ui/CLAUDE.md`), so the panel stays up and
    /// nothing is asked of a driver that is not there, while BACK still closes it.
    #[test]
    fn with_no_playback_driver_every_row_is_disabled_and_reports_nothing() {
        use plx_machine::machine::{InputEvent, Source, Tick};
        let _lang = plx_os::i18n::language_on_this_thread_for_test(plx_os::i18n::Preference::En);
        let why = plx_os::i18n::msg::browse_detail_no_playback();
        let anchor = [500.0f32, 600.0, 64.0, 64.0].map(f32::to_bits);
        let arg = VersionsArg { host: InstanceId(9), sid: ServerId::from_raw(1), rk: "42".into(), anchor, opener: None, source: crate::versions_panel::Source::Page };
        // What one event does to a panel: the effects it pushes.
        let drive = |p: &mut VersionsPanelScreen, ev: ScreenEvent<HostFixture>| {
            let mut out = Vec::new();
            let mut present = plx_machine::present::Present::new();
            let mut fx: Effects<'_, HostFixture> =
                Effects::new(&mut out, MachineId::Instance(InstanceId(4)), &mut present);
            Machine::<HostFixture>::step(p, &ev, &cx(), &mut fx);
            out.into_iter().map(|e| e.fx).collect::<Vec<_>>()
        };
        let closes = |fx: &[Fx<HostFixture>]| fx.iter().any(|f| matches!(f, Fx::Nav(NavOp::Dismiss(EntryId(3)))));
        let reports = |fx: &[Fx<HostFixture>]| fx.iter().any(|f| matches!(f,
            Fx::Deliver(_, Delivery::Screen(ScreenEvent::App(AppMsg::SelectVersion(_))))));
        for (can_play, disabled) in [(true, false), (false, true)] {
            let mut p = VersionsPanelScreen::new(EntryId(3), arg.clone(), &plx_appkit::version_list::sample(), None, can_play);
            let rows: Vec<_> = p.form().table.sections.iter().flat_map(|s| s.rows.iter()).collect();
            let versions: Vec<_> = rows.iter().filter(|row| !row.is_note()).collect();
            let notes: Vec<_> = rows.iter().filter(|row| row.is_note()).collect();
            assert_eq!(versions.len(), 2);
            for row in versions {
                assert_eq!(row.disabled, disabled, "can_play={can_play}");
            }
            assert_eq!(notes.iter().map(|row| row.label.as_str()).collect::<Vec<_>>(),
                disabled.then_some(why).into_iter().collect::<Vec<_>>(), "can_play={can_play}: the one note says why");
            // OK, through the handler's own arm (`Activate`, what a click and the engine's OK deliver).
            let ok = drive(&mut p, ScreenEvent::Activate(1));
            assert_eq!((closes(&ok), reports(&ok)), (!disabled, !disabled),
                "can_play={can_play}: OK on a row closes the panel and reports it only while the row is enabled");
            if disabled {
                assert!(ok.is_empty(), "OK on a disabled row does nothing at all, got {} effects", ok.len());
            }
            // BACK closes the panel whichever state it is in, and reports nothing.
            let back = drive(&mut p, ScreenEvent::Input(InputEvent {
                at: Tick::default(),
                source: Source::Script,
                kind: InputKind::Key { key: Key::Back, sym: 0, wcode: 0, edge: Edge::Down, at_edge: false },
            }));
            assert!(closes(&back) && !reports(&back), "can_play={can_play}: BACK closes the panel and plays nothing");
        }
    }

    /// **LEFT and RIGHT at a one-page menu are inert**, never a close: LEFT has no page to pop and RIGHT no
    /// Nav row to enter. They are re-delivered to the screen (`EdgeRule::Screen`), so the screen must
    /// answer them (an unhandled one is a dead press), and only the down edge of BACK closes.
    #[test]
    fn left_and_right_never_close_the_panel_and_only_back_down_does() {
        use plx_machine::machine::{InputEvent, Source, Tick};
        let key = |key: Key, edge: Edge| {
            ScreenEvent::<HostFixture>::Input(InputEvent {
                at: Tick::default(),
                source: Source::Script,
                kind: InputKind::Key { key, sym: 0, wcode: 0, edge, at_edge: true },
            })
        };
        let drive = |ev: ScreenEvent<HostFixture>| {
            let mut p = panel();
            let mut out = Vec::new();
            let mut present = plx_machine::present::Present::new();
            let mut fx: Effects<'_, HostFixture> = Effects::new(&mut out, MachineId::Instance(InstanceId(4)), &mut present);
            let handled = Machine::<HostFixture>::step(&mut p, &ev, &cx(), &mut fx);
            (handled, out.len())
        };
        for k in [Key::Left, Key::Right] {
            for edge in [Edge::Down, Edge::Up] {
                assert_eq!(drive(key(k, edge)), (Handled::Yes, 0), "{k:?} {edge:?}: answered, closes nothing");
            }
        }
        assert_eq!(drive(key(Key::Back, Edge::Down)), (Handled::Yes, 1), "BACK down closes");
        assert_eq!(drive(key(Key::Back, Edge::Up)), (Handled::No, 0), "BACK up is not the menu's");
    }

    // ---- the hero control's morph ------------------------------------------------------------

    /// The panel over a hero control at `(500, 700)`: the page's entry 1 and the control's elem 9.
    fn morphing_panel(opener: Option<(EntryId, u32)>) -> VersionsPanelScreen {
        let anchor = [500.0f32, 700.0, 64.0, 64.0].map(f32::to_bits);
        VersionsPanelScreen::new(
            EntryId(3),
            VersionsArg { host: InstanceId(9), sid: ServerId::from_raw(1), rk: "42".into(), anchor, opener, source: crate::versions_panel::Source::Page },
            &plx_appkit::version_list::sample(),
            None,
            true,
        )
    }

    fn step(p: &mut VersionsPanelScreen, ev: ScreenEvent<HostFixture>) {
        let mut out = Vec::new();
        let mut present = plx_machine::present::Present::new();
        let mut fx: Effects<'_, HostFixture> = Effects::new(&mut out, MachineId::Instance(InstanceId(4)), &mut present);
        Machine::<HostFixture>::step(p, &ev, &cx(), &mut fx);
    }

    fn tick(p: &mut VersionsPanelScreen, ms: &mut u32, n: usize) {
        for _ in 0..n {
            *ms += 16;
            step(p, ScreenEvent::Tick(plx_machine::machine::Tick { ms: *ms, dt_us: 16_667 }));
        }
    }

    /// The glass this frame: the panel ground (a `rect_rimmed`, tag 8, on a host with no frozen field).
    fn glass(p: &mut VersionsPanelScreen) -> Rect {
        plx_ui::draw_census::capture(|| {
            let cx = cx();
            let mut f = DrawFrame::new(&cx, plx_ui::Painter::recording());
            Screen::<HostFixture>::draw(p, &mut f);
        })
        .into_iter()
        .find_map(|(tag, r)| (tag == 8).then_some(r))
        .unwrap_or_default()
    }

    /// **Select Version GROWS out of the hero control that opened it**, as every header menu does
    /// (`Popover::morphing` + `open_from`): the control is the surface's `opener` (the page leaves it
    /// out of its pass; the lift draws it at the morph's `opener_alpha`), the panel covers it, three
    /// frames in the glass is still the control's size or a little over, the settled panel is exactly
    /// `frame()`, and BACK (or a pick) runs the morph home into the control.
    #[test]
    fn the_panel_grows_out_of_its_control_settles_on_its_frame_and_runs_home_on_back() {
        let _guard = plx_base::testlock::serial();
        let mut p = morphing_panel(Some((EntryId(1), 9)));
        assert_eq!(
            Screen::<HostFixture>::opener(&p),
            Some((EntryId(1), Some(FocusKey { entry: EntryId(1), elem: 9 }))),
            "the opener is the hero control, on the page"
        );
        step(&mut p, ScreenEvent::Mount);
        let settled = p.frame(&plx_ui::fixture::FixtureMeasure);
        let pill = plx_ui::detail_layout::hero_pill(p.arg.anchor);
        assert!(
            settled.x <= pill.x && settled.y <= pill.y && settled.x + settled.w >= pill.x + pill.w && settled.y + settled.h >= pill.y + pill.h,
            "the settled panel {settled:?} covers the control {pill:?}, which the morph consumes"
        );
        assert_eq!(Screen::<HostFixture>::opener_alpha(&p), 1.0, "frame 0 is the control whole");
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
            "the settled panel is the frame {settled:?}, drew {end:?}"
        );
        assert_eq!(Screen::<HostFixture>::opener_alpha(&p), 0.0, "the control stays consumed while the panel stands");
        step(&mut p, ScreenEvent::Closing);
        tick(&mut p, &mut ms, 3);
        assert!(glass(&mut p).w < settled.w - 20.0, "three frames into the close the glass is narrowing into the control");
        assert!(!Screen::<HostFixture>::handed_back(&p));
        tick(&mut p, &mut ms, 60);
        assert!(Screen::<HostFixture>::handed_back(&p), "the droplet has drained into the control");
        assert_eq!(Screen::<HostFixture>::opener_alpha(&p), 1.0);
    }

    /// A panel with no opener (every test rig, and any host that names none) draws as it always did:
    /// under the disc, no morph, the opener whole, no handback of its own.
    #[test]
    fn a_panel_with_no_opener_keeps_the_fade() {
        let _guard = plx_base::testlock::serial();
        let mut p = morphing_panel(None);
        step(&mut p, ScreenEvent::Mount);
        step(&mut p, ScreenEvent::Closing);
        assert_eq!(Screen::<HostFixture>::opener(&p), None);
        assert_eq!(Screen::<HostFixture>::opener_alpha(&p), 1.0);
        assert!(!Screen::<HostFixture>::handed_back(&p));
        assert!(Screen::<HostFixture>::scrim(&p).ramp.is_none());
        let [x, y, w, h] = [500.0f32, 700.0, 64.0, 64.0];
        let under = plx_ui::popover::under_or_above(Rect::new(x, y, w, h), p.form().table.measured_width(&plx_ui::fixture::FixtureMeasure), p.form().table.measured_height());
        assert_eq!(p.frame(&plx_ui::fixture::FixtureMeasure), under);
    }
}
