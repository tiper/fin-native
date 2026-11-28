//! **Number entry** — the panel that opens when a viewer types a channel number on the What's on page or over the
//! live player (Live TV PR5; handoff SPEC-5). A top-right panel on the safe frame (right 96, top 54, 560 wide): the
//! typed number, then the channels whose number starts with it, a window of five rows on the shared `TableView`
//! (its lineup face), and "+N more" under the window when more match.
//!
//! **What it decides is [`NumberEntry`]'s** (`channel_entry`, a pure machine); this screen feeds it events (a digit,
//! the cursor moving, OK, BACK, a tick) and performs the one effect that comes back. The cursor is the focus
//! engine's, as in every list here (up and down fall through to it); the machine holds the highlight as a lineup
//! index and asks to seat the cursor when the digits change the list ([`EntryFx::Seat`]).
//!
//! **A surface on the container tree**, [`versions_panel`](crate::versions_panel)'s shape: whoever opens it names the
//! style. Over the What's on page it is `Style::Compact`; over the live player it is `Style::PlayerPanel`, which
//! leaves the host live, so the video plane keeps presenting under it.
//!
//! **Its ground follows that style, from one declaration** (owner, 2026-11-27: glass while on the GUI). The panel
//! draws `Ground::Menu` and asks for the compact dim over the host page only ([`Scrim::over_host`]). Over the page
//! the container owes the dim, the field latches and the panel is glass (a menu's own); over the player it owes none,
//! nothing latches and the panel is the opaque sheet it was, with no dim on the picture, for the whole entry
//! (including the tune window, so nothing changes when the picture arrives). Style is fixed for the panel's life and
//! the video plane's bound bit is not, so nothing here follows the bit. If playback starts under a page-hosted panel
//! (a pick, which closes it) the panel's remaining frames, bound-plane frames, are the sheet: `popover_glass` never
//! draws glass over a bound plane, so there is no frame of glass over the video.
//!
//! The panel REPORTS the channel it chose
//! ([`AppMsg::LiveChannelPicked`], to the host instance on its argument) and starts nothing itself: the What's on page
//! plays it through its own play path, and the player hands it to its zap machine (`player::zap`, one
//! `PlayerReq::Tune`), so a number-entry tune and a zap share one tune path, one banner and one art ground.
//!
//! **The opening digits ride on the argument** as a boot address, not an identity: the surface that is already up
//! (mounted, or parked for the next commit) is the one surface, whichever digit opened it, and keys that arrive
//! before it mounts are appended to the parked argument ([`ChannelEntryArg::absorb`]), never a second panel. The lineup is the current store's (one server's channels, in
//! number order), captured when the panel mounts.

use std::borrow::Cow;
use std::convert::Infallible;

use plx_data::livetv::Channel;
use plx_machine::machine::{
    Canon, Cx, Delivery, Edge, Effects, Enter, EntryId, FocusKey, Fx, GroupId, Handled, InputKind, InstanceId, Key,
    LogicalState, Machine, MachineId, NavOp,
};
use plx_plex::plex::ServerId;
use plx_ui::a11y::{A11y, A11yPath};
use plx_ui::consts::pin_digit;
use plx_ui::form::{Form, FormId, FormSection, FormTable, RowKey, RowKind};
use plx_ui::frame::Budget;
use plx_ui::hstack::Item;
use plx_ui::label::{HAlign, Label, VAlign};
use plx_ui::popover::{Popover, Side};
use plx_ui::screen::{
    Activate, At, AxisMask, Dir, DrawFrame, EdgeRule, ElemKind, FocusSource, FocusTarget, Focusable, GroupKind,
    GroupSpec, HitSource, Hover, Placed, RenderStrategy, Scrim, Screen, ScreenEvent, Seat, Step, Stop,
};
use plx_ui::table::{Badge, Row, CONTENT_X, PAD_V, ROW_H, SIDE};
use plx_ui::vstack::{Anchor, VStack};
use plx_ui::{theme, Rect};

use super::channel_entry::{self as entry, Cand, Ev, Fx as EntryFx, NumberEntry, LIST_ROWS};
use crate::registry::{AppLike, AppMsg, LiveTvLike, PageMemory};

/// The fields [`ChannelEntryScreen::write`] canonicalises, for the recorder's shape pin (§5.4).
pub const SHAPE: &str = "ChannelEntry{arg:ChannelEntryArg{host:u32,digits:str,playing:Option<{sid:u32,dvr:str,channel:str}>},entry:{Fresh{digits:str},Typing{digits:str,highlight:Option<u32>,last:u32},Done{digits:str}},table:TableViewMotion}";

/// The channel that is on screen when the panel opens, by identity: server, DVR and key.
pub type PlayingKey = (ServerId, String, String);

/// What the container is asked to present: the instance the choice is reported to, the digits that opened the panel
/// (one key, or several that arrived in the same frame), and the channel already on screen (the player's; `None`
/// over the page).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ChannelEntryArg {
    pub host: InstanceId,
    pub digits: String,
    pub playing: Option<PlayingKey>,
}

impl ChannelEntryArg {
    /// Open with the one digit `first` (0 to 9).
    pub fn opened_by(host: InstanceId, first: u8, playing: Option<PlayingKey>) -> Self {
        Self { host, digits: char::from(b'0' + first.min(9)).to_string(), playing }
    }

    /// A second request for this panel arrived before it mounted: its digits follow ours.
    pub fn absorb(&mut self, later: &Self) {
        self.digits.push_str(&later.digits);
    }
}

impl LogicalState for ChannelEntryArg {
    fn write(&self, c: &mut Canon) {
        c.u32(self.host.0).str(&self.digits);
        match &self.playing {
            Some((sid, dvr, channel)) => {
                c.u32(1).u32(u32::from(sid.raw())).str(dvr).str(channel);
            }
            None => {
                c.u32(0);
            }
        }
    }
    fn probe(&self, out: &mut String) {
        out.push_str("channel_entry_arg");
    }
}

/// A row's identity: the channel's index in the lineup captured at open (its key, never a position in the list).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RowId(pub u32);

impl FormId for RowId {
    fn key(&self) -> RowKey {
        RowKey(self.0)
    }
}

/// `--radius-panel`: the shared panel corner.
const PANEL_RAD: f32 = 24.0;
/// The panel's width and inner padding (SPEC-5).
const PANEL_W: f32 = 560.0;
const PAD: f32 = 24.0;
/// The typed number's line: DISPLAY 48, and the caret is 4 x 48 beside it.
const HEAD_H: f32 = 48.0;
const CARET_W: f32 = 4.0;
/// How far the panel settles down into place (SPEC-5: "settles 16 px down").
const SETTLE: f32 = 16.0;
const GROUP: GroupId = GroupId(0);

pub struct ChannelEntryScreen {
    entry: EntryId,
    arg: ChannelEntryArg,
    /// The lineup as it was when the panel opened, in number order.
    lineup: Vec<Channel>,
    /// The machine's view of it: each channel's number and whether it is the one on screen.
    cands: Vec<Cand>,
    /// The one source of what is typed, what matches and what is highlighted.
    machine: NumberEntry,
    form: FormTable<RowId, RowId, Infallible>,
}

impl ChannelEntryScreen {
    pub fn new(entry: EntryId, arg: ChannelEntryArg, channels: &[Channel]) -> Self {
        let cands = channels
            .iter()
            .map(|c| {
                let playing = arg
                    .playing
                    .as_ref()
                    .is_some_and(|(sid, dvr, channel)| *sid == c.sid && *dvr == c.dvr && *channel == c.channel);
                Cand::new(&c.vcn, playing)
            })
            .collect();
        let machine = NumberEntry::open_digits(&arg.digits);
        let mut form = FormTable::with_class(crate::registry::BAND, plx_ui::menu::Class::Action);
        form.table.lineup(true);
        let mut screen = Self { entry, arg, lineup: channels.to_vec(), cands, machine, form };
        screen.rebuild();
        screen
    }

    pub fn machine(&self) -> &NumberEntry {
        &self.machine
    }

    /// The matches of the digits typed so far, as lineup indices in lineup order.
    pub fn shown(&self) -> Vec<usize> {
        entry::matches(&self.cands, self.machine.typed()).collect()
    }

    /// How many channels match the digits typed so far.
    fn shown_len(&self) -> usize {
        entry::matches(&self.cands, self.machine.typed()).count()
    }

    /// How many matches the window of [`LIST_ROWS`] rows leaves under it: the "+N more" count.
    pub fn more(&self) -> usize {
        self.shown_len().saturating_sub(LIST_ROWS)
    }

    /// The panel's three stacked parts: the typed number, the list (or the no-match read-out) and the "+N more" line.
    fn parts(&self, measure: &dyn plx_machine::machine::Measure) -> [Item; 3] {
        let n = self.shown_len();
        let list = if n == 0 { ROW_H } else { n.min(LIST_ROWS) as f32 * ROW_H + PAD_V };
        let more = if self.more() > 0 { Item::new(measure.cap_h(theme::size::MICRO)) } else { Item::absent() };
        [Item::new(HEAD_H), Item::new(list), more]
    }

    /// The panel's rect: the safe frame's top-right corner ([`plx_ui::popover::corner_panel`]).
    pub fn rect(&self, measure: &dyn plx_machine::machine::Measure) -> Rect {
        let items = self.parts(measure);
        let h = 2.0 * PAD + VStack::new(&items, theme::space::SM, Anchor::Top(0.0)).height();
        plx_ui::popover::corner_panel(PANEL_W, h).rect
    }

    /// The three parts' bands inside `r`, top to bottom.
    fn bands(&self, r: Rect, measure: &dyn plx_machine::machine::Measure) -> [Option<Rect>; 3] {
        let items = self.parts(measure);
        let stack = VStack::new(&items, theme::space::SM, Anchor::Top(r.y + PAD));
        let band = |i: usize| stack.span(i).map(|(y, h)| Rect::new(r.x + PAD, y, r.w - 2.0 * PAD, h));
        [band(0), band(1), band(2)]
    }

    /// The list's table frame: the table draws its pill [`SIDE`] in from its frame, so the frame is wider than the
    /// band and the pill is exactly the band wide.
    fn frame(&self, measure: &dyn plx_machine::machine::Measure) -> Rect {
        match self.bands(self.rect(measure), measure)[1] {
            Some(b) => Rect::new(b.x - SIDE, b.y, b.w + 2.0 * SIDE, b.h),
            None => Rect::new(0.0, 0.0, 0.0, 0.0),
        }
    }

    /// Rebuild the rows for the digits typed so far. The highlight is the machine's.
    fn rebuild(&mut self) {
        let tag = plx_os::i18n::msg::widgets_live_playing();
        let mut section: FormSection<RowId, RowId, Infallible> = FormSection::new("");
        for i in self.shown() {
            let c = &self.lineup[i];
            let mut row = Row::new(c.name.clone()).lead(c.vcn.clone()).server_label();
            if self.cands[i].playing {
                row = row.badge(Badge::Text(tag.to_string()));
            }
            let id = RowId(i as u32);
            section = section.item(id, RowKind::Button, id, row);
        }
        let keep = self.machine.highlight().map(|i| RowId(i as u32));
        self.form.set(Form::new().section(section), keep.as_ref());
    }

    /// Step the machine and perform what it asked for.
    fn step_machine<H: AppLike<Memory = PageMemory>>(&mut self, ev: Ev, fx: &mut Effects<'_, H>) {
        // digits only ever grow (the opening ones are re-typed in place), so a change of length is a change of list
        let before = self.machine.typed().len();
        let ask = self.machine.step(ev, &self.cands);
        if self.machine.typed().len() != before {
            self.rebuild();
            fx.invalidate(plx_machine::present::Provenance::Input);
        }
        match ask {
            EntryFx::None => {}
            EntryFx::Seat(i) => {
                let focus = FocusTarget::Elem(FocusKey { entry: self.entry, elem: i as u32 });
                fx.push(Fx::Deliver(fx.from(), Delivery::Screen(ScreenEvent::Enter(Enter::Fresh { focus }))));
            }
            EntryFx::Tune(i) => {
                let c = &self.lineup[i];
                fx.push(Fx::Deliver(
                    MachineId::Instance(self.arg.host),
                    Delivery::Screen(ScreenEvent::App(AppMsg::LiveChannelPicked {
                        sid: c.sid,
                        dvr: c.dvr.clone(),
                        channel: c.channel.clone(),
                    })),
                ));
                self.leave(fx);
            }
            EntryFx::Leave => self.leave(fx),
        }
    }

    fn leave<H: AppLike<Memory = PageMemory>>(&self, fx: &mut Effects<'_, H>) {
        fx.push(Fx::Nav(NavOp::Dismiss(self.entry)));
    }

    /// Follow the cursor the engine holds (the table scrolls with its selection).
    fn follow(&mut self, elem: u32) {
        if let Some(i) = self.form.index_of_key(RowKey(elem)) {
            self.form.table.sel = i as i32;
        }
    }

    /// Where focus seats: the machine's highlight, else the table's cursor, else its first row.
    fn seat_key(&self) -> FocusKey<u32> {
        let key = self
            .machine
            .highlight()
            .map(|i| RowKey(i as u32))
            .filter(|k| self.form.index_of_key(*k).is_some())
            .or_else(|| self.form.selected_key())
            .or_else(|| self.form.opening_key());
        FocusKey { entry: self.entry, elem: key.map_or(0, |k| k.0) }
    }
}

#[cfg(test)]
#[path = "channel_entry_panel_tests.rs"]
pub(crate) mod tests;

impl<H: AppLike<Memory = PageMemory>> Machine<H> for ChannelEntryScreen {
    type Ev = ScreenEvent<H>;
    fn step(&mut self, ev: &Self::Ev, cx: &Cx<'_, H>, fx: &mut Effects<'_, H>) -> Handled {
        match ev {
            ScreenEvent::Tick(tick) => {
                self.step_machine(Ev::Tick { now: tick.ms }, fx);
                if let Some(held) = cx.focus.current.filter(|key| key.entry == self.entry) {
                    self.follow(held.elem);
                }
                let h = self.frame(cx.measure).h;
                self.form.table.update(tick.dt(), h);
                Handled::Yes
            }
            // dismissed from outside (a press that missed the panel): it stops acting for the page, so its timer can
            // no longer tune after the dismissal. The container already dismissed it: BACK's `Leave` is dropped.
            ScreenEvent::Closing => {
                self.machine.step(Ev::Back, &self.cands);
                Handled::Yes
            }
            ScreenEvent::FocusMoved { to, .. } => {
                if self.form.index_of_key(RowKey(to.elem)).is_some() {
                    self.step_machine(Ev::Moved { to: to.elem as usize, now: cx.tick.ms }, fx);
                    self.follow(to.elem);
                }
                fx.invalidate(plx_machine::present::Provenance::Input);
                Handled::Yes
            }
            ScreenEvent::Activate(elem) => {
                if self.form.index_of_key(RowKey(*elem)).is_some() {
                    self.step_machine(Ev::Moved { to: *elem as usize, now: cx.tick.ms }, fx);
                    self.step_machine(Ev::Ok { now: cx.tick.ms }, fx);
                }
                Handled::Yes
            }
            ScreenEvent::PressCommit(_) => {
                self.step_machine(Ev::Ok { now: cx.tick.ms }, fx);
                Handled::Yes
            }
            ScreenEvent::Input(input) => match &input.kind {
                InputKind::Key { key: Key::Back, edge: Edge::Down, .. } => {
                    self.step_machine(Ev::Back, fx);
                    Handled::Yes
                }
                InputKind::Key { sym, wcode, edge: Edge::Down, at_edge: false, .. } => match pin_digit(*sym, *wcode) {
                    Some(d) => {
                        self.step_machine(Ev::Digit { d: d - b'0', now: input.at.ms }, fx);
                        Handled::Yes
                    }
                    // an arrow or OK: the engine moves the cursor / commits; the idle timer restarts
                    None => {
                        self.step_machine(Ev::Touch { now: input.at.ms }, fx);
                        Handled::No
                    }
                },
                _ => Handled::No,
            },
            _ => Handled::No,
        }
    }
}

/// One column of `Bare` rows over the table's own cursor, *Select Version*'s `Focusable`.
impl<H: LiveTvLike> Focusable<H> for ChannelEntryScreen {
    fn groups(&self, cx: &Cx<'_, H>, out: &mut Vec<GroupSpec>) {
        out.push(GroupSpec {
            id: GROUP,
            kind: GroupKind::Column,
            seat: Seat::Remembered,
            reachable: AxisMask::VERTICAL,
            edge: [EdgeRule::Stop; 4],
            extent: self.frame(cx.measure),
            len: self.form.focusable_len(),
            elem: ElemKind::Bare,
        });
    }
    fn group_of(&self, elem: &u32, _cx: &Cx<'_, H>) -> Option<GroupId> {
        self.form.index_of_key(RowKey(*elem)).map(|_| GROUP)
    }
    fn neighbour(&self, key: FocusKey<u32>, dir: Dir, _cx: &Cx<'_, H>) -> Step<u32> {
        let delta = match dir {
            Dir::Up => -1,
            Dir::Down => 1,
            _ => return Step::Edge,
        };
        match self.form.step_key(RowKey(key.elem), delta) {
            Some(next) => Step::Move(FocusKey { entry: self.entry, elem: next.0 }),
            None => Step::Edge,
        }
    }
    fn place(&self, elem: &u32, cx: &Cx<'_, H>, _at: At) -> Option<Placed> {
        let index = self.form.index_of_key(RowKey(*elem))?;
        let frame = self.frame(cx.measure);
        let rect = self.form.table.row_frame(frame, index as i32)?;
        Some(Placed { rect, rest_rect: rect, clip: frame, index: Some(index as u32) })
    }
    fn reconcile(&self, want: FocusKey<u32>, cx: &Cx<'_, H>) -> FocusKey<u32> {
        if self.group_of(&want.elem, cx).is_some() {
            want
        } else {
            self.seat_key()
        }
    }
    fn seat(&self, _g: GroupId, _from: Placed, _cx: &Cx<'_, H>) -> FocusKey<u32> {
        self.seat_key()
    }

    /// **Row `n` is `row.<n>`**, the shared table convention ([`plx_ui::geom::table_a11y`]): `n` is the row's ordinal
    /// among the rows shown, so a press by id names the match, not a lineup position. A row picks a channel of the
    /// lineup, not an item of the page, so it carries no subject.
    fn a11y(&self, elem: &u32, _cx: &Cx<'_, H>) -> Option<A11y<'_>> {
        let slot = self.form.index_of_key(RowKey(*elem))?;
        plx_ui::geom::table_a11y(&self.form.table, Some(&self.form), slot)
    }
    fn a11y_elems(&self, g: GroupId, _cx: &Cx<'_, H>, out: &mut Vec<u32>) {
        if g == GROUP {
            out.extend(
                plx_ui::geom::table_elems(&self.form.table, Some(&self.form))
                    .filter_map(|i| self.form.key_at(i))
                    .map(|k| k.0),
            );
        }
    }
    fn a11y_find(&self, path: &A11yPath, _cx: &Cx<'_, H>) -> Option<u32> {
        let slot = plx_ui::geom::table_a11y_find(&self.form.table, Some(&self.form), path)?;
        self.form.key_at(slot).map(|k| k.0)
    }
}

impl LogicalState for ChannelEntryScreen {
    fn write(&self, c: &mut Canon) {
        self.arg.write(c);
        match &self.machine {
            NumberEntry::Fresh { digits } => {
                c.u32(0).str(digits);
            }
            NumberEntry::Typing { digits, highlight, last } => {
                c.u32(1).str(digits).u32(highlight.map_or(0, |h| h as u32 + 1)).u32(*last);
            }
            NumberEntry::Done { digits } => {
                c.u32(2).str(digits);
            }
        }
        self.form.table.write_motion(c);
    }
    fn probe(&self, out: &mut String) {
        out.push_str("channel_entry");
    }
}

impl<H: LiveTvLike> Screen<H> for ChannelEntryScreen {
    fn name(&self) -> &'static str {
        "channel_entry"
    }
    fn state(&self) -> &dyn LogicalState {
        self
    }
    fn crumb(&self, _cx: &Cx<'_, H>) -> Option<Cow<'_, str>> {
        None
    }
    fn prepare(&mut self, _b: &mut Budget, _cx: &Cx<'_, H>) {}
    /// The compact menu's dim over a held host page only: what makes the panel glass there ([`Scrim::over_host`]).
    fn scrim(&self) -> Scrim {
        Scrim::over_host(theme::underlay::DIM_COMPACT)
    }
    fn draw(&mut self, f: &mut DrawFrame<'_, '_, H>) {
        let appear = f.page_alpha;
        let measure = f.measure;
        let r = self.rect(measure);
        let (dx, dy) = Popover::rise_offset(Side::Below, SETTLE, appear);
        let p = f.painter.alpha(appear).translate(dx, dy);
        let field = f.underlay;
        let [head, list, more] = self.bands(r, measure);
        let typed = self.machine.typed();
        let shown = self.shown_len();
        let left = shown.saturating_sub(LIST_ROWS);
        let frame = self.frame(measure);
        let text_x = |band: Rect| band.x + CONTENT_X - SIDE;
        plx_ui::profile::phase("dt.channel_entry", || {
            plx_ui::widgets::panel_ground(p, r, PANEL_RAD, field, plx_ui::widgets::Ground::Menu);
            if let (Some(band), Ok(run)) = (head, std::ffi::CString::new(typed)) {
                let w = Label::new(run.as_ptr(), theme::size::DISPLAY, theme::TEXT_PRIMARY)
                    .bold()
                    .h(HAlign::Left)
                    .v(VAlign::Middle)
                    .draw(p, Rect::new(text_x(band), band.y, band.w, band.h));
                let caret = Rect::new(text_x(band) + w + CARET_W, band.y, CARET_W, band.h);
                p.rect(caret, 0.0, theme::TEXT_PRIMARY, theme::TEXT_PRIMARY, 0.0);
            }
            if let Some(band) = list {
                if shown == 0 {
                    let none = plx_os::i18n::msg::widgets_live_entry_none(typed);
                    if let Ok(text) = std::ffi::CString::new(none) {
                        Label::new(text.as_ptr(), theme::size::CAPTION, theme::TEXT_SECONDARY)
                            .h(HAlign::Left)
                            .v(VAlign::Middle)
                            .draw(p, Rect::new(text_x(band), band.y, band.w, band.h));
                    }
                } else {
                    self.form.table.draw(p, frame, measure);
                }
            }
            if let Some(band) = more {
                let rest = plx_os::i18n::msg::widgets_live_entry_more(&left.to_string());
                if let Ok(text) = std::ffi::CString::new(rest) {
                    Label::new(text.as_ptr(), theme::size::MICRO, theme::TEXT_TERTIARY)
                        .h(HAlign::Left)
                        .v(VAlign::CapTop)
                        .draw(p, Rect::new(text_x(band), band.y, band.w, band.h));
                }
            }
        });
        let hit_p = f.painter.alpha(appear);
        for index in 0..self.form.table.n_rows() as usize {
            let Some(elem) = self.form.key_at(index).map(|k| k.0) else {
                continue;
            };
            if let Some(placed) = <Self as Focusable<H>>::place(self, &elem, f.cx, At::Drawn) {
                f.stop(
                    hit_p,
                    Stop {
                        key: FocusKey { entry: self.entry, elem },
                        rect: placed.rect,
                        rest_rect: placed.rest_rect,
                        clip: placed.clip,
                        hover: Hover::Focus,
                        activate: Activate::Immediate,
                    },
                );
            }
        }
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
