//! The What's on page (Live TV's landing page, the handoff's `SPEC-1` / `SPEC-2`): the profile's Recent
//! channels on top, the programmes airing now below, and the channels with neither under them: three adjacent
//! shared card shelves over the store's one lineup. A Recent tile, a programme poster and a Channels tile all ask
//! the content owner to play their channel.
//!
//! **On now has one fixed order**: the most recent start first, a tie in lineup order. **Recent never
//! reorders** (the store keeps it, newest first) and is hidden while it is empty. **Every channel stays
//! reachable**: a third shelf, the page's earlier "Channels" shelf (the lineup's own tile and caption),
//! holds the lineup channels with no programme on that are not in Recent, under the other rows, and is
//! hidden while that set is empty. It is the interim path for a channel without guide data (the Pluto
//! channels of a server that has none) until the Guide and Browse pages replace it; with nothing recent
//! and nothing on it is the page's only row. A lineup with no channel at all is the store's empty read-out.
//! Which rows the page shows is decided once, by [`Mode`] in `Page::sync`, and matched everywhere after.
//! The page reads the frame's wall time ([`LiveTvLike::wall`]) for its captions and progress bars, and asks
//! for a frame when the words of any airing change.
use plx_data::livetv::{Channel, LiveTvStatus, LiveTvView};
use plx_data::stores::{StoreCmd, StoreId};
use plx_data::stores::livetv::LiveTvCmd;
use plx_machine::machine::{Canon, Cx, Delivery, Edge, Effects, Enter, EntryId, FocusKey, FocusTarget,
    Fx, GroupId, Handled, InputEvent, InputKind, Key, LogicalState, Machine, Tick};
use plx_machine::present::Provenance;
use plx_ui::cards::{heading_ink_drop, CardEvent, CardSource, Kind, RowStyle, SectionSpec, Stack, StackEvent, StackPage, TileLabel};
use plx_ui::consts::*;
use plx_ui::page_header::{HeaderItem, HeaderRow, PageHeader, PageHeaderSpec, SharedRow};
use plx_ui::page_regions::{Arrival, BackStep, PageRegions};
use plx_ui::screen::{AxisMask, DrawFrame, EdgeRule, ElemKind, FocusSource, Focusable, GroupKind,
    GroupSpec, HitSource, Link, RenderStrategy, Screen, ScreenEvent, Seat};
use plx_ui::theme;
use plx_ui::fmt::{self, Airing, AiringPhase, Wall};
use plx_ui::tile::Resume;
use plx_ui::widgets::{AmbientWash, Art, PageGround, StatusKind, StatusOverlay};
use plx_ui::{Env, Rect};
use crate::registry::{captions, tile_facts, AppFx, ContentReq, HomeReq, HomeTab, LiveTvLike, PlayIntent};

#[cfg(test)]
pub(crate) mod cards_harness;

const FIRST_CARD_ELEM: u32 = 0x1000;
const RETRY_ELEM: u32 = 1;
const RECENT_GROUP: GroupId = GroupId(0);
const NOW_GROUP: GroupId = GroupId(1);
const HEAD_GROUP: GroupId = GroupId(2);
const STATUS_GROUP: GroupId = GroupId(3);
const CHANNELS_GROUP: GroupId = GroupId(4);
/// A shelf's heading to its tiles as the handoff has it, from the top of the heading's line box (a Home
/// shelf keeps `TITLE_DY + CARD_DY`, 60). The page's shelves draw their heading at the cap-top, which is
/// [`heading_ink_drop`] below that line box's top, so each gives its heading that much less.
const BOARD_HEADING: f32 = 56.0;
/// The air from the bottom of a shelf's tiles to the heading of the shelf under it at rest (the handoff's
/// 48, to the line box's top).
const BOARD_GAP_AT_REST: f32 = 48.0;
/// Recent's tiles: a logo plate, 250 x 140 on the poster pitch, so a poster stands under each tile.
const RECENT_STYLE: RowStyle = RowStyle::CHANNEL;
/// On now's posters and the Channels shelf's: the Home poster tile, with art that is not poster-shaped
/// contained on its own blurred copy (a logo with transparency: on the Recent plates' flat ground)
/// rather than cover-cropped (`RowStyle::PROGRAMME`).
const NOW_STYLE: RowStyle = RowStyle::PROGRAMME;
const STATUS_FRAME: Rect = Rect { x: MARGIN_X, y: plx_ui::page_header::GRID_TOP, w: SCR_W - 2.0 * MARGIN_X, h: SCR_H - plx_ui::page_header::GRID_TOP - MARGIN_Y };

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sec { Head, Recent, Now, Channels, Status }

impl Sec {
    /// The focus group the section's cards (or its retry read-out) stand in.
    const fn group(self) -> GroupId {
        match self {
            Sec::Head => HEAD_GROUP,
            Sec::Recent => RECENT_GROUP,
            Sec::Now => NOW_GROUP,
            Sec::Channels => CHANNELS_GROUP,
            Sec::Status => STATUS_GROUP,
        }
    }
}

/// **Which rows the page shows**, decided once per store revision ([`Page::sync`], the one place the store's
/// status is read) and matched everywhere after: the sections, the focus regions, the air under each shelf,
/// the cards, the headings and the status read-out. The page keeps no status beside it. A row has its own [`Sec`], and so its own shelf body and row style, in every
/// mode; the modes only say which of the rows are present. The rows come top to bottom: Recent, On now,
/// Channels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    /// The store is loading: no row, the working read-out.
    Loading,
    /// The lineup is empty: no row, the store's empty read-out.
    Empty,
    /// The store failed: the retry read-out.
    Failed,
    /// Recent tiles over the programmes on now.
    RecentNow,
    /// Recent tiles alone: no programme on, and every other channel is in Recent.
    RecentOnly,
    /// The programmes on now alone.
    NowOnly,
    /// Recent tiles over the lineup channels that have no programme on.
    RecentChannels,
    /// Recent tiles, the programmes on now, then the lineup channels that have none.
    RecentNowChannels,
    /// The programmes on now, then the lineup channels that have none.
    NowChannels,
    /// Nothing recent and nothing on: the lineup as its Channels shelf, the page's only row.
    ChannelsOnly,
}

impl Mode {
    /// The mode of a store that is `Ready` with a lineup, from which rows have channels.
    fn of(recent: bool, now: bool, channels: bool) -> Self {
        match (recent, now, channels) {
            (true, true, true) => Mode::RecentNowChannels,
            (true, true, false) => Mode::RecentNow,
            (true, false, true) => Mode::RecentChannels,
            (true, false, false) => Mode::RecentOnly,
            (false, true, true) => Mode::NowChannels,
            (false, true, false) => Mode::NowOnly,
            (false, false, true) => Mode::ChannelsOnly,
            // every channel of a lineup is on now, in Recent or in Channels: no channel, no row
            (false, false, false) => Mode::Empty,
        }
    }

    /// The sections that take room, top to bottom.
    fn rows(self) -> &'static [Sec] {
        match self {
            Mode::Loading | Mode::Empty => &[],
            Mode::Failed => &[Sec::Status],
            Mode::RecentNow => &[Sec::Recent, Sec::Now],
            Mode::RecentOnly => &[Sec::Recent],
            Mode::NowOnly => &[Sec::Now],
            Mode::RecentChannels => &[Sec::Recent, Sec::Channels],
            Mode::RecentNowChannels => &[Sec::Recent, Sec::Now, Sec::Channels],
            Mode::NowChannels => &[Sec::Now, Sec::Channels],
            Mode::ChannelsOnly => &[Sec::Channels],
        }
    }

    fn has(self, k: Sec) -> bool {
        self.rows().contains(&k)
    }

    /// Whether the page is a status read-out (the working, empty and failed ones) rather than rows.
    fn reads_out(self) -> bool {
        matches!(self, Mode::Loading | Mode::Empty | Mode::Failed)
    }

    /// The first row: where the arrival is seated.
    fn head(self) -> Option<Sec> {
        self.rows().first().copied()
    }

    /// Is another shelf drawn under row `k`?
    fn shelf_under(self, k: Sec) -> bool {
        self.rows().iter().position(|&r| r == k).is_some_and(|i| i + 1 < self.rows().len())
    }
}

struct Page {
    entry: EntryId,
    /// The lineup, one `(server, key)` per channel in the store's order (the cards' stable identities).
    channels: Vec<(plx_plex::plex::ServerId, String)>,
    /// The channels with a programme on, as indexes into the lineup, in On now's one order: the most recent
    /// start first, a tie in lineup order.
    airings: Vec<usize>,
    /// Recent's channels, as indexes into the lineup, in the store's order (newest first).
    recent: Vec<usize>,
    /// The lineup channels with no programme on that are not in Recent, as indexes into the lineup, in the
    /// lineup's order: the Channels shelf, which keeps a channel without guide data reachable.
    rest: Vec<usize>,
    /// The rows the page shows ([`Page::sync`] decides it from the store's status and the three lists).
    mode: Mode,
    /// The focus groups of the rows, top to bottom, from [`Mode::rows`].
    content: Vec<GroupId>,
    synced: Option<u32>,
    /// What the airings' words and bars were last drawn from ([`Page::clock_stamp`]); not state, a
    /// frame's reason.
    clock: u64,
    header_capsules: plx_ui::widgets::TabStrip,
    header_pop: plx_ui::widgets::CtlPop<1>,
    handoff: plx_ui::scroll_chrome::Handoff,
    header_rows: SharedRow<HeaderRow>,
}

impl Page {
    fn sync(&mut self, view: LiveTvView<'_>) {
        if self.synced == Some(view.revision()) { return; }
        self.synced = Some(view.revision());
        self.channels = view.tiles.iter().map(|m| (m.sid, m.rk.clone())).collect();
        self.airings = on_now_order(view.channels);
        // An empty Recent is a row to hide, whatever else the page holds.
        self.recent = view.recent.iter().filter_map(|r| lineup_index(view.channels, r)).collect();
        // Every channel stays reachable: one with no programme on that is not in Recent has no tile above, so
        // it is in the Channels shelf.
        self.rest = (0..view.channels.len()).filter(|i| view.channels[*i].now.is_empty() && !self.recent.contains(i)).collect();
        self.mode = match view.status {
            LiveTvStatus::Loading => Mode::Loading,
            LiveTvStatus::Empty => Mode::Empty,
            LiveTvStatus::Failed => Mode::Failed,
            LiveTvStatus::Ready if !view.channels.is_empty() => Mode::of(!self.recent.is_empty(), !self.airings.is_empty(), !self.rest.is_empty()),
            LiveTvStatus::Ready => Mode::Empty,
        };
        self.content = self.mode.rows().iter().map(|k| k.group()).collect();
    }

    /// One number that moves exactly when the words or the bar of any airing the page shows does
    /// ([`AiringPhase::stamp`]): what the page asks a frame for. `0` without a wall time.
    fn clock_stamp(&self, view: LiveTvView<'_>, wall: Option<Wall>) -> u64 {
        let Some(wall) = wall else { return 0 };
        self.recent.iter().chain(&self.airings).filter_map(|&i| view.channels.get(i))
            .fold(0xcbf2_9ce4_8422_2325_u64, |acc, c| {
                (acc ^ fmt::phase(c.begins_at, c.ends_at, wall.unix_s()).stamp()).wrapping_mul(0x0100_0000_01b3)
            })
    }

    fn regions(&self) -> PageRegions<'_> {
        PageRegions { header: None, content: &self.content, rail: None, back_walks_header: false, arrival: Arrival::Content, pending: None }
    }

    fn header_row(&self, measure: &dyn plx_machine::machine::Measure) -> std::rc::Rc<HeaderRow> {
        let key = std::ptr::from_ref(plx_os::i18n::current()) as usize as u64;
        self.header_rows.get(key, || {
            let left = vec![HeaderItem { label: plx_os::i18n::msg::browse_livetv_whats_on().to_owned(), title: true, ..Default::default() }];
            let page = PageHeader::new(PageHeaderSpec { left, ..Default::default() }, &[], measure);
            let layout = page.layout(SCR_W, measure);
            HeaderRow::new(page, layout, Vec::new())
        })
    }

    fn status_overlay(&self, tick: u32) -> StatusOverlay<'static> {
        match self.mode {
            // placeholder-exempt: built here, counted by StatusOverlay's Working draw.
            Mode::Loading => StatusOverlay::new(STATUS_FRAME, plx_os::i18n::msg::browse_livetv_loading_c(), StatusKind::Working).phase(tick),
            Mode::Empty => StatusOverlay::new(STATUS_FRAME, plx_os::i18n::msg::browse_livetv_empty_c(), StatusKind::Empty),
            Mode::Failed => StatusOverlay::new(Rect::FULL, plx_os::i18n::msg::browse_home_failed_c(), StatusKind::Failed)
                .page(plx_ui::icons::Icon::ServerBadgeMinus).action(plx_os::i18n::msg::browse_action_retry_c()),
            // rows are showing: nothing to read out (never drawn)
            _ => StatusOverlay::new(STATUS_FRAME, c"", StatusKind::Empty),
        }
    }
}

/// On now's one order, as indexes into `channels` (the lineup, in number order): the channels with a
/// programme, the most recent start first; a tie keeps the lineup's order (the sort is stable).
fn on_now_order(channels: &[Channel]) -> Vec<usize> {
    let mut order: Vec<usize> = channels.iter().enumerate().filter_map(|(i, c)| (!c.now.is_empty()).then_some(i)).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(channels[i].begins_at));
    order
}

/// Where a Recent channel stands in the lineup: a channel is its server, DVR and key, never its number.
fn lineup_index(channels: &[Channel], of: &Channel) -> Option<usize> {
    channels.iter().position(|c| c.sid == of.sid && c.dvr == of.dvr && c.channel == of.channel)
}

/// Which shelf a [`Cards`] serves: it decides the cards' art, words and element ids.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Row {
    Recent,
    Now,
    /// The page's earlier "Channels" shelf: the lineup channels with no programme on that are not in Recent.
    Channels,
}

struct Cards<'a> { view: LiveTvView<'a>, order: &'a [usize], row: Row, wall: Option<Wall> }
impl Cards<'_> {
    /// The lineup index of card `i`, in the shelf's own order.
    fn index(&self, i: usize) -> Option<usize> {
        self.order.get(i).copied()
    }
    fn channel(&self, i: usize) -> Option<(usize, &Channel)> {
        let index = self.index(i)?;
        Some((index, self.view.channels.get(index)?))
    }
    /// The amber bar: how much of the programme has aired. `Resume`'s own rule (present exactly while
    /// in progress) is the airing's phase.
    fn resume(&self, channel: &Channel) -> Option<Resume> {
        let w = self.wall?;
        if !matches!(fmt::phase(channel.begins_at, channel.ends_at, w.unix_s()), AiringPhase::JustStarted { .. } | AiringPhase::Running { .. }) {
            return None;
        }
        Some(Resume { frac: fmt::frac(channel.begins_at, channel.ends_at, w)?, left_ms: (channel.ends_at - w.unix_s()).max(0) * 1_000 })
    }
    /// The low bit of a card's element id: Recent's tiles and the Channels shelf's are even (no channel is in
    /// both), On now's posters odd.
    fn parity(&self) -> u32 { u32::from(self.row == Row::Now) }
}
impl<H: LiveTvLike> CardSource<H> for Cards<'_> {
    fn len(&self) -> usize { self.order.len() }
    fn elem(&self, i: usize) -> u32 { FIRST_CARD_ELEM + self.index(i).unwrap_or(0) as u32 * 2 + self.parity() }
    fn index_of(&self, e: &u32) -> Option<usize> {
        let offset = e.checked_sub(FIRST_CARD_ELEM)?;
        if offset % 2 != self.parity() { return None; }
        let i = offset as usize / 2;
        self.order.iter().position(|&n| n == i)
    }
    fn art(&self, i: usize) -> Art<'_> {
        let Some((index, channel)) = self.channel(i) else { return Art::Poster(None) };
        if self.row == Row::Channels { return Art::Poster(self.view.tiles.get(index).map(|tile| tile_facts::of(tile))); }
        if self.row == Row::Recent {
            return Art::Channel { sid: channel.sid.raw(), logo: &channel.thumb, number: &channel.vcn, name: &channel.name };
        }
        Art::Poster(self.view.tiles.get(index).map(|tile| {
            let mut facts = tile_facts::of(tile);
            facts.owner = channel as *const _ as usize;
            facts.title = &channel.now;
            facts.show_title = "";
            facts.thumb = &channel.now_thumb;
            facts.still = "";
            facts.art = "";
            facts.resume = self.resume(channel);
            facts
        }))
    }
    /// The poster's bar; a channel tile has none.
    fn progress(&self, i: usize) -> Option<f32> {
        if self.row != Row::Now { return None; }
        self.channel(i).and_then(|(_, c)| self.resume(c)).map(|r| r.frac)
    }
    fn label(&self, i: usize) -> TileLabel {
        let Some((_, channel)) = self.channel(i) else { return TileLabel::default() };
        use plx_os::i18n::msg;
        match self.row {
            Row::Channels => match self.view.tiles.get(self.channel(i).map_or(usize::MAX, |(index, _)| index)) {
                Some(item) => captions::shelf_label(&captions::live_tv_policy(), item, false),
                None => TileLabel::default(),
            },
            // "On now: <programme>" over when it started and how long is left; a channel with no
            // programme is its name alone.
            Row::Recent if channel.now.is_empty() => captions::live_label(channel.name.clone(), None),
            Row::Recent => captions::live_label(msg::browse_livetv_recent_on_now(&channel.now),
                self.wall.and_then(|w| fmt::airing_line(Airing::Tile, channel.begins_at, channel.ends_at, w))),
            // The programme over its channel, number first ("Ch 2009 · Station · 42m left").
            Row::Now => {
                let identity = if channel.vcn.is_empty() { channel.name.clone() } else { msg::browse_livetv_channel(&channel.name, &channel.vcn) };
                let line = match self.wall.and_then(|w| fmt::left(channel.begins_at, channel.ends_at, w)) {
                    Some(left) => msg::core_airing_pair(&identity, &left),
                    None => identity,
                };
                captions::live_label(channel.now.clone(), Some(line))
            }
        }
    }

    fn subject(&self, i: usize) -> Option<std::borrow::Cow<'_, str>> {
        use std::fmt::Write as _;
        let tile = self.view.tiles.get(self.channel(i)?.0)?;
        // Two servers may advertise the same DVR and channel. A press names the exact source.
        // Hex preserves that key without punctuation the remote subject grammar rejects.
        let mut subject = format!("live-{}-", tile.sid.raw());
        for byte in tile.rk.bytes() {
            write!(subject, "{byte:02x}").expect("writing to a String succeeds");
        }
        Some(subject.into())
    }
}

impl<H: LiveTvLike> StackPage<H> for Page {
    type Key = Sec;
    type Cards<'a> = Cards<'a>;
    fn revision(&self, cx: &Cx<'_, H>) -> u64 { u64::from(H::livetv(cx).revision()) }
    fn sections(&self, cx: &Cx<'_, H>, out: &mut Vec<SectionSpec<Sec>>) {
        let heading = BOARD_HEADING - heading_ink_drop(cx.measure);
        out.push(SectionSpec::new(Sec::Head, Kind::Custom { height: plx_ui::page_header::GRID_TOP + heading_ink_drop(cx.measure), focusable: false }, Sec::Head.group()));
        // Each row is its own section, with its own style: a shelf keeps its style from the body it was made
        // with, so a key never stands for two row styles. The head of the content is seated by its own rule
        // (`CONTENT_HEAD_SEAT`): the first row there is.
        for (k, style, name) in [(Sec::Recent, &RECENT_STYLE, "recent"), (Sec::Now, &NOW_STYLE, "on_now"), (Sec::Channels, &NOW_STYLE, "channels")] {
            let spec = SectionSpec::new(k, Kind::Shelf { style, heading }, k.group()).named(name);
            out.push(if self.mode.head() == Some(k) { spec.seated(plx_ui::page_regions::CONTENT_HEAD_SEAT) } else { spec });
        }
        let retry = (self.mode == Mode::Failed).then(|| self.status_overlay(cx.tick.ms).action_frame_measured(cx.measure)).flatten();
        out.push(SectionSpec::new(Sec::Status, Kind::Overlay { rect: retry.unwrap_or(STATUS_FRAME), focusable: retry.is_some() }, Sec::Status.group()).named("status"));
    }
    /// A shelf with another under it keeps the handoff's 48 from its tiles to that shelf's heading at rest
    /// (the label band's collapsed 18, then this) and the label band's own room (the 552 under Recent) with a
    /// tile focused ([`shelf_foot_open`](Self::shelf_foot_open)); the last shelf keeps nothing.
    fn shelf_foot(&self, cx: &Cx<'_, H>, k: Sec) -> f32 {
        if self.mode.shelf_under(k) { BOARD_GAP_AT_REST + heading_ink_drop(cx.measure) - plx_ui::cards::LABEL_BAND_COLLAPSED } else { 0.0 }
    }
    fn shelf_foot_open(&self, cx: &Cx<'_, H>, k: Sec) -> f32 {
        if self.mode.shelf_under(k) { UNDER_LABEL_AIR + heading_ink_drop(cx.measure) } else { 0.0 }
    }
    /// The air under a shelf is the room of the heading of the shelf below, which that shelf's own reveal brings
    /// on: focusing a shelf scrolls for its tiles and open label band only.
    fn reveal_foot(&self, _k: Sec) -> bool { false }
    fn fallback(&self, _cx: &Cx<'_, H>, out: &mut Vec<Sec>) { out.extend([Sec::Recent, Sec::Now, Sec::Channels, Sec::Status, Sec::Head]); }
    fn cards<'a>(&'a self, cx: &'a Cx<'_, H>, k: Sec) -> Option<Cards<'a>> {
        let (order, row) = match k { Sec::Recent => (&self.recent, Row::Recent), Sec::Now => (&self.airings, Row::Now), Sec::Channels => (&self.rest, Row::Channels), _ => return None };
        self.mode.has(k).then(|| Cards { view: H::livetv(cx), order, row, wall: H::wall(cx) })
    }
    fn card_has_menu(&self, _cx: &Cx<'_, H>, _k: Sec, _elem: &u32) -> bool { false }
    fn caption_policy(&self, _cx: &Cx<'_, H>, k: Sec) -> Option<plx_ui::caption_policy::CaptionPolicy> {
        matches!(k, Sec::Recent | Sec::Now | Sec::Channels).then(captions::live_tv_policy)
    }
    fn elem_of(&self, k: Sec) -> Option<u32> { (k == Sec::Status).then_some(RETRY_ELEM) }
    fn plain_name(&self, k: Sec, i: usize) -> Option<&'static str> { (k == Sec::Status && i == 0).then_some("retry") }
    fn plain_group(&self, _cx: &Cx<'_, H>, _k: Sec, id: GroupId, extent: Rect) -> GroupSpec {
        GroupSpec { id, kind: GroupKind::Free, seat: Seat::First, reachable: AxisMask::VERTICAL,
            edge: [EdgeRule::Geometric, EdgeRule::Geometric, EdgeRule::Stop, EdgeRule::Stop], extent, len: 1, elem: ElemKind::Bare }
    }
    fn draw_heading(&self, k: Sec, f: &mut DrawFrame<'_, '_, H>, r: Rect, lift: f32) {
        let title = match k { Sec::Recent => plx_os::i18n::msg::browse_livetv_recent_c(), Sec::Now => plx_os::i18n::msg::browse_livetv_on_now_c(), Sec::Channels => plx_os::i18n::msg::browse_livetv_channels_c(), _ => return };
        plx_ui::cards::draw_heading_c(f.painter.alpha(f.page_alpha).translate(0.0, r.y), title, c"", c"", MARGIN_X, -lift, 1.0, f.measure);
    }
    fn custom_draw(&self, k: Sec, f: &mut DrawFrame<'_, '_, H>, r: Rect, _scroll: f32) {
        if k == Sec::Head {
            let p = f.painter.alpha(f.page_alpha).alpha(self.handoff.head_alpha(r.y));
            self.header_row(f.measure).draw(p, r.y, &self.header_capsules, self.header_pop.scale_with(0, f.press), None);
        } else if k == Sec::Status && self.mode.reads_out() {
            self.status_overlay(f.cx.tick.ms).focused(f.focus.current.is_some_and(|k| k.entry == self.entry && k.elem == RETRY_ELEM))
                .draw_measured(&Env::inert(), f.painter.alpha(f.page_alpha), f.measure);
        }
    }
}

pub struct LiveTvScreen {
    page: Page,
    stack: Stack<Sec>,
    ground: PageGround,
    ground_seeded: bool,
    /// The page's own arrival seat while nobody has chosen another (the Watchlist's rule): `None`
    /// once the reader acts or the page is restored.
    seat: Option<Option<GroupId>>,
    /// Seats this page asked for whose `Enter` has not come back yet.
    own_enters: u8,
    /// How many of those were queued before the reader acted.
    stale_enters: u8,
}

impl LiveTvScreen {
    /// The heartbeat word, byte-identical to the route word.
    pub const WORD: &'static str = "livetv";

    pub const SHAPE: &'static str = "LiveTvScreen{entry:u32,status:u32,channels:[{sid:u32,rk:str}],airings:[u32],recent:[u32],stack:Stack{scroll:{pos:f32,vel:f32},target:f32,sections:[Shelf{row,pop},Shelf{row,pop},Shelf{row,pop}]},handoff:Handoff{tucked:bool,chrome:f32,capsule:f32}}";

    pub fn new(entry: EntryId) -> Self {
        Self {
            page: Page {
                entry,
                channels: Vec::new(),
                airings: Vec::new(),
                recent: Vec::new(),
                rest: Vec::new(),
                mode: Mode::Loading,
                content: Vec::new(),
                synced: None,
                clock: 0,
                header_capsules: plx_ui::widgets::TabStrip::new(),
                header_pop: plx_ui::widgets::CtlPop::new(),
                handoff: plx_ui::scroll_chrome::Handoff::new(),
                header_rows: SharedRow::new(),
            },
            stack: Stack::new(entry).home_when_unfocused(true),
            ground: PageGround::new(),
            ground_seeded: false,
            seat: None,
            own_enters: 0,
            stale_enters: 0,
        }
    }

    /// Every shelf names the channel that supplies its card (Recent's tiles and the Channels shelf's on the even
    /// elements, On now's poster on the odd ones).
    fn press<H: LiveTvLike>(&self, elem: u32, cx: &Cx<'_, H>) -> Option<ContentReq> {
        let index = elem.checked_sub(FIRST_CARD_ELEM)? as usize / 2;
        let view = H::livetv(cx);
        let channel = view.channels.get(index)?;
        if elem % 2 == 1 && channel.now.is_empty() { return None; }
        Some(ContentReq::Play { play: PlayIntent::Live {
            sid: channel.sid, dvr: channel.dvr.clone(), channel: channel.channel.clone(), title: channel.name.clone(),
        }, resume_ns: 0 })
    }

    fn tick<H: LiveTvLike>(&mut self, t: Tick, cx: &Cx<'_, H>, fx: &mut Effects<'_, H>) {
        self.page.header_row(cx.measure).step(self.page.entry, cx.focus.current, &mut self.page.header_capsules, &mut self.page.header_pop, t.dt());
        self.page.handoff.step(t.dt(), self.stack.scroll(), self.stack.target(), NOW_STYLE.pitch_fixed(), false);
        // The page's own haze, fixed: it does not follow the focused tile.
        let haze = AmbientWash::target(theme::WASH_LIVE_TV, [theme::WASH_LIVE_TV_W; 4]);
        if self.ground_seeded {
            self.ground.key_target(haze, t.dt());
        } else {
            self.ground.jump_target(haze);
            self.ground_seeded = true;
        }
        // A programme's words and bar move with the minute, with no event to say so.
        let stamp = self.page.clock_stamp(H::livetv(cx), H::wall(cx));
        if stamp != self.page.clock {
            self.page.clock = stamp;
            fx.invalidate(Provenance::Input);
        }
    }

    fn route<H: LiveTvLike>(&self, out: HomeReq, fx: &mut Effects<'_, H>) {
        fx.push(Fx::App(AppFx::Home(out)));
        fx.invalidate(Provenance::Input);
    }

    /// Seat the arrival in the first shelf once it has a channel; until then seat nothing (the
    /// Watchlist's `seat_first`).
    fn seat_first<H: LiveTvLike>(&mut self, fx: &mut Effects<'_, H>) {
        let regions = self.page.regions();
        let want = regions.first_seat();
        let target = want.map(FocusTarget::FirstInGroupAnimated).or_else(|| regions.unseat());
        self.seat = (want != Some(RECENT_GROUP)).then_some(want);
        if let Some(target) = target {
            self.seat_own(target, fx);
        }
    }

    fn follow_seat<H: LiveTvLike>(&mut self, fx: &mut Effects<'_, H>) {
        let Some(seated) = self.seat else { return };
        if seated != self.page.regions().first_seat() {
            self.seat_first(fx);
        }
    }

    fn seat_own<H: LiveTvLike>(&mut self, focus: FocusTarget<u32>, fx: &mut Effects<'_, H>) {
        self.own_enters = self.own_enters.saturating_add(1);
        fx.push(Fx::Deliver(fx.from(), Delivery::Screen(ScreenEvent::Enter(Enter::Fresh { focus }))));
    }

    /// BACK, decided by [`PageRegions::back`]: a scrolled page first returns to its top, on the
    /// first tile; from there it leaves for Home.
    fn back<H: LiveTvLike>(&mut self, cx: &Cx<'_, H>, fx: &mut Effects<'_, H>) -> Handled {
        let focus = cx.focus.current.filter(|key| key.entry == self.page.entry);
        let regions = self.page.regions();
        let head = regions.first_seat().unwrap_or(RECENT_GROUP);
        let at = focus.and_then(|key| regions.region_of(key.elem, Focusable::<H>::group_of(self, &key.elem, cx)));
        match regions.back(at, self.stack.target() > 0.5) {
            BackStep::Top => {
                self.seat_own(FocusTarget::FirstInGroupAnimated(head), fx);
            }
            BackStep::Content | BackStep::Header => self.seat_own(FocusTarget::ContainerGroup(head), fx),
            BackStep::Nav => self.seat_own(FocusTarget::ContainerGroup(plx_ui::containers::tabs::STRIP), fx),
            BackStep::Leave => self.route(HomeReq::Tab(HomeTab::Home), fx),
        }
        Handled::Yes
    }

    /// A non-card element committed: the strip's pills and the chip.
    fn activate<H: LiveTvLike>(&mut self, elem: u32, fx: &mut Effects<'_, H>) -> Handled {
        match Self::strip_press(elem) {
            Some(Some(out)) => self.route(out, fx),
            Some(None) => {}
            None => return Handled::No,
        }
        Handled::Yes
    }

    /// A press on the shared strip: the other tabs and the chip are requests; the Live TV pill is
    /// the page the reader is already on.
    fn strip_press(elem: u32) -> Option<Option<HomeReq>> {
        use plx_ui::widgets::{
            TAB_KEY_CHIP, TAB_KEY_HOME, TAB_KEY_LIVETV, TAB_KEY_MOVIES, TAB_KEY_SEARCH, TAB_KEY_SHOWS, TAB_KEY_WATCHLIST,
        };
        match elem {
            TAB_KEY_HOME => Some(Some(HomeReq::Tab(HomeTab::Home))),
            TAB_KEY_MOVIES => Some(Some(HomeReq::Tab(HomeTab::Movies))),
            TAB_KEY_SHOWS => Some(Some(HomeReq::Tab(HomeTab::Shows))),
            TAB_KEY_SEARCH => Some(Some(HomeReq::Tab(HomeTab::Search))),
            TAB_KEY_WATCHLIST => Some(Some(HomeReq::Tab(HomeTab::Watchlist))),
            TAB_KEY_CHIP => Some(Some(HomeReq::Account)),
            TAB_KEY_LIVETV => Some(None),
            _ => None,
        }
    }

    fn view<H: LiveTvLike>(&self) -> impl Focusable<H> + '_ {
        self.stack.view(&self.page)
    }

    /// The page's top edge once the chrome has tucked away (the Watchlist's).
    fn draw_chrome<H: LiveTvLike>(&self, f: &mut DrawFrame<'_, '_, H>) {
        let tuck = self.page.handoff.tuck();
        if tuck <= 0.0 {
            return;
        }
        let ground = self.ground.sample(Rect::FULL, SCR_W * 0.5, 0.0);
        plx_ui::scroll_capsule::draw_top_fade(f.painter.alpha(f.page_alpha), SCR_W, ground, tuck, None);
    }
}

plx_ui::focusable_via_view!(LiveTvScreen, H: [LiveTvLike], view);

impl LogicalState for LiveTvScreen {
    fn write(&self, c: &mut Canon) {
        c.u32(self.page.entry.0);
        // the store's status as the mode reads it: any row is `Ready` (1); a `Ready` store with no channel is the empty read-out (2)
        c.u32(match self.page.mode { Mode::Loading => 0, Mode::Empty => 2, Mode::Failed => 3, _ => 1 });
        c.seq(self.page.channels.len());
        for (sid, rk) in &self.page.channels { c.u32(u32::from(sid.raw())).str(rk); }
        c.seq(self.page.airings.len());
        for &i in &self.page.airings { c.u32(i as u32); }
        c.seq(self.page.recent.len());
        for &i in &self.page.recent { c.u32(i as u32); }
        self.stack.write(c);
        self.page.handoff.write(c);
    }

    fn probe(&self, out: &mut String) {
        out.push_str(&format!("livetv channels={}", self.page.channels.len()));
    }
}

impl<H: LiveTvLike> Machine<H> for LiveTvScreen {
    type Ev = ScreenEvent<H>;

    fn step(&mut self, ev: &Self::Ev, cx: &Cx<'_, H>, fx: &mut Effects<'_, H>) -> Handled {
        match ev {
            ScreenEvent::Tick(_) | ScreenEvent::Enter(Enter::Fresh { .. }) => self.page.sync(H::livetv(cx)),
            ScreenEvent::StoreChanged(ord, _) if *ord == StoreId::LiveTv.ord() => {
                self.page.synced = None;
                self.page.sync(H::livetv(cx));
            }
            _ => {}
        }
        let card = match self.stack.on(&self.page, ev, cx, fx) {
            Some(StackEvent::Card(_, card)) => Some(card),
            _ => None,
        };
        // The reader acting makes the seat theirs (claimed on the event, as the Watchlist does).
        if let ScreenEvent::Input(InputEvent { kind, .. }) = ev {
            if matches!(kind, InputKind::Key { edge: Edge::Down, .. } | InputKind::Pointer { .. }
                | InputKind::Click { .. } | InputKind::Drag { .. } | InputKind::Wheel { .. }) {
                self.seat = None;
                self.stale_enters = self.own_enters;
            }
        }
        match ev {
            ScreenEvent::Tick(t) => {
                self.tick(*t, cx, fx);
                self.follow_seat(fx);
                Handled::Yes
            }
            ScreenEvent::Enter(enter) => {
                match enter {
                    Enter::Fresh { .. } if self.own_enters > 0 => {
                        self.own_enters -= 1;
                        if self.stale_enters > 0 {
                            self.stale_enters -= 1;
                            if let Some(held) = cx.focus.current.filter(|key| key.entry == self.page.entry) {
                                self.own_enters = self.own_enters.saturating_add(1);
                                fx.push(Fx::Deliver(fx.from(), Delivery::Screen(ScreenEvent::Enter(Enter::Fresh {
                                    focus: FocusTarget::Elem(held),
                                }))));
                            }
                        }
                    }
                    Enter::Fresh { .. } => {
                        self.seat_first(fx);
                    }
                    Enter::Restored => {
                        self.seat = None;
                        self.stale_enters = self.own_enters;
                    }
                }
                fx.invalidate(Provenance::Nav);
                Handled::Yes
            }
            ScreenEvent::Uncover => {
                fx.invalidate(Provenance::Nav);
                Handled::Yes
            }
            ScreenEvent::FocusMoved { .. } | ScreenEvent::StoreChanged(..) => {
                fx.invalidate(Provenance::Input);
                Handled::Yes
            }
            ScreenEvent::Activate(elem) => {
                if *elem == RETRY_ELEM && self.page.mode == Mode::Failed {
                    fx.push(Fx::App(AppFx::Store(StoreId::LiveTv, StoreCmd::LiveTv(LiveTvCmd::Retry))));
                    fx.invalidate(Provenance::Input);
                    Handled::Yes
                } else { self.activate(*elem, fx) }
            },
            ScreenEvent::PressCommit(_) => {
                if let Some(CardEvent::Activate(elem)) = card {
                    if let Some(out) = self.press(elem, cx) {
                        fx.push(Fx::App(AppFx::Content(out)));
                        fx.invalidate(Provenance::Input);
                    }
                } else if let Some(key) = cx.focus.current.filter(|key| key.entry == self.page.entry) {
                    if key.elem == RETRY_ELEM && self.page.mode == Mode::Failed {
                        fx.push(Fx::App(AppFx::Store(StoreId::LiveTv, StoreCmd::LiveTv(LiveTvCmd::Retry))));
                        fx.invalidate(Provenance::Input);
                    } else { self.activate(key.elem, fx); }
                }
                Handled::Yes
            }
            ScreenEvent::Input(InputEvent { kind: InputKind::Key { key: Key::Back, edge: Edge::Down, .. }, .. }) => {
                self.back(cx, fx)
            }
            _ => Handled::No,
        }
    }
}

impl<H: LiveTvLike> Screen<H> for LiveTvScreen {
    fn focused_card_rect(&self, cx: &Cx<'_, H>, focus: Option<FocusKey<u32>>, at: plx_ui::screen::At) -> Option<Rect> {
        let key = focus.filter(|k| k.entry == self.page.entry && k.elem >= FIRST_CARD_ELEM)?;
        Focusable::<H>::place(self, &key.elem, cx, at).map(|placed| placed.rect)
    }
    fn redraw_focused(&self, f: &mut DrawFrame<'_, '_, H>, focus: Option<FocusKey<u32>>) {
        self.stack.view(&self.page).redraw_focused(f, focus);
    }
    fn opener_departing(&self, key: FocusKey<u32>) -> bool {
        self.stack.view(&self.page).opener_departing::<H>(key)
    }
    fn name(&self) -> &'static str {
        Self::WORD
    }
    fn a11y_ns(&self) -> &'static str {
        Self::WORD
    }
    fn chrome_tuck(&self) -> f32 {
        self.page.handoff.tuck()
    }
    fn state(&self) -> &dyn LogicalState {
        self
    }
    fn crumb(&self, _cx: &Cx<'_, H>) -> Option<std::borrow::Cow<'_, str>> {
        None
    }
    fn prepare(&mut self, _budget: &mut plx_ui::frame::Budget, _cx: &Cx<'_, H>) {}
    fn draw(&mut self, f: &mut DrawFrame<'_, '_, H>) {
        plx_gfx::gfx::frame_clear(theme::CLEAR_RGB.0, theme::CLEAR_RGB.1, theme::CLEAR_RGB.2);
        let p = f.painter.alpha(f.page_alpha);
        self.ground.draw(p, Rect::FULL);
        self.stack.view(&self.page).paint(f);
        self.draw_chrome(f);
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
    fn links(&self, out: &mut Vec<Link>) {
        self.page.regions().links(out);
    }
    fn holds_arrival(&self, dir: plx_ui::screen::Dir) -> bool {
        self.page.regions().holds(dir)
    }
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
#[path = "livetv_tests.rs"]
mod tests;
