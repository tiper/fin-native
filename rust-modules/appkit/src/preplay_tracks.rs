//! **Choose audio and subtitle tracks before playback starts** (#1532): the rows, the picks and the
//! commit of the *Audio & Subtitles* panel, with no session and no screen.
//!
//! The Detail hero's disc, the `…` menu's row and the playback failure screen's entry all open the
//! same two-level panel, so the rows live here once and each host owns only its surface. A host
//! holds a [`PickerState`], whose `ui::menu::Menu` owns the form, the stack and the slide (one stack,
//! one slide, one header with the back mark); the host feeds it the screen's events through
//! [`PickerState::on_screen`], and when the menu answers `Out::Picked` resolves the row to a [`Pick`]
//! ([`PickerState::pick`], [`pick_of`]) and commits with [`commit`]. BACK pops a page before it dismisses.
//!
//! **Not a second track list.** Audio rows are [`crate::track_menu::audio_track_row`], the
//! in-player Audio tab's own row; the Subtitles page is `metadata::sub_layout::sub_sections`
//! grouped the same way (Off, your languages, one section per multi-track language) with the
//! Subtitles tab's own row builders. Differences, each forced by there being no playback yet: no
//! Timing / Style / Forced only (they act on a drawn subtitle or on the audio that plays), no
//! "converted by the server" sub-line (no route), and the Other languages drill-in is flattened
//! into one trailing section, A-Z (a second slide level for a rarely used list).
//!
//! **A pick is the user pick the in-player menu makes**: [`commit`] is
//! [`plx_media::route::commit_preplay_pick`], which queues `PUT /library/parts/{id}?audioStreamID=..
//! &subtitleStreamID=..` (Off = `subtitleStreamID=0`) and, for an episode, writes the show's track
//! memory, so the start's plan reads the pick instead of the old selection.
//!
//! **One pick value.** What is chosen is a [`plx_data::metadata::PreplayPick`] (part, audio stream,
//! subtitle as `Server` / `Off` / `Track`): the metadata store owns it; `MetadataCmd::PatchTracks`
//! ([`patch_command`]) writes it and `MetadataCmd::SpendPick` spends it when a playback it fed starts, and
//! the page's close or replacement drops it. [`PickerState`] is seeded from [`PreplayTracks::pick`]
//! (the store's, or on the failure screen the session's held / standing selection) and carries each
//! [`Selection::pick`] forward as the only record of the choice; what the rows show is [`chosen`] of
//! it. The resolve applies the same value to the item it plans, so a pick reaches the start whichever
//! way the item arrives (the page's copy, a default fetch, a named version's fetch). Other
//! readers read it; none keeps a second copy.
//!
//! **Host contract.** [`select`] writes a memo only for the half that differs from the standing pick,
//! which the panel carries forward; a host that rebuilds a panel seeds it from the stored pick, never
//! from the server's flags.

use plx_data::metadata::sub_layout::{self, SubHeader, SubRow};
use plx_data::metadata::track_names::TrackNames;
use plx_data::metadata::{PreplayPick, Stream, SubPick};
use plx_plex::plex::session::prefs::{AudioMemo, SubtitleMemo};
use plx_machine::machine::Measure;
use plx_ui::form::{Form, FormId, FormSection, RowKey, RowKind};
use plx_ui::menu::{Class, Event, Host, Menu, Out, Page};
use plx_ui::table::{Row, Section};
use plx_ui::Rect;

use plx_media::route::PreplayItem;
use crate::track_menu::{audio_track_row, flat_row, in_lang_row};

/// The panel's pages. The root is the two rows; the other two are pushed by OK on one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PickPage {
    Root,
    Audio,
    Subtitles,
}

/// A row's identity: a family base plus the track's index in the item's list, never a list
/// position of the page (the in-player menu's rule), so a rebuild lands on the same row.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PickRow {
    OpenAudio,
    OpenSubtitles,
    Audio(usize),
    SubOff,
    Sub(usize),
}

impl FormId for PickRow {
    fn key(&self) -> RowKey {
        RowKey(match self {
            PickRow::OpenAudio => 1,
            PickRow::OpenSubtitles => 2,
            PickRow::SubOff => 3,
            PickRow::Audio(i) => 1000 + *i as u32,
            PickRow::Sub(i) => 2000 + *i as u32,
        })
    }
}

pub type PickForm = Form<PickRow, (), PickPage>;

/// The item's tracks the picker shows: the playing-item store's own lists (`metadata::playing()`),
/// in the server's order.
#[derive(Clone, Copy)]
pub struct TrackSet<'a> {
    /// The part these lists are of: the file's key ([`PreplayPick::is_for`]), which a file with no audio
    /// stream still has.
    pub part_id: i64,
    pub audio: &'a [Stream],
    pub subs: &'a [Stream],
}

/// What is chosen, as indices into [`TrackSet`]'s lists; `sub: None` is Off.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Chosen {
    pub audio: usize,
    pub sub: Option<usize>,
}

/// A subtitle the picker offers: before playback there is no route to burn a sidecar, so only the
/// embedded tracks and the sidecars the app can draw itself are listed.
fn offered(s: &Stream) -> bool {
    !s.external || s.sidecar_renderable()
}

/// What is chosen: the pick when there is one for this file, over what the server has selected
/// (`Stream.selected`), falling back to the file's default audio and to Off, which is what the plan
/// starts from. The ONE reading of a [`PreplayPick`] into indices; the panel's seed, its carried choice
/// and each pick's diff all come through it.
pub fn chosen(set: TrackSet<'_>, pick: Option<&PreplayPick>) -> Chosen {
    let pick = pick.filter(|p| p.is_for(set.part_id));
    let audio = pick
        .and_then(|p| set.audio.iter().position(|s| s.id == p.audio_sid))
        .or_else(|| set.audio.iter().position(|s| s.selected))
        .or_else(|| set.audio.iter().position(|s| s.default))
        .unwrap_or(0);
    // The server's selected subtitle, even a sidecar the app cannot draw: Off must not read as
    // checked while the server still has a track selected (the page then lists it, see
    // `subtitle_form`).
    let sub = match pick.map_or(SubPick::Server, |p| p.sub) {
        SubPick::Server => set.subs.iter().position(|s| s.selected),
        SubPick::Off => None,
        SubPick::Track(id) => set.subs.iter().position(|s| s.id == id),
    };
    Chosen { audio, sub }
}

/// The audio list's row label for track `i`, the word the root's Audio row reads out.
pub fn audio_label(set: TrackSet<'_>, i: usize) -> String {
    set.audio.get(i).map_or_else(String::new, |s| {
        if s.lang.is_empty() { plx_os::i18n::msg::widgets_tracks_unknown().to_string() } else { s.lang.clone() }
    })
}

/// The word the root's Subtitles row reads out: the chosen track's language, or Off.
pub fn subtitle_label(set: TrackSet<'_>, sub: Option<usize>) -> String {
    match sub.and_then(|i| set.subs.get(i)) {
        Some(s) if !s.lang.is_empty() => s.lang.clone(),
        Some(_) => plx_os::i18n::msg::widgets_tracks_unknown().to_string(),
        None => plx_os::i18n::msg::widgets_tracks_off().to_string(),
    }
}

/// The panel's title band and the first page's header: the catalog's existing *Audio & subtitles*.
pub fn title() -> &'static str {
    plx_os::i18n::msg::settings_audio_title()
}

/// The root page: Audio and Subtitles, each a drill-in whose value is the current choice. A page
/// with nothing to choose from (one audio track, no subtitles) still lists its row, reading what
/// it has; the pages themselves are the place to see there is nothing else.
pub fn root_form(set: TrackSet<'_>, now: Chosen) -> PickForm {
    Form::new().section(
        FormSection::from_head(Section::new(""))
            .item(
                PickRow::OpenAudio,
                RowKind::Nav(PickPage::Audio),
                (),
                Row::new(plx_os::i18n::msg::widgets_tracks_audio()).value(audio_label(set, now.audio)),
            )
            .item(
                PickRow::OpenSubtitles,
                RowKind::Nav(PickPage::Subtitles),
                (),
                Row::new(plx_os::i18n::msg::widgets_tracks_subtitles()).value(subtitle_label(set, now.sub)),
            ),
    )
}

/// The Audio page: the in-player Audio tab's rows, the chosen one checked.
pub fn audio_form(set: TrackSet<'_>, now: Chosen) -> PickForm {
    let names = TrackNames::new();
    let mut sec = FormSection::new(plx_os::i18n::msg::widgets_tracks_audio());
    for i in 0..set.audio.len() {
        sec = sec.item(PickRow::Audio(i), RowKind::Choice, (), audio_track_row(set.audio, i, i == now.audio, &names, |s| s));
    }
    Form::new().section(sec)
}

/// The Subtitles page: `sub_layout::sub_sections`' grouping with the Subtitles tab's rows. `yours`
/// is "your languages" in preference order (the plan's account preference, then the playing
/// audio's language; callers pass what they know). The model's Timing / Style / Forced only rows
/// and its Other languages drill-in are not drawn (see the module doc): the languages behind that
/// row follow as one section, A-Z.
pub fn subtitle_form(set: TrackSet<'_>, now: Chosen, yours: &[String]) -> PickForm {
    let names = TrackNames::new();
    let shown: Vec<usize> =
        (0..set.subs.len()).filter(|i| offered(&set.subs[*i]) || now.sub == Some(*i)).collect();
    let model = sub_layout::sub_sections(set.subs, &shown, &names, yours, false);
    let active = now.sub.map_or(-1, |i| i as i32);
    let mut form = Form::new();
    for sec in &model.sections {
        let head = match &sec.header {
            // Kept on purpose: the in-player Subtitles tab draws the same `SubHeader::Subtitles` label under its
            // own Subtitles tab, so the title band + label pair is the track menu's (parity, not a duplicate).
            SubHeader::Subtitles => Section::new(plx_os::i18n::msg::widgets_tracks_subtitles()),
            SubHeader::Language { name, tracks } => {
                Section::new(name.clone()).accessory(plx_os::i18n::msg::widgets_tracks_count(*tracks as i64))
            }
            SubHeader::Bare => Section::new(""),
        };
        let mut out = FormSection::from_head(head);
        for row in &sec.rows {
            out = match row {
                SubRow::Off => out.item(
                    PickRow::SubOff,
                    RowKind::Choice,
                    (),
                    Row::new(plx_os::i18n::msg::widgets_tracks_off()).checked(now.sub.is_none()),
                ),
                SubRow::Flat(t) => out.item(PickRow::Sub(t.i), RowKind::Choice, (), flat_row(t, active)),
                SubRow::InLanguage(t) => out.item(PickRow::Sub(t.i), RowKind::Choice, (), in_lang_row(t, active)),
                SubRow::ForcedOnly | SubRow::OtherLanguages { .. } | SubRow::Timing | SubRow::Style => out,
            };
        }
        form = form.section(out);
    }
    if !model.other.is_empty() {
        let mut out = FormSection::new(plx_os::i18n::msg::widgets_tracks_other_languages());
        for lang in &model.other {
            for t in &lang.tracks {
                out = out.item(PickRow::Sub(t.i), RowKind::Choice, (), flat_row(t, active));
            }
        }
        form = form.section(out);
    }
    form
}

/// What a focused row means when OK lands on it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pick {
    /// A drill-in: push this page.
    Open(PickPage),
    Audio(usize),
    /// A subtitle track, or `None` for Off.
    Subtitle(Option<usize>),
}

/// A row's meaning, by identity.
pub fn pick_of(row: PickRow) -> Pick {
    match row {
        PickRow::OpenAudio => Pick::Open(PickPage::Audio),
        PickRow::OpenSubtitles => Pick::Open(PickPage::Subtitles),
        PickRow::Audio(i) => Pick::Audio(i),
        PickRow::SubOff => Pick::Subtitle(None),
        PickRow::Sub(i) => Pick::Subtitle(Some(i)),
    }
}

/// A pick, resolved to the stream ids the PUT carries, the descriptors the show's memory keeps and
/// the [`PreplayPick`] that is the one value of what is chosen.
#[derive(Clone, PartialEq, Debug)]
pub struct Selection {
    pub chosen: Chosen,
    pub audio_sid: i64,
    /// `0` is Off.
    pub subtitle_sid: i64,
    /// The standing pick after this one: what the store keeps for a page (`PatchTracks`; the failure
    /// screen's goes to the session, `route::hold_pick_for_retry`) and the next pick is
    /// diffed against. Off is a value of its own, not `subtitle_sid == 0`: an audio-only pick on an
    /// item whose subtitle was never touched also sends `0`.
    pub pick: PreplayPick,
    /// Only for the half the viewer changed.
    pub audio_memo: Option<AudioMemo>,
    pub subtitle_memo: Option<SubtitleMemo>,
}

/// Apply `pick` over `standing` (the pick so far for this file, `set.part_id` its part): the new choice
/// and everything [`commit`] writes. `None` for a drill-in or an index off the list. A subtitle row
/// (Off or a track) replaces the standing subtitle, an audio pick keeps it.
pub fn select(set: TrackSet<'_>, standing: Option<&PreplayPick>, pick: Pick) -> Option<Selection> {
    let now = chosen(set, standing);
    let mut next = now;
    let mut sub = standing.filter(|p| p.is_for(set.part_id)).map_or(SubPick::Server, |p| p.sub);
    let (mut audio_memo, mut subtitle_memo) = (None, None);
    match pick {
        Pick::Open(_) => return None,
        Pick::Audio(i) => {
            set.audio.get(i)?;
            next.audio = i;
            if i != now.audio {
                audio_memo = plx_media::route::memo_of_audio(set.audio, i);
            }
        }
        Pick::Subtitle(row) => {
            if let Some(i) = row {
                set.subs.get(i)?;
            }
            next.sub = row;
            sub = row.map_or(SubPick::Off, |i| SubPick::Track(set.subs[i].id));
            if row != now.sub {
                subtitle_memo = Some(row.map_or(SubtitleMemo::Off, |i| plx_media::route::memo_of_subtitle(&set.subs[i])));
            }
        }
    }
    let audio_sid = set.audio.get(next.audio).map_or(0, |s| s.id);
    Some(Selection {
        chosen: next,
        audio_sid,
        subtitle_sid: next.sub.map_or(0, |i| set.subs[i].id),
        pick: PreplayPick { part_id: set.part_id, audio_sid, sub },
        audio_memo,
        subtitle_memo,
    })
}

/// Write a pick: the in-player user-pick door, session-free ([`plx_media::route::commit_preplay_pick`]).
pub fn commit(item: &plx_media::route::PreplayItem<'_>, sel: &Selection) -> bool {
    plx_media::route::commit_preplay_pick(item, sel.audio_sid, sel.subtitle_sid, sel.audio_memo.clone(), sel.subtitle_memo.clone())
}

// ---- the panel's state: pages, slide, carried choice -------------------------------------------

/// **What an Audio & Subtitles picker knows, with no table, no stack and no screen**: the item's tracks
/// and the choice made so far, and the page forms they build. The standalone panel ([`PickerState`])
/// and the hero's `…` menu (`screens::item_menu`, whose page stack it stands on) each host one, so the
/// rows, the picks and what a pick writes are this code once.
#[derive(Clone)]
pub struct PickerCore {
    tracks: plx_data::metadata::PreplayTracks,
    /// **What is chosen, as the one value** ([`PreplayPick`]): seeded from [`PreplayTracks::pick`] (the
    /// store's pick, or on the failure screen the held / standing selection), then each pick's
    /// [`Selection::pick`]. `None` until there is one: the rows then read the server's selection.
    /// What the panel shows ([`Self::now`]) is [`chosen`] of it, never a second field.
    pick: Option<PreplayPick>,
}

impl PickerCore {
    pub fn new(tracks: plx_data::metadata::PreplayTracks) -> Self {
        Self { pick: tracks.pick, tracks }
    }

    fn set(&self) -> TrackSet<'_> {
        TrackSet { part_id: PreplayItem::of(&self.tracks).part_id, audio: &self.tracks.audio, subs: &self.tracks.subs }
    }

    pub fn tracks(&self) -> &plx_data::metadata::PreplayTracks {
        &self.tracks
    }

    /// Does the item offer nothing to pick (no audio, no subtitle)? A page of it would list rows with nothing
    /// behind them.
    pub fn is_empty(&self) -> bool {
        self.tracks.audio.is_empty() && self.tracks.subs.is_empty()
    }

    pub fn now(&self) -> Chosen {
        chosen(self.set(), self.pick.as_ref())
    }

    /// "Your languages" for the Subtitles page: the language of the audio that will play.
    fn yours(&self) -> Vec<String> {
        self.tracks.audio.get(self.now().audio).map(|a| a.lang_code.clone()).filter(|c| !c.is_empty()).into_iter().collect()
    }

    /// The rows of `page`, as the standalone panel and the menu both show them.
    pub fn page_form(&self, page: PickPage) -> PickForm {
        match page {
            PickPage::Root => root_form(self.set(), self.now()),
            PickPage::Audio => audio_form(self.set(), self.now()),
            PickPage::Subtitles => subtitle_form(self.set(), self.now(), &self.yours()),
        }
    }

    /// Where focus opens on a page: the checked row, so the viewer sees what is chosen.
    pub fn page_initial(&self, page: PickPage) -> Option<PickRow> {
        match page {
            PickPage::Root => None,
            PickPage::Audio => Some(PickRow::Audio(self.now().audio)),
            PickPage::Subtitles => Some(self.now().sub.map_or(PickRow::SubOff, PickRow::Sub)),
        }
    }

    /// The page's title band (`None` for the root).
    pub fn title_of(page: PickPage) -> Option<String> {
        match page {
            PickPage::Root => None,
            PickPage::Audio => Some(plx_os::i18n::msg::widgets_tracks_audio().to_string()),
            PickPage::Subtitles => Some(plx_os::i18n::msg::widgets_tracks_subtitles().to_string()),
        }
    }

    /// A pick (not a drill-in): carry the choice forward as the standing pick and answer what it
    /// writes. `None` for a drill-in or an index off the list.
    pub fn select(&mut self, pick: Pick) -> Option<Selection> {
        let sel = select(self.set(), self.pick.as_ref(), pick)?;
        self.pick = Some(sel.pick);
        Some(sel)
    }
}

/// **The Audio & Subtitles panel's state, with no screen**: the [`PickerCore`] (the tracks and the choice made
/// so far) and the [`Menu`] that shows its pages (`ui::menu`: the stack, the slide, the placement, the glass).
/// The page showing is the menu's top (the root when nothing is pushed), never a field of its own.
pub struct PickerState {
    core: PickerCore,
    menu: Menu<PickPage, PickRow, ()>,
}

/// `page` (`None` asks for the root), built from the core: its rows, its header and the row focus opens on.
fn page_of(core: &PickerCore, page: Option<PickPage>) -> Option<Page<PickRow, (), PickPage>> {
    let page = page.unwrap_or(PickPage::Root);
    Some(Page::new(core.page_form(page), PickerCore::title_of(page).unwrap_or_default()).initial(core.page_initial(page)))
}

impl PickerState {
    /// A panel that stands at `host`: the host says where it hangs, how it enters and what it stands on (a glass
    /// that grows out of a hero control, a sheet under or above its opener, a card over the video plane), and is
    /// chosen by the screen from where the tracks come from, not here.
    pub fn new(tracks: plx_data::metadata::PreplayTracks, host: Host) -> Self {
        let mut s = Self { core: PickerCore::new(tracks), menu: Menu::new(host, Class::Picker, plx_ui::table_screen::BAND_BASE) };
        s.step(Event::ReplaceRoot { slide: None });
        s
    }

    /// The menu: what the host draws, registers stops on and answers focus questions with.
    pub fn menu(&self) -> &Menu<PickPage, PickRow, ()> {
        &self.menu
    }

    /// One step of the menu, its pages built from the core.
    pub fn step(&mut self, ev: Event<'_, PickRow>) -> Out<()> {
        let core = &self.core;
        self.menu.step(ev, &mut |page| page_of(core, page))
    }

    /// One screen event of the menu (`Menu::on_screen`), its pages built from the core.
    pub fn on_screen<H: plx_machine::machine::Host>(
        &mut self,
        ev: &plx_machine::machine::ScreenEvent<H>,
        cx: &plx_machine::machine::Cx<'_, H>,
        entry: plx_machine::machine::EntryId,
        fx: &mut plx_machine::machine::Effects<'_, H>,
    ) -> Option<plx_ui::menu::Glued<()>>
    where
        H::Elem: plx_ui::geom::IndexElem,
    {
        let core = &self.core;
        self.menu.on_screen(ev, cx, entry, fx, &mut |page| page_of(core, page))
    }

    pub fn tracks(&self) -> &plx_data::metadata::PreplayTracks {
        self.core.tracks()
    }

    pub fn now(&self) -> Chosen {
        self.core.now()
    }

    /// The page showing.
    pub fn page(&self) -> PickPage {
        self.menu.top().unwrap_or(PickPage::Root)
    }

    pub fn depth(&self) -> usize {
        self.menu.depth()
    }

    /// **What the row `key` picked**, after the menu answered its OK with [`Out::Picked`] (a drill-in is a page the
    /// menu pushed and never gets here): carry the choice forward as the standing pick, rebuild the page with the
    /// check moved, and answer what the pick writes. `None` for a row that is no pick.
    pub fn pick(&mut self, key: u32) -> Option<Selection> {
        let id = *self.menu.id_of(key)?;
        let sel = self.core.select(pick_of(id))?;
        self.step(Event::Refresh { keep: Some(id) });
        Some(sel)
    }

    /// The panel's natural rect: the menu's one placement rule for its host.
    pub fn panel_rect(&self, measure: &dyn Measure) -> Rect {
        self.menu.panel_rect(measure)
    }
}

/// The store command that holds what was picked for the item's next start: the plan applies the stored
/// pick over the file's own flags (`PreplayPick::apply`, its audio as the session audio) and the start of
/// the playback that took it spends it (`MetadataCmd::SpendPick`).
pub fn patch_command(tracks: &plx_data::metadata::PreplayTracks, sel: &Selection) -> plx_data::stores::metadata::MetadataCmd {
    plx_data::stores::metadata::MetadataCmd::PatchTracks {
        sid: tracks.sid,
        rk: tracks.rk.clone(),
        pick: sel.pick,
    }
}

/// [`commit`] for a picker's own item.
pub fn commit_for(tracks: &plx_data::metadata::PreplayTracks, sel: &Selection) -> bool {
    commit(&plx_media::route::PreplayItem::of(tracks), sel)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(id: i64, lang: &str, code: &str) -> Stream {
        Stream { id, lang: lang.into(), lang_code: code.into(), ..Stream::default() }
    }

    /// 2 audio (the second is the server's choice) + 2 subtitles (none selected).
    fn item() -> (Vec<Stream>, Vec<Stream>) {
        let mut a2 = stream(102, "French", "fra");
        a2.selected = true;
        (vec![stream(101, "English", "eng"), a2], vec![stream(201, "English", "eng"), stream(202, "French", "fra")])
    }

    /// How many selectable rows a page declares.
    fn rows(form: &PickForm) -> usize {
        form.slots().iter().flatten().count()
    }

    #[test]
    fn the_first_page_has_two_rows_reading_the_current_tracks() {
        let (audio, subs) = item();
        let set = TrackSet { part_id: 9, audio: &audio, subs: &subs };
        let now = chosen(set, None);
        assert_eq!(now, Chosen { audio: 1, sub: None });
        let form = root_form(set, now);
        assert_eq!(rows(&form), 2);
        assert_eq!(audio_label(set, now.audio), "French");
        assert_eq!(subtitle_label(set, now.sub), plx_os::i18n::msg::widgets_tracks_off());
    }

    #[test]
    fn the_pages_list_every_track_and_off() {
        let (audio, subs) = item();
        let set = TrackSet { part_id: 9, audio: &audio, subs: &subs };
        let now = chosen(set, None);
        assert_eq!(rows(&audio_form(set, now)), 2);
        // Off + the two subtitles
        assert_eq!(rows(&subtitle_form(set, now, &[])), 3);
    }

    #[test]
    fn a_subtitle_pick_carries_both_ids_and_off_is_zero() {
        let (audio, subs) = item();
        let set = TrackSet { part_id: 9, audio: &audio, subs: &subs };
        let on = select(set, None, Pick::Subtitle(Some(1))).unwrap();
        assert_eq!((on.audio_sid, on.subtitle_sid), (102, 202), "the audio half is kept, not erased");
        assert!(on.audio_memo.is_none() && on.subtitle_memo.is_some());
        let off = select(set, Some(&on.pick), Pick::Subtitle(None)).unwrap();
        assert_eq!((off.audio_sid, off.subtitle_sid), (102, 0));
        assert_eq!(off.subtitle_memo, Some(SubtitleMemo::Off));
        assert_eq!((on.pick.sub, off.pick.sub), (SubPick::Track(202), SubPick::Off));
        assert_eq!(off.pick.part_id, 9);
        // Off sticks across an audio pick; a never-touched subtitle is not "Off chosen" by one.
        let audio = select(set, Some(&off.pick), Pick::Audio(0)).unwrap();
        assert_eq!(audio.pick.sub, SubPick::Off);
        assert_eq!(select(set, None, Pick::Audio(0)).unwrap().pick.sub, SubPick::Server);
    }

    #[test]
    fn an_audio_pick_changes_only_the_audio_and_a_drill_in_picks_nothing() {
        let (audio, subs) = item();
        let set = TrackSet { part_id: 9, audio: &audio, subs: &subs };
        let sel = select(set, None, Pick::Audio(0)).unwrap();
        assert_eq!((sel.audio_sid, sel.subtitle_sid), (101, 0));
        assert!(sel.audio_memo.is_some() && sel.subtitle_memo.is_none());
        assert!(select(set, None, pick_of(PickRow::OpenAudio)).is_none());
        assert!(select(set, None, Pick::Audio(9)).is_none());
    }

    /// OK on the row `id`, as a host answers it: the menu steps (a drill-in pushes), and a row it picked is then carried
    /// forward and answers what it writes.
    fn ok(p: &mut PickerState, id: PickRow) -> Option<Selection> {
        let key = id.key().0;
        match p.step(Event::Ok(key)) {
            Out::Picked(()) => p.pick(key),
            _ => None,
        }
    }

    /// A picker hung under or above its opener (a Detail page's, with no control to grow from).
    fn sheet(tracks: plx_data::metadata::PreplayTracks, anchor: [u32; 4]) -> PickerState {
        PickerState::new(tracks, Host::Sheet { anchor })
    }

    fn picker() -> PickerState {
        let (audio, subs) = item();
        let tracks = plx_data::metadata::PreplayTracks {
            sid: plx_plex::plex::ServerId::from_raw(0),
            rk: "m1".into(),
            kind: "movie".into(),
            show_rk: String::new(),
            part: "/library/parts/9001/1/f.mkv".into(),
            audio,
            subs,
            pick: None,
        };
        sheet(tracks, [0; 4])
    }

    /// OK on a drill-in pushes (the stack is the page), BACK pops to the root and only then dismisses.
    #[test]
    fn back_pops_a_page_before_it_dismisses() {
        let mut p = picker();
        assert_eq!((p.page(), p.depth()), (PickPage::Root, 0));
        assert_eq!(p.step(Event::Ok(PickRow::OpenSubtitles.key().0)), Out::Moved, "a drill-in pushes");
        assert_eq!((p.page(), p.depth()), (PickPage::Subtitles, 1));
        assert_eq!(p.menu().form().focusable_len(), 3, "Off and the two tracks");
        assert_eq!(p.step(Event::Back), Out::Moved, "BACK pops");
        assert_eq!((p.page(), p.depth()), (PickPage::Root, 0));
        assert_eq!(p.step(Event::Back), Out::Dismiss, "at the root BACK leaves the panel");
    }

    /// A pick carries its choice forward: the page is rebuilt checking it, the next pick is diffed
    /// against it (a pick back to the original writes no memory of the half it did not change), and
    /// the root reads it after the pop.
    #[test]
    fn a_pick_is_carried_forward_and_the_root_reads_it_after_the_pop() {
        let mut p = picker();
        assert_eq!(p.now(), Chosen { audio: 1, sub: None });
        ok(&mut p, PickRow::OpenSubtitles);
        let Some(on) = ok(&mut p, PickRow::Sub(1)) else { panic!("a track row picks") };
        assert_eq!((on.audio_sid, on.subtitle_sid), (102, 202));
        assert_eq!(p.now(), Chosen { audio: 1, sub: Some(1) });
        let Some(off) = ok(&mut p, PickRow::SubOff) else { panic!("Off picks") };
        assert_eq!(off.subtitle_sid, 0);
        assert_eq!(off.subtitle_memo, Some(SubtitleMemo::Off), "diffed against the carried pick, not the server's");
        assert_eq!(p.step(Event::Back), Out::Moved, "BACK pops");
        assert_eq!(subtitle_label(p.core.set(), p.now().sub), plx_os::i18n::msg::widgets_tracks_off());
        assert_eq!(ok(&mut p, PickRow::Audio(9)), None, "an index off the list picks nothing");
    }

    /// Off, close, reopen, audio pick: the pick is the store's one value, so the reopened panel
    /// reads Off and an audio pick keeps it (the plan's account rule then has nothing to turn on).
    #[test]
    fn a_reopened_panel_seeds_from_the_pick_and_an_audio_pick_keeps_off() {
        let mut p = picker();
        ok(&mut p, PickRow::OpenSubtitles);
        let Some(off) = ok(&mut p, PickRow::SubOff) else { panic!("Off picks") };
        let mut tracks = p.tracks().clone();
        tracks.pick = Some(off.pick);
        let mut again = sheet(tracks, [0; 4]);
        assert_eq!(again.now().sub, None);
        ok(&mut again, PickRow::OpenAudio);
        let Some(audio) = ok(&mut again, PickRow::Audio(0)) else { panic!("an audio row picks") };
        assert_eq!((audio.pick.sub, audio.subtitle_sid), (SubPick::Off, 0), "Off survives the audio pick");
    }

    /// The failure screen seeds from the held / standing pick, over the lists' own flags.
    #[test]
    fn a_pick_seeds_the_rows_over_the_servers_flags() {
        let (audio, subs) = item();
        let set = TrackSet { part_id: 9, audio: &audio, subs: &subs };
        let pick = PreplayPick { part_id: 9, audio_sid: 101, sub: SubPick::Track(202) };
        assert_eq!(chosen(set, Some(&pick)), Chosen { audio: 0, sub: Some(1) });
        let other_file = PreplayPick { part_id: 8, audio_sid: 101, sub: SubPick::Track(202) };
        assert_eq!(chosen(set, Some(&other_file)), chosen(set, None), "a pick of another file seeds nothing");
    }

    /// Select Version 2, then pick French: the panel offers version 2's part and streams, the PUT
    /// addresses part 2, and the stored pick lands on version 2's item only (stream ids name the file).
    #[test]
    fn after_select_version_the_pick_addresses_that_versions_part_and_lands_on_its_streams() {
        use plx_data::metadata::{Detail, PlayingItem, VersionPart};
        use plx_data::stores::metadata::MetadataStore;
        use plx_plex::plex::MediaId;
        let _g = plx_base::testlock::serial();
        let sid = plx_plex::plex::ServerId::from_raw(0);
        let v1 = VersionPart {
            id: MediaId(1),
            part: "/library/parts/9001/1/a.mkv".into(),
            audio: vec![stream(11, "English", "eng"), stream(12, "French", "fra")],
            subs: vec![stream(21, "English", "eng")],
        };
        let v2 = VersionPart {
            id: MediaId(2),
            part: "/library/parts/9002/1/b.mkv".into(),
            audio: vec![stream(31, "English", "eng"), stream(32, "French", "fra")],
            subs: vec![stream(41, "English", "eng")],
        };
        let mut store = MetadataStore::default();
        let detail = Detail {
            sid,
            rk: "m1".into(),
            kind: "movie".into(),
            part: v1.part.clone(),
            audio: v1.audio.clone(),
            subs: v1.subs.clone(),
            version_parts: vec![v1.clone(), v2.clone()],
            ..Default::default()
        };
        plx_data::metadata::install_for_test(store.state_mut(), Some(detail));
        let tracks = store.view().preplay_tracks(sid, "m1", Some(MediaId(2))).expect("the chosen version's tracks");
        assert_eq!(plx_media::route::PreplayItem::of(&tracks).part_id, 9002, "the PUT addresses version 2's part");
        assert_eq!(tracks.audio.iter().map(|s| s.id).collect::<Vec<_>>(), [31, 32]);
        let mut p = sheet(tracks, [0; 4]);
        ok(&mut p, PickRow::OpenAudio);
        let Some(sel) = ok(&mut p, PickRow::Audio(1)) else { panic!("an audio row picks") };
        assert_eq!((sel.audio_sid, sel.pick.part_id), (32, 9002));
        assert!(store.run(patch_command(p.tracks(), &sel)));
        let picks = store.view().picks_of(sid, "m1");
        let item = |part: &VersionPart| {
            let mut it = PlayingItem::with_subs(part.subs.clone());
            it.audio = part.audio.clone();
            it.apply_picks(&picks, plx_plex::plex::part_id_of(&part.part));
            it
        };
        let on_v2 = item(&v2);
        assert_eq!(on_v2.audio.iter().position(|s| s.selected), Some(1), "Play plans French on version 2");
        assert!(item(&v1).audio.iter().all(|s| !s.selected), "version 1's streams are not the pick's");
        let reopened = store.view().preplay_tracks(sid, "m1", Some(MediaId(2))).unwrap();
        assert_eq!(reopened.pick, Some(sel.pick));
        assert_eq!(store.view().preplay_tracks(sid, "m1", None).unwrap().pick, None, "another file's panel is not seeded by it");
    }

    /// Failure screen: seeded with the held French pick, the panel reads French checked and picking
    /// English writes the memo (the diff is against the held pick, not the lists' old flags).
    #[test]
    fn a_failure_panel_seeded_with_french_shows_it_checked_and_english_writes_the_memo() {
        let mut p = picker();
        let mut tracks = p.tracks().clone();
        for a in &mut tracks.audio {
            a.selected = a.id == 101;
        }
        tracks.pick = Some(PreplayPick { part_id: 9001, audio_sid: 102, sub: SubPick::Server });
        p = sheet(tracks, [0; 4]);
        assert_eq!(p.now().audio, 1, "French, over the flags' English");
        ok(&mut p, PickRow::OpenAudio);
        let Some(sel) = ok(&mut p, PickRow::Audio(0)) else { panic!("an audio row picks") };
        assert!(sel.audio_memo.is_some(), "English differs from the held French");
        assert_eq!(sel.pick.audio_sid, 101);
    }

    /// **The standalone panel keeps the picker's size class** (HEADLINE bold rows, one per glass): its
    /// root and every page pushed over it, in contrast to the same pages inside an action menu, which take
    /// the menu's compact class.
    #[test]
    fn the_standalone_panel_stays_headline_bold_on_every_page() {
        let (audio, _) = item();
        let tracks = plx_data::metadata::PreplayTracks {
            sid: plx_plex::plex::ServerId::from_raw(0),
            rk: "m1".into(),
            kind: "movie".into(),
            show_rk: String::new(),
            part: "/library/parts/9001/1/f.mkv".into(),
            audio,
            subs: vec![stream(300, "English", "eng")],
            pick: None,
        };
        let headline_bold = (plx_ui::theme::size::HEADLINE, true);
        let mut p = sheet(tracks, [0; 4]);
        assert_eq!(p.menu().form().table.label_style(), headline_bold, "the root");
        ok(&mut p, PickRow::OpenSubtitles);
        assert_eq!(p.menu().form().table.label_style(), headline_bold, "a pushed page");
        assert_eq!(p.step(Event::Back), Out::Moved, "BACK pops");
        assert_eq!(p.menu().form().table.label_style(), headline_bold, "back on the root");
    }

    /// The panel hangs off its opener by one rule, root and pushed page alike: never over the opener,
    /// and the pushed page stays adjacent to the SAME opener rect (above it and growing upward when
    /// it no longer fits below), never detached from it. The failure read-out's button is low on the
    /// screen, the case that was placed from a fallback rect and jumped to the top on a push.
    #[test]
    fn a_pushed_page_stays_anchored_to_its_opener_like_the_root() {
        let measure = plx_ui::fixture::FixtureMeasure;
        let opener = plx_ui::Rect::new(900.0, 770.0, 220.0, 64.0);
        let anchor = [opener.x, opener.y, opener.w, opener.h].map(f32::to_bits);
        let (audio, _) = item();
        let subs: Vec<Stream> = (0..5).map(|i| stream(300 + i, "English", "eng")).collect();
        let tracks = plx_data::metadata::PreplayTracks {
            sid: plx_plex::plex::ServerId::from_raw(0),
            rk: "m1".into(),
            kind: "movie".into(),
            show_rk: String::new(),
            part: "/library/parts/9001/1/f.mkv".into(),
            audio,
            subs,
            pick: None,
        };
        let mut p = sheet(tracks, anchor);
        let root = p.panel_rect(&measure);
        ok(&mut p, PickRow::OpenSubtitles);
        let pushed = p.panel_rect(&measure);
        let clear_of = |r: plx_ui::Rect| r.y + r.h <= opener.y || r.y >= opener.y + opener.h;
        assert!(clear_of(root) && clear_of(pushed), "never over the opener: {root:?} {pushed:?}");
        let h = p.menu().form().table.measured_height();
        let want = plx_ui::popover::under_or_above_fit(opener, p.menu().form().table.menu_panel_width(&measure), h);
        assert_eq!(pushed, want, "the pushed page is placed by the one anchor rule, off the opener's real rect");
        assert!(
            (pushed.y + pushed.h - opener.y).abs() <= 64.0 || (pushed.y - (opener.y + opener.h)).abs() <= 64.0,
            "the pushed page touches the opener's edge, it did not jump away: {pushed:?}"
        );
    }
}
