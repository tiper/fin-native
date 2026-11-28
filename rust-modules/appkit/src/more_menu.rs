//! The player transport's **overflow menu** — the popover behind the third control disc (`…`), on
//! the same animated [`TableView`] as the subtitle/audio and profile menus. It only REPORTS the
//! chosen [`Action`]; `app.rs` performs it, exactly as the profile menu did before it became
//! an owned surface (`crate::screens::account_menu`, restructure phase 10).
//!
//! # Why an overflow menu exists at all
//!
//! **Stats for nerds**, the diagnostics overlay (`crate::app::diagnostics`), needs a home a stranger can
//! find, because it is how this app gets bug reports off televisions nobody here owns. Stats for
//! nerds and the opt-in playback diagnostics in Settings ship in release builds. Developer triggers,
//! the remote FIFO and capture stream require the `devtriggers` feature. "Press `…`, turn Stats
//! for nerds on, photograph the screen" is a sentence that
//! fits in a GitHub reply and needs no ssh, no root and no rebuild.
//!
//! It held that one row for a while, and a menu with one row is not a mistake either: the
//! alternative — hanging the toggle off a hidden key chord — is undiscoverable by exactly the
//! people who would report the bug, and the alternative to THAT is a fourth disc for a control most
//! users touch once. Overflow is what a `…` means.
//!
//! # A root with a Quality row, a Quality page, and the two row idioms they are drawn in
//!
//! **Quality leads.** It is the primary playback control this popover exists to reach — a rung
//! picked here re-routes the picture that is on screen right now (see below). On the ROOT it is one
//! headerless drill-in row ([`MoreRow::OpenQuality`]) whose value is the choice then what is playing ("Original · Converted", [`quality_readout`]);
//! RIGHT or OK pushes the **Quality page** (a title band, then the rungs). **Options** trails it on
//! the root: the diagnostics switch is an overflow affordance, something a viewer reaches for once,
//! to photograph a bug, not a control anyone returns to.
//!
//! **Playback speed** follows Quality, in the same idiom: a drill-in row reading out the current
//! speed, onto a page of Plex Web's speeds (0.5× to 2×) with the current one checked — those this
//! playback can play ([`plx_media::player::speed_offer`]), and no row at all where that is only 1x.
//! It is offered under Force Direct Play too, where it is the root's first row, because a speed changes
//! how fast the picture plays and not what the server sends. A pick reloads the stream at the
//! viewer's position, with the picture retimed and the sound time-stretched by the app
//! ([`plx_media::player::set_play_rate`]).
//!
//! **Quality** (the page) is the [`plx_media::route::Quality`] ladder — Original, fixed rungs, and Auto once its
//! playback readiness gate opens — and its rows carry
//! [`Row::checked`]'s LEADING checkmark, which means "the active one of several". That is the same
//! design-system rule from the other side: **a mark says where you are and a word says what is set,
//! and no row says both**, which is why a rung's rate rides inside its own label rather than in a
//! trailing value beside the mark. (The Options row drew as a PAIR OF MARKS for one day, a ring
//! ticked when on; those assets were deleted the same evening — see [`plx_ui::icons`].)
//!
//! **Options** holds switches. Its row carries [`Row::toggle`], so it states itself as the WORD
//! `On`/`Off` at the row's trailing edge. It is a STATE, not a destination: a chevron would promise
//! a page behind the row and there is none.
//!
//! **Folded rows** sit between Speed and Options on a root whose HUD package places a command in no reachable
//! state (`docs/hud-plugins.md`, R2): Previous, Next, the track menu, the two sections and the Info actions, each
//! listed only when the item playing can use it ([`folded_rows`]). They are headerless buttons that report
//! [`Action::Hud`], and the container closes the menu and hands the command to the app's one performer. Classic
//! folds nothing, so its root has none.
//!
//! `docs/parity-gaps.md`'s standing decision is that this app has **no full-screen menu sheets** —
//! the reference clients put playback quality in one and we do not. Quality drills in INSIDE the
//! popover instead, on the same [`plx_ui::menu::Menu`] the Subtitles tab of
//! [`crate::track_menu`] stands on: BACK, LEFT and a click on the title band mean "up one page", and
//! only BACK on the root dismisses. Six rungs fit; when they stop fitting, the [`TableView`]
//! scrolls, which is what it is for.
//!
//! A rung closes the menu on commit (opening the page does not), so the read-out is never what confirms the press: the
//! overlay appearing behind the dismissed panel — or, for a rung, the next play routing differently
//! — is.
//!
//! # What a picked rung does, and what it deliberately does not
//!
//! It is a ROUTING policy, not a number handed to the transcoder: over-ceiling content loses direct
//! play *and* the container remux, which is the only way a cap can bind at all. Original preserves
//! the source unchanged. Auto is exposed only through `route::auto_quality_ready()`, the named
//! fail-closed gate owned by the integrated HLS prime/swap path. The whole argument is
//! [`plx_media::route::Quality`]'s doc.
//!
//! It binds every future play, **and it re-decides the one on screen** — because this menu is the
//! ladder's only entry point, so a rung that waited for the next play would be a control that
//! visibly does nothing everywhere it can be reached. `route::set_quality` re-asks the routing
//! question with the new rung and reloads only when the answer changed; picking a HIGHER rung than
//! the picture already satisfies does nothing at all. That is a user-initiated switch and not an
//! adaptive one — nothing measures a link or moves a rung on its own.
#![allow(non_upper_case_globals)]
use plx_ui::a11y::{A11y, A11yPath};
use plx_ui::geom::IndexElem;
use plx_machine::machine::GroupId;
use plx_ui::form::{Form, FormId, FormSection, RowKey, RowKind};
use crate::player_hud::command::HudCommand;
use plx_ui::menu::{Class, Event, Host, Menu, Out, Page, Shade};
use plx_ui::page_stack::{popover_a11y, popover_a11y_elems};
use plx_ui::table::Row;
use plx_ui::Rect;

/// What the highlighted row does on OK.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    None,
    /// flip `crate::app::diagnostics`'s overlay on/off
    ToggleStats,
    /// select a rung of the playback-quality ladder ([`plx_media::route::set_quality`])
    SetQuality(plx_media::route::Quality),
    /// select a playback speed, in thousandths (one of [`plx_media::player::PLAY_RATES`];
    /// [`plx_media::player::set_play_rate`])
    SetSpeed(i32),
    /// **Lab builds only** — snapshot and upload the diagnostic ring (`crate::lab`). Here as well
    /// as in `account_menu` because the account menu is unreachable during playback, and playback
    /// is what a Cloud Test Lab session is usually reproducing.
    SendDiagnostics,
    /// **A folded row**: a command the HUD package places in no reachable state, listed here so the viewer
    /// can still reach it (`docs/hud-plugins.md`, R2). The row is the same [`HudCommand`] a placed control
    /// would carry, and the container hands it to the app's one performer (`PlayerReq::Command`,
    /// `app::playback::perform_hud_command`) after closing the menu, so a row can never mean something a
    /// control does not. Never produced by [`rows_for`]: [`FoldedRow`]s carry it.
    Hud(HudCommand),
}

/// **One row of More's root for a folded command**, with the label it reads: the label is chosen in
/// [`folded_rows`] from the playing item's metadata through `info_panel::actions`, since `go_to_detail` reads Go to
/// Show for an episode and Go to Movie otherwise.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FoldedRow {
    pub command: HudCommand,
    pub label: &'static str,
}

/// **The folded rows to list for the content playing**, from the compiled package's folds
/// ([`crate::hud_format::compile::Compiled::folds`], R2) in the order the compiler reports. A row is listed
/// only when the app's own predicate for its command holds for this frame's content
/// ([`HudCommand::enabled`]): a movie gets no Previous or Next row rather than a dead one. `open_tracks`
/// folds as one row, which opens the track menu on its Subtitles tab (the menu holds both tabs).
pub fn folded_rows(
    folds: &[crate::hud_format::Fold],
    model: &crate::player_hud::model::HudModel,
    meta: plx_data::metadata::MetadataView<'_>,
) -> Vec<FoldedRow> {
    use crate::player_hud::command::{Section, TrackTab};
    use plx_os::i18n::msg;
    folds
        .iter()
        .filter_map(|fold| {
            let (command, label) = match (fold.command, fold.section) {
                ("previous", _) => (HudCommand::Previous, msg::widgets_menu_previous_episode()),
                ("next", _) => (HudCommand::Next, msg::widgets_next_episode()),
                ("open_tracks", _) => (HudCommand::OpenTracks(TrackTab::Subtitles), msg::settings_audio_title()),
                ("toggle_section", Some("chapters")) => (HudCommand::ToggleSection(Section::Chapters), msg::widgets_player_chapters()),
                ("toggle_section", Some("info")) => (HudCommand::ToggleSection(Section::Info), msg::widgets_player_info()),
                ("from_beginning", _) => (HudCommand::FromBeginning, crate::info_panel::actions(meta)[0]),
                ("go_to_detail", _) => (HudCommand::GoToDetail, crate::info_panel::actions(meta)[1]),
                _ => return None,
            };
            command.enabled(model).then_some(FoldedRow { command, label })
        })
        .collect()
}

/// **A drill-in page of the More menu** — the form's `Dest`. The root is the empty page stack, not
/// a value of this type (`docs/player-submenus.md`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MorePage {
    /// The playback-quality ladder: one checked-rung row per rung ([`MoreRow::Act`] of a
    /// [`Action::SetQuality`]).
    Quality,
    /// The playback speeds, slowest first, Normal checked by default ([`MoreRow::Act`] of a
    /// [`Action::SetSpeed`]).
    Speed,
}

impl MorePage {
    /// The title band's text.
    fn title(self) -> String {
        match self {
            Self::Quality => plx_os::i18n::msg::widgets_menu_quality().to_string(),
            Self::Speed => plx_os::i18n::msg::widgets_menu_speed().to_string(),
        }
    }

    /// A stable small number for the replay canon. Never reordered: recordings hash it.
    fn code(self) -> u32 {
        match self {
            Self::Quality => 1,
            Self::Speed => 2,
        }
    }
}

/// A row's identity on either page: the Quality drill-in on the root, or a row that commits an
/// [`Action`] (a rung on the Quality page, an Options switch on the root).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoreRow {
    /// The root's Quality row ([`MorePage::Quality`]): reads out the choice and what is playing.
    OpenQuality,
    /// The root's Playback speed row ([`MorePage::Speed`]): reads out the current speed.
    OpenSpeed,
    Act(Action),
}

type MoreForm = Form<MoreRow, Action, MorePage>;

/// The menu's whole state, owned by the container that mounts this panel — the modal PHASE and the
/// appear spring belong to `ui::containers::modal::ModalStack` now, not to this struct. The form, the stack of
/// pages, the card and its motion are the one menu component's ([`plx_ui::menu::Menu`], `Host::Video`: flat over the
/// picture); what stays here is what only this menu knows, the playback read-outs its pages are built from.
/// The panel's width is the shared menu rule (`TableView::menu_panel_width`): it hugs the widest
/// row, capped at `MENU_MAX_W`, and every row must fit the cap in every language
/// (`every_row_fits_the_panel_in_every_language`).
pub struct MoreMenuState {
    /// What the pages are built from ([`MoreCore::build`]); apart from the menu so the builder borrows it while the
    /// menu steps.
    core: MoreCore,
    /// The rows AND the action each one commits, declared together ([`root_form`], [`quality_form`]) — the row SET
    /// varies (the Quality ladder is built from `route::available_quality_ladder`, and Force Direct Play drops it), so a
    /// row is found by its identity ([`MoreRow`]'s [`FormId`] key), never by a position. One table serves the page that is
    /// showing. The table is main-thread-only, like every other panel's.
    menu: Menu<MorePage, MoreRow, Action>,
}

/// **What More's pages are built from.**
struct MoreCore {
    /// The row set the form was last built from (what [`rows_for`] answered), so a live change to
    /// it — Auto's readiness gate opening, Force Direct Play flipping — is noticed and the panel
    /// rebuilt and RESIZED ([`MoreMenuState::refresh`]) rather than left showing a ladder that has moved.
    rows: Vec<Action>,
    forced: bool,
    /// The rung the root's Quality row reads out and the Quality page checks, as of the last build.
    current: plx_media::route::Quality,
    /// The speed the root's Playback speed row reads out and the Speed page checks, as of the last
    /// build ([`plx_media::player::play_rate_permille`]).
    speed: i32,
    /// The speeds this playback offers ([`plx_media::player::speed_offer`]); empty hides the row.
    speeds: Vec<i32>,
    /// The folded rows listed between Speed and Options ([`folded_rows`]), fixed when the menu opened: a
    /// command's predicate holds for the item playing, which does not change under an open menu.
    folded: Vec<FoldedRow>,
}

/// **The tallest page this menu shows with app-owned rows** — the Quality page, its title band,
/// seven rungs (Auto first) and the table's padding: 536 px in English. It decides the SIDE of the
/// opener the panel opens on (`plx_ui::popover::anchor_panel`'s `need_h`) for a root with up to three folded
/// rows; [`need_h`] decides it beyond that. The number is the same on every page. 600 leaves room for a longer label that wraps. Classic's room above its row is 710,
/// which this stays under, so classic opens upward. `pub(crate)` because the HUD lint places this
/// menu with it (`hud_format::lint_menus`, M2), placed by the same placement ([`plx_ui::popover::Placement::PLAYER_MENU`]).
pub(crate) const NEED_H: f32 = 600.0;

/// **The height the root asks of its room when it lists `folded` rows**: the root at its tallest standard shape
/// (Quality and Speed, Options with Stats for nerds and Send diagnostics) plus the folded section, which is
/// [`FOLDED_SECTION_H`] and a [`FOLDED_ROW_H`] per row. Never below [`NEED_H`], the tallest page, so a menu with
/// no folded rows (classic) asks what it always asked and opens where it always opened. A fixed function of
/// the count, not of what is on screen: the side must not change between pages or with the rows a language or
/// a playback happens to offer, and the HUD lint (`hud_format::lint_menus`, M2) asks the same function for
/// the most rows a package can fold. `root_height_is_the_tables` pins the three numbers to the real table.
pub(crate) fn need_h(folded: usize) -> f32 {
    let root = if folded == 0 { ROOT_BASE_H } else { ROOT_BASE_H + FOLDED_SECTION_H + FOLDED_ROW_H * folded as f32 };
    NEED_H.max(root)
}

/// The root's height with Quality, Speed, Stats for nerds and Send diagnostics and no folded row.
const ROOT_BASE_H: f32 = 344.0;
/// One folded row: a compact table row.
const FOLDED_ROW_H: f32 = 60.0;
/// The gap the folded section adds between Speed and Options.
const FOLDED_SECTION_H: f32 = 24.0;

/// The hand-assigned focus key of each row. `SetQuality` rungs are an exhaustive match, so a new
/// rung cannot compile without claiming a key; none of these is a discriminant, a position or a
/// hash, so reordering the menu moves no key.
impl FormId for Action {
    fn key(&self) -> RowKey {
        use plx_media::route::Quality;
        RowKey(match self {
            Action::None => 0,
            Action::ToggleStats => 1,
            Action::SendDiagnostics => 2,
            // 30..=43: the folded commands, one key each (the four a package can never fold are listed so a
            // new command cannot compile without claiming one)
            Action::Hud(command) => match command {
                HudCommand::Previous => 30,
                HudCommand::Next => 31,
                HudCommand::OpenTracks(_) => 32,
                HudCommand::ToggleSection(crate::player_hud::command::Section::Chapters) => 33,
                HudCommand::ToggleSection(crate::player_hud::command::Section::Info) => 34,
                HudCommand::FromBeginning => 35,
                HudCommand::GoToDetail => 36,
                HudCommand::TogglePlay => 40,
                HudCommand::Seek(_) => 41,
                HudCommand::OpenMore(_) => 42,
                HudCommand::ToggleStats => 43,
            },
            Action::SetQuality(Quality::Auto) => 10,
            Action::SetQuality(Quality::Original) => 11,
            Action::SetQuality(Quality::P1080High) => 12,
            Action::SetQuality(Quality::P1080) => 13,
            Action::SetQuality(Quality::P720) => 14,
            Action::SetQuality(Quality::P720Low) => 15,
            Action::SetQuality(Quality::P480) => 16,
            // 20..=26 by the speed's place in the ladder; a value off the ladder is never offered
            Action::SetSpeed(r) => {
                20 + plx_media::player::PLAY_RATES.iter().position(|x| *x == *r).map_or(9, |i| i as u32)
            }
        })
    }
}

impl FormId for MoreRow {
    fn key(&self) -> RowKey {
        match self {
            // free in `Action`'s key space (0..=2, 10..=16 and 20..=29)
            MoreRow::OpenQuality => RowKey(3),
            MoreRow::OpenSpeed => RowKey(4),
            MoreRow::Act(a) => a.key(),
        }
    }
}

impl MoreRow {
    /// **The row's name in the menu's accessibility ids** (`plx_ui::a11y`), relative to the menu: one
    /// word per row IDENTITY, never a position, so the Quality rungs and the speeds keep their ids
    /// when the offered set changes. The root's drill-ins are `quality` and `speed`, the switches
    /// `stats` and `diagnostics`, a folded command `fold.<command>`; the Quality page's rungs are
    /// `rung.<name>` and the Speed page's `rate.<thousandths>` (1500 is 1.5x). A page shows one of
    /// these sets at a time, so no two rows of a view share an id. An exhaustive match, so a new
    /// rung or command cannot compile without a name, as it cannot without a focus key.
    fn a11y_path(&self) -> A11yPath {
        use plx_media::route::Quality;
        let path = A11yPath::new();
        match self {
            MoreRow::OpenQuality => path.name("quality"),
            MoreRow::OpenSpeed => path.name("speed"),
            MoreRow::Act(action) => match action {
                Action::None => path.name("none"),
                Action::ToggleStats => path.name("stats"),
                Action::SendDiagnostics => path.name("diagnostics"),
                Action::SetQuality(quality) => path.name("rung").name(match quality {
                    Quality::Auto => "auto",
                    Quality::Original => "original",
                    Quality::P1080High => "p1080_high",
                    Quality::P1080 => "p1080",
                    Quality::P720 => "p720",
                    Quality::P720Low => "p720_low",
                    Quality::P480 => "p480",
                }),
                Action::SetSpeed(thousandths) => path.name("rate").index(usize::try_from(*thousandths).unwrap_or(0)),
                Action::Hud(command) => path.name("fold").name(match command {
                    HudCommand::Previous => "previous",
                    HudCommand::Next => "next",
                    HudCommand::OpenTracks(_) => "tracks",
                    HudCommand::ToggleSection(crate::player_hud::command::Section::Chapters) => "chapters",
                    HudCommand::ToggleSection(crate::player_hud::command::Section::Info) => "info",
                    HudCommand::FromBeginning => "from_beginning",
                    HudCommand::GoToDetail => "go_to_detail",
                    HudCommand::TogglePlay => "play_pause",
                    HudCommand::Seek(_) => "seek",
                    HudCommand::OpenMore(_) => "more",
                    HudCommand::ToggleStats => "stats",
                }),
            },
        }
    }
}

/// Whether the Quality section is withheld for this session: under Force Direct Play (no rung can change what plays) and
/// when no composed backend can carry a rung (no Feed, and the URI provider certifies no converted output:
/// [`plx_media::route::PlaybackSession::quality_pick_carriable`]), where a pick would persist a ceiling a playing original
/// cannot honour. `forced` below is this answer.
fn quality_withheld(ps: &plx_media::route::PlaybackSession) -> bool {
    plx_media::route::forced_direct_play(ps) || !ps.quality_pick_carriable()
}

/// Is the Quality row offered? Force Direct Play never offers one (no rung can change what plays),
/// and an empty ladder has nothing to open onto — a row leading to an empty page would read as a
/// menu that failed to load.
fn quality_offered(rows: &[Action], forced: bool) -> bool {
    !forced && rows.iter().any(|a| matches!(a, Action::SetQuality(_)))
}

/// The ROOT as a pure form: the Quality and Playback speed drill-ins (headerless, like the
/// Subtitles root's Style — each reads out its current choice as its value, Quality also what is
/// playing), then Options.
fn root_form(
    ps: &plx_media::route::PlaybackSession,
    rows: &[Action],
    forced: bool,
    current: plx_media::route::Quality,
    speed: i32,
    speeds: &[i32],
    folded: &[FoldedRow],
) -> MoreForm {
    let mut options = FormSection::new(plx_os::i18n::msg::widgets_menu_options());
    for a in rows.iter().copied().filter(|a| !matches!(a, Action::SetQuality(_))) {
        options = options.item(MoreRow::Act(a), RowKind::Button, a, row_for(ps, a));
    }
    let quality = FormSection::new("").item_if(
        quality_offered(rows, forced),
        MoreRow::OpenQuality,
        RowKind::Nav(MorePage::Quality),
        Action::None,
        Row::new(plx_os::i18n::msg::widgets_menu_quality()).value(quality_readout(ps, current)),
    )
    // Offered under Force Direct Play too: a speed changes how fast the picture plays, not what
    // the server sends. Not offered where this playback cannot play any speed but 1x
    // (`player::speed_offer`): a row that cannot change the outcome is not shown.
    .item_if(
        speeds.len() > 1,
        MoreRow::OpenSpeed,
        RowKind::Nav(MorePage::Speed),
        Action::None,
        Row::new(plx_os::i18n::msg::widgets_menu_speed()).value(speed_label(speed)),
    );
    let mut form = Form::new().section(quality);
    // The commands the package places nowhere: after Quality and Speed, before Options, in the compiler's order.
    // No section at all for none, so a package that folds nothing (classic) builds the root it always did.
    if !folded.is_empty() {
        let rows = folded.iter().fold(FormSection::new(""), |section, row| {
            let act = Action::Hud(row.command);
            section.item(MoreRow::Act(act), RowKind::Button, act, Row::new(row.label))
        });
        form = form.section(rows);
    }
    form.section(options)
}

/// The Speed PAGE: one checked row per offered speed (Plex Web's seven, in its order, less any this
/// source cannot play — `player::speed_offer`), the current one checked.
fn speed_form(ps: &plx_media::route::PlaybackSession, speed: i32, speeds: &[i32]) -> MoreForm {
    let sec = speeds.iter().map(|r| Action::SetSpeed(*r)).fold(
        FormSection::new(""),
        |sec, a| sec.choice(MoreRow::Act(a), a, row_for(ps, a), |id| *id == MoreRow::Act(Action::SetSpeed(speed))),
    );
    Form::new().section(sec)
}

/// A speed's name: `Normal` at 1x, else the multiplier (`plx_media::player::rate_name`).
fn speed_label(permille: i32) -> String {
    if permille == plx_media::player::RATE_NORMAL {
        return plx_os::i18n::msg::widgets_speed_normal().to_string();
    }
    plx_media::player::rate_name(permille)
}

/// **The Quality row's value: "{choice} · {playing}".** The first word is what was picked; the
/// second says what is actually playing and is left out only when it would repeat the first. Auto
/// always shows its result. A fixed rung reads by its SHORT name ("720p"; the page keeps the full
/// "720p · 4 Mbps"). While the stream is unsettled the choice stands alone. Read when the popover
/// opens or rebuilds, never while it is on screen.
fn quality_readout(ps: &plx_media::route::PlaybackSession, current: plx_media::route::Quality) -> String {
    use plx_media::route::{PlayingQuality, Quality};
    let joined = |second: String| format!("{} \u{b7} {second}", current.short_label());
    match (current, plx_media::route::playing_quality(ps)) {
        (Quality::Original | Quality::Auto, PlayingQuality::Converted) => {
            joined(plx_os::i18n::msg::widgets_menu_quality_converted().to_owned())
        }
        (Quality::Auto, PlayingQuality::Original) => joined(Quality::Original.label()),
        (Quality::Auto, PlayingQuality::Rung(lines)) => joined(format!("{lines}p")),
        _ => current.short_label(),
    }
}

/// Does the checked Quality entry carry the "your server is converting this video" sub-line? Only
/// when the row reports a conversion the viewer did not pick by name: "Original · Converted" and
/// "Auto · 720p".
fn quality_conversion_note(ps: &plx_media::route::PlaybackSession, current: plx_media::route::Quality) -> bool {
    use plx_media::route::{PlayingQuality, Quality};
    matches!(
        (current, plx_media::route::playing_quality(ps)),
        (Quality::Original | Quality::Auto, PlayingQuality::Converted) | (Quality::Auto, PlayingQuality::Rung(_))
    )
}

/// The Quality PAGE: one checked-rung row per rung of `rows`, the current one checked.
fn quality_form(
    ps: &plx_media::route::PlaybackSession,
    rows: &[Action],
    current: plx_media::route::Quality,
) -> MoreForm {
    let sec = rows.iter().copied().filter(|a| matches!(a, Action::SetQuality(_))).fold(
        FormSection::new(""),
        |sec, a| {
            let mut row = row_for(ps, a);
            if a == Action::SetQuality(current) && quality_conversion_note(ps, current) {
                row = row.detail(plx_os::i18n::msg::widgets_menu_quality_converted_note());
            }
            sec.choice(MoreRow::Act(a), a, row, |id| {
                *id == MoreRow::Act(Action::SetQuality(current))
            })
        },
    );
    Form::new().section(sec)
}

impl MoreCore {
    /// **The page builder of the one menu component** ([`plx_ui::menu::Build`]): `None` asks for the root. A page that
    /// is not offered for this playback (Quality under Force Direct Play or with an empty ladder, Speed when the playback
    /// takes one speed only) cannot be built, which is how a live change that takes the page away under the viewer drops
    /// it back to the root ([`Event::Refresh`]).
    fn build(&self, ps: &plx_media::route::PlaybackSession, page: Option<MorePage>) -> Option<Page<MoreRow, Action, MorePage>> {
        match page {
            None => Some(Page::root(root_form(ps, &self.rows, self.forced, self.current, self.speed, &self.speeds, &self.folded))),
            Some(MorePage::Quality) => quality_offered(&self.rows, self.forced).then(|| {
                Page::new(quality_form(ps, &self.rows, self.current), MorePage::Quality.title())
                    .initial(Some(MoreRow::Act(Action::SetQuality(self.current))))
            }),
            Some(MorePage::Speed) => (self.speeds.len() > 1).then(|| {
                Page::new(speed_form(ps, self.speed, &self.speeds), MorePage::Speed.title())
                    .initial(Some(MoreRow::Act(Action::SetSpeed(self.speed))))
            }),
        }
    }
}

/// **The More menu's card over the video plane**: flat, hung from the control row by the player's placement, at the
/// sheet's dim weight (the player's Tracks menu is darker). The anchor is the rect the host names, the segment's
/// ([`MoreMenuState::new_folded_with_quality`]); every page opens on the side the root's tallest shape holds
/// ([`need_h`] of the `folded` rows it lists).
fn host(anchor: Rect, folded: usize) -> Host {
    Host::Video {
        anchor: bits(anchor),
        placement: plx_ui::popover::Placement::PLAYER_MENU,
        shade: Shade::Sheet,
        need_h: Some(need_h(folded)),
    }
}

fn bits(r: Rect) -> [u32; 4] {
    [r.x, r.y, r.w, r.h].map(f32::to_bits)
}

impl MoreMenuState {
    fn open_focused(
        ps: &plx_media::route::PlaybackSession,
        page: Option<MorePage>,
        folded: Vec<FoldedRow>,
        current: plx_media::route::Quality,
        anchor: Rect,
    ) -> Self {
        let forced = quality_withheld(ps);
        let rows = rows_for(forced);
        let speed = plx_media::player::selected_rate(ps).unwrap_or(plx_media::player::RATE_NORMAL);
        let speeds = plx_media::player::speed_offer(ps);
        // a short action list: BODY labels, like the profile menu
        let mut menu = Menu::new(host(anchor, folded.len()), Class::Action, plx_ui::table_screen::BAND_BASE);
        let core = MoreCore { rows, forced, current, speed, speeds, folded };
        menu.step(Event::ReplaceRoot { slide: None }, &mut |p| core.build(ps, p));
        let mut st = MoreMenuState { core, menu };
        // A page entry opens straight on that page, no slide. Quality (the failure screen's OK)
        // lands on the active rung: the viewer is fixing a bad decision, not browsing. A page that
        // is not offered (no ladder under Force Direct Play, no choice of speed) is the ordinary
        // root, as the page-vanished refresh does. The page is entered from its opener's row, so
        // BACK lands where a viewer would have pressed to get here.
        if let Some(page) = page {
            st.step(ps, Event::Enter(match page {
                MorePage::Quality => MoreRow::OpenQuality,
                MorePage::Speed => MoreRow::OpenSpeed,
            }));
        }
        st
    }

    /// **The one entry of a key, a click or a focus move** ([`Event`]): the screen feeds the panel what the viewer did and
    /// acts on the answer ([`Out`]): OK (or a click) on a row picks its action or opens its page; BACK and LEFT pop a page
    /// and BACK at the root asks to close; RIGHT enters a Nav row (any other answer is [`Out::Edge`], which this menu, with
    /// no tabs to switch, leaves alone); the title band's key pops. The pages are built from the panel's own read-outs.
    pub fn step(&mut self, ps: &plx_media::route::PlaybackSession, ev: Event<'_, MoreRow>) -> Out<Action> {
        let core = &self.core;
        self.menu.step(ev, &mut |p| core.build(ps, p))
    }

    /// **The one menu component** this panel is: the form, the stack of pages, the card and its motion
    /// ([`plx_ui::menu::Menu`]), for the host to draw, to prewarm and to ask for focus.
    pub fn menu(&self) -> &Menu<MorePage, MoreRow, Action> {
        &self.menu
    }

    /// The page showing, `None` at the root.
    #[cfg(any(test, feature = "test-support"))]
    pub fn page(&self) -> Option<MorePage> {
        self.menu.top()
    }

    /// The highlighted row's identity, for tests.
    #[cfg(any(test, feature = "test-support"))]
    pub fn sel_id(&self) -> Option<MoreRow> {
        self.menu.form().selected_id().copied()
    }

    /// **The replay canon**: the page path (each pushed page and the row that opened it) and the
    /// selected row's [`RowKey`] — not its index, which two pages share.
    pub fn canon(&self, c: &mut plx_machine::machine::Canon) {
        self.menu.stack_canon(c, MorePage::code, |r| r.key().0);
        c.u32(self.menu.form().key_at(self.menu.form().table.sel.max(0) as usize).map_or(u32::MAX, |k| k.0));
    }

    /// What the `moreosc` trigger needs to choose its next key: how many pages are pushed.
    pub fn osc_depth(&self) -> usize {
        self.menu.depth()
    }

    /// **Follow a live change of the row set or the current rung**: rebuild the page that is
    /// showing when [`rows_for`], Force Direct Play or the current rung no longer answer what it
    /// was built from. On the root the form is rebuilt keeping the focused row by identity; on the
    /// Quality page it is refreshed IN PLACE (focus kept by id, the checked rung following the
    /// pick), and the page pops to the root when Quality stops being offered. The panel's height
    /// follows, and the card animates to it ([`Self::update`]). Returns whether it rebuilt.
    pub fn refresh(&mut self, ps: &plx_media::route::PlaybackSession) -> bool {
        self.refresh_with_quality(ps, self.core.current)
    }

    pub fn refresh_with_quality(&mut self, ps: &plx_media::route::PlaybackSession, current: plx_media::route::Quality) -> bool {
        let forced = quality_withheld(ps);
        self.refresh_to(
            ps,
            forced,
            rows_for(forced),
            current,
            plx_media::player::selected_rate(ps).unwrap_or(plx_media::player::RATE_NORMAL),
            plx_media::player::speed_offer(ps),
        )
    }

    /// [`Self::refresh`] against explicit answers (what a test or a probe supplies in place of the
    /// live route).
    fn refresh_to(
        &mut self,
        ps: &plx_media::route::PlaybackSession,
        forced: bool,
        rows: Vec<Action>,
        current: plx_media::route::Quality,
        speed: i32,
        speeds: Vec<i32>,
    ) -> bool {
        let core = &mut self.core;
        if forced == core.forced && rows == core.rows && current == core.current && speed == core.speed && speeds == core.speeds {
            return false;
        }
        (core.speeds, core.rows, core.forced, core.current, core.speed) = (speeds, rows, forced, current, speed);
        self.step(ps, Event::Refresh { keep: None });
        true
    }

    pub fn new(ps: &plx_media::route::PlaybackSession) -> Self {
        Self::new_on(ps, None)
    }

    /// The existing overflow menu, opened on the Quality page with the ACTIVE rung focused.
    ///
    /// The terminal playback screen has no transport discs, so OK must enter the ladder on the rung
    /// that is actually playing — a viewer arriving here is fixing a bad decision, not browsing the
    /// list. It is still the SAME menu, form and action map as the ordinary `…` menu; only the page
    /// it opens on differs (BACK pops to the root, as from any pushed page).
    ///
    /// Under Force Direct Play the menu has no Quality section (see [`rows_for`]), and this is the
    /// same menu as [`Self::new`]; `screens::player::overlay` does not route here then.
    pub fn new_quality(ps: &plx_media::route::PlaybackSession) -> Self {
        Self::new_on(ps, Some(MorePage::Quality))
    }

    /// The menu opened on `page`: the root for `None`, else the drill-in, focused on its checked
    /// row. A page that is not offered for this playback (Quality under Force Direct Play, Speed
    /// when the playback takes only one) opens the root instead.
    pub fn new_on(ps: &plx_media::route::PlaybackSession, page: Option<MorePage>) -> Self {
        Self::open_focused(ps, page, Vec::new(), plx_media::route::Quality::Original, crate::player_hud::classic::ctrl_row_hit_rect())
    }

    /// [`Self::new_on`] with the folded rows ([`folded_rows`]) listed on the root between Speed and Options.
    /// A page opened directly still pops to that root, so its folded rows are there on BACK.
    pub fn new_folded(ps: &plx_media::route::PlaybackSession, page: Option<MorePage>, folded: Vec<FoldedRow>) -> Self {
        Self::open_focused(ps, page, folded, plx_media::route::Quality::Original, crate::player_hud::classic::ctrl_row_hit_rect())
    }

    /// Every row's focus key, in drawn order (for a test that walks the rows).
    #[cfg(any(test, feature = "test-support"))]
    pub fn keys(&self) -> Vec<u32> {
        let form = self.menu.form();
        (0..form.table.n_rows() as usize).filter_map(|i| form.key_at(i).map(|k| k.0)).collect()
    }

    /// The highlighted row, for the focus probe (`crate::focusprobe`) — a READ of the cursor the
    /// key ladder moves, and the reason it exists: `app.rs`'s UP/DOWN arm for this panel changes
    /// nothing else, so without this the fingerprint records the panel opening and closing and
    /// nothing between.
    pub fn sel(&self) -> i32 {
        self.menu.form().table.sel
    }

    /// Construction from the host's captured effective quality, never a process preference read, and the rect `anchor`
    /// of the control (widened to the end of its segment) the menu hangs from, as the player screen hands it over: the
    /// openers of one segment share the panel's edge, and opening one menu after the other does not make the panel hop.
    /// The anchor is the host's for the menu's whole life ([`Host::Video`]).
    pub fn new_folded_with_quality(
        ps: &plx_media::route::PlaybackSession,
        page: Option<MorePage>,
        folded: Vec<FoldedRow>,
        current: plx_media::route::Quality,
        anchor: Rect,
    ) -> Self {
        Self::open_focused(ps, page, folded, current, anchor)
    }

    pub fn update_with_quality(&mut self, dt: f32, measure: &dyn plx_machine::machine::Measure, ps: &plx_media::route::PlaybackSession, current: plx_media::route::Quality) {
        self.refresh_with_quality(ps, current);
        self.update(dt, measure, ps);
    }

    pub fn update(&mut self, dt: f32, measure: &dyn plx_machine::machine::Measure, ps: &plx_media::route::PlaybackSession) {
        self.refresh(ps);
        // the cursor is this panel's own: it is not read back from the focus engine here
        self.step(ps, Event::Tick { dt, measure, focus: None });
    }
}

impl MoreMenuState {
    /// **What a press of the row `key` performs**, for a row that performs one (the Quality and Speed drill-ins
    /// open a page and have none): the row's [`Action`], read off the form's binding. The overlay screen asks it
    /// to name the title a folded row plays.
    pub fn action_at<E: IndexElem>(&self, key: &E) -> Option<Action> {
        let form = self.menu.form();
        match form.binding_at(form.index_of_key(RowKey(key.index()?))?)?.id {
            MoreRow::Act(action) => Some(action),
            MoreRow::OpenQuality | MoreRow::OpenSpeed => None,
        }
    }

    /// A row is its [`MoreRow::a11y_path`], relative to the menu (the overlay screen that hosts the menu places it
    /// under its own namespace). The inverse is the default walk: a page of a popover is a handful of rows.
    pub fn a11y<E: IndexElem>(&self, key: &E) -> Option<A11y<'static>> {
        popover_a11y(self.menu.form(), key, MoreRow::a11y_path)
    }

    /// Every row of the page showing, for the `a11y_elems` walk.
    pub fn a11y_elems<E: IndexElem>(&self, g: GroupId, group: GroupId, out: &mut Vec<E>) {
        popover_a11y_elems(self.menu.form(), g, group, out)
    }
}

/// Every row the menu can offer, in order and ACROSS SECTIONS. A free function (rather than a
/// literal inside [`MoreMenuState::open_focused`]) so the index mapping a press relies on is one testable value.
///
/// **The order here is the whole contract**, because [`TableView`]'s `sel` is a single flat index
/// over every row of every section: this list must be built in exactly the order
/// [`MoreMenuState::open_focused`] pushes rows, or a press commits its neighbour. A separator would
/// be a row here too — there is none, and the debug assert in [`MoreMenuState::open_focused`] is
/// what would catch one being added on one side only.
///
/// **`forced` (Force Direct Play, or a composition that converts nothing) drops the Quality section entirely.** Every rung is a request for
/// the server to convert, and Force forbids conversion (a no-Feed composition cannot perform one), so under it no rung can change what plays:
/// a row that cannot change the outcome is not offered (the same rule as the failure read-out's
/// `player::failure_actions`). Pure over the flag so both shapes are testable without a session.
fn rows_for(forced: bool) -> Vec<Action> {
    let mut v: Vec<Action> = if forced {
        Vec::new()
    } else {
        plx_media::route::available_quality_ladder()
            .iter()
            .map(|q| Action::SetQuality(*q))
            .collect()
    };
    v.push(Action::ToggleStats);
    if plx_os::labcfg::menu_row_enabled() {
        v.push(Action::SendDiagnostics);
    }
    v
}

fn label(a: Action) -> std::borrow::Cow<'static, str> {
    match a {
        Action::ToggleStats => plx_os::i18n::msg::widgets_menu_stats().into(),
        // the rung names itself — rate and frame in one string, because the row already carries
        // the picker's leading mark (see this module's doc)
        Action::SetQuality(q) => q.label().into(),
        Action::SetSpeed(r) => speed_label(r).into(),
        Action::SendDiagnostics => plx_os::i18n::msg::widgets_menu_diagnostics().into(),
        Action::None => "".into(),
        // a folded row is labelled by its `FoldedRow`, which knows the content
        Action::Hud(_) => "".into(),
    }
}

/// Where the Stats for nerds switch reads its state. The read-out's flag is owned by
/// `app::diagnostics`, whose caller names this menu's [`Action`]s, so the application registers a
/// reader at boot ([`install_stats_reader`], from `app::enter_application`) instead of this
/// module naming `app`. Unset it reads OFF, which is what the flag itself reads before anything has
/// toggled it.
static STATS_READER: std::sync::OnceLock<fn() -> bool> = std::sync::OnceLock::new();

/// Register the reader for the Stats for nerds switch. The first registration wins; the loop makes
/// exactly one, before any screen exists.
pub fn install_stats_reader(read: fn() -> bool) {
    let _ = STATS_READER.set(read);
}

fn stats_on() -> bool {
    STATS_READER.get().is_some_and(|read| read())
}

/// Whether the SWITCH a row names is currently on. It reaches the row as [`Row::toggle`] and so
/// draws as the WORD `On`/`Off` at the trailing edge — never as a picker's leading checkmark, which
/// means "the active one of several" and is what the Quality rung rows use instead. Two idioms, one
/// rule: a mark says where you are and a word says what is set, and no row says both. (Named
/// `checked` until 2026-08-21, from the row builder it does not call — a name that read as a
/// promise of the leading mark an Options row deliberately does not draw.)
fn is_on(a: Action) -> bool {
    match a {
        Action::ToggleStats => stats_on(),
        // a rung is not a switch — see `row_for`, which gives it the leading mark instead
        Action::SetQuality(_) | Action::SetSpeed(_) | Action::SendDiagnostics | Action::Hud(_) | Action::None => false,
    }
}

/// **"Original" is a claim about the SOURCE, and for some sources it is false.**
///
/// Every other rung names a bound the viewer can reason about — "1080p · 20 Mbps". This one names a
/// provenance, and when the television cannot decode the source video at all (MPEG-2, and AV1 / VP9
/// on a set whose codec table lists no such decoder — `route::source_decodable`) the server must
/// re-encode the pixels whatever is picked. The row
/// still does something: it is the only rung that sends no bitrate or resolution cap. But it cannot
/// deliver the original, and until this it said so nowhere, while the DETAIL page for the same item
/// already said "Converts on server" from the same predicate.
///
/// **The same words as the detail page, deliberately.** One vocabulary for one fact; a second
/// phrasing here would read as a second fact.
///
/// **A sub-line rather than a trailing value**, because `Row`'s rule is that a leading mark and a
/// trailing word may not both appear and every rung row carries the picker's mark. And no `dim`:
/// dimming is the ink of "unavailable", the row is fully selectable and still useful, and this is
/// the one rung that can ask for more than 1080p — an annotation that reads as "do not pick this"
/// would steer people off the only >1080p ask the app has.
/// **Pure, and it takes the fact rather than reading it.** The predicate lives on the session and
/// the row is drawn from it (`row_for`); passing it in is what lets the copy be tested without a
/// resolved playback, and what keeps this function a statement about the LADDER rather than about
/// global state.
fn quality_detail(q: plx_media::route::Quality, source_decodable: bool) -> &'static str {
    if q == plx_media::route::Quality::Original && !source_decodable {
        plx_ui::fmt::converts_on_server()
    } else {
        ""
    }
}

/// One row, drawn in the idiom its ACTION calls for. Free-standing (rather than inline in the form
/// builders) so the two idioms are decided in one place: a switch gets the trailing word, a picker
/// rung gets the leading mark ([`FormSection::choice`] derives it from the current rung), and nothing gets
/// both.
fn row_for(ps: &plx_media::route::PlaybackSession, a: Action) -> Row {
    match a {
        Action::SetQuality(q) => Row::new(label(a)).detail(quality_detail(q, plx_media::route::source_decodable(ps))),
        // a picker row like a rung: the leading mark comes from the page's `choice`
        Action::SetSpeed(_) => Row::new(label(a)),
        _ => Row::new(label(a)).toggle(is_on(a)),
    }
}

/// The panel at its WIDEST ([`plx_ui::table::MENU_MAX_W`]) and TALLEST, for the overscan audit
/// ([`plx_ui::consts::SAFE`]), placed by the same placer as the live panel beside the anchor of the compiled classic (`player_hud::audit_geometry`):
/// asked for the whole screen's height it gets the whole room, which is what a list longer than
/// the room fills (the track menu's audit asks the same).
#[cfg(any(test, feature = "test-support"))]
pub fn overscan_rects(out: &mut Vec<(&'static str, Rect)>) {
    let placed = plx_ui::popover::anchor_panel(
        crate::player_hud::audit_geometry().row,
        (plx_ui::table::MENU_MAX_W, plx_ui::consts::SCR_H),
        NEED_H,
        plx_ui::popover::Placement::PLAYER_MENU,
    );
    out.push(("… overflow menu panel", placed.rect));
}

/// The index→action mapping, which is the only part of a popover that is testable off the main
/// thread: a real `MoreMenuState` owns `TableView`/its row list, and both are main-thread-only,
/// like every other panel's state.
#[cfg(test)]
mod tests {
    use super::*;
    use plx_ui::form::{Activation, FormTable};
    use plx_ui::menu::Motion;
    use plx_ui::page_stack;

    type MoreTable = FormTable<MoreRow, Action, MorePage>;

    /// **The Original row says so when it cannot be Original.**
    ///
    /// For a source this television cannot decode — MPEG-2, and AV1 / VP9 on a set whose codec table
    /// lists no such decoder — the server must re-encode
    /// the pixels whatever rung is picked, so the word "Original" is a promise the pipeline cannot
    /// keep. The DETAIL page for the same item already said "Converts on server" from the same
    /// predicate; the quality picker, which is where a viewer goes to do something about it, said
    /// nothing at all.
    ///
    /// Differential: unmodified code draws no sub-line on any quality row, in any state.
    ///
    /// The negative half is the important one. A fixed rung ALSO converts, and Auto on such an
    /// item runs an encoded ladder too — but neither of them is named after the source, so neither
    /// is making the claim this line corrects. Annotating them would turn one honest correction
    /// into four lines of noise.
    #[test]
    fn only_the_original_row_says_the_source_cannot_be_preserved() {
        plx_media::speed_audio::force_speed_decoder_for_test(true); // a host test loads no FFmpeg
        use plx_media::route::Quality;
        assert_eq!(
            quality_detail(Quality::Original, false),
            "Converts on server"
        );
        assert_eq!(
            quality_detail(Quality::Original, true),
            "",
            "a source the panel decodes needs no correction — Original means Original",
        );
        for q in plx_media::route::QUALITY_LADDER {
            if q == Quality::Original {
                continue;
            }
            assert_eq!(
                quality_detail(q, false),
                "",
                "{q:?} is not named after the source, so it makes no claim to correct",
            );
        }
    }

    /// The copy is the DETAIL page's, verbatim. One vocabulary for one fact: a second phrasing
    /// would read as a second fact, and the two surfaces answer from the same predicate.
    #[test]
    fn the_conversion_notice_is_the_words_the_detail_page_already_uses() {
        plx_media::speed_audio::force_speed_decoder_for_test(true); // a host test loads no FFmpeg
        assert_eq!(
            quality_detail(plx_media::route::Quality::Original, false),
            plx_ui::fmt::converts_on_server(),
        );
    }

    #[test]
    fn every_row_has_a_label() {
        plx_media::speed_audio::force_speed_decoder_for_test(true); // a host test loads no FFmpeg
        for a in rows_for(false) {
            assert!(!label(a).is_empty(), "{a:?} would draw a blank row");
        }
        // The full persisted ladder must keep finished UI copy even if the readiness gate is
        // deliberately closed again for a future protocol regression.
        for q in plx_media::route::QUALITY_LADDER {
            assert!(
                !label(Action::SetQuality(q)).is_empty(),
                "{q:?} would draw a blank row when enabled"
            );
        }
    }

    impl MoreMenuState {
        /// OK on the row the cursor is on, as the screen feeds it ([`MoreMenuState::step`]).
        pub(super) fn on_ok(&mut self, ps: &plx_media::route::PlaybackSession) -> Out<Action> {
            match self.menu.form().selected_key() {
                Some(key) => self.step(ps, Event::Ok(key.0)),
                None => Out::Stay,
            }
        }

        /// The focus engine moved to the row `key` ([`Event::Focused`]); `false` when the page showing has no such row.
        pub(super) fn focus_key(&mut self, key: u32) -> bool {
            self.menu.step(Event::Focused(key), &mut |_| None);
            self.menu.form().selected_key().is_some_and(|k| k.0 == key)
        }

        /// LEFT: pops the page; `false` at the root.
        pub(super) fn pop(&mut self, ps: &plx_media::route::PlaybackSession) -> bool {
            matches!(self.step(ps, Event::Left), Out::Moved)
        }

        /// RIGHT on the row the cursor is on: a Nav row opens, any other answer is left alone (this menu has no tabs).
        pub(super) fn on_right(&mut self, ps: &plx_media::route::PlaybackSession) {
            let key = self.menu.form().selected_key().map(|k| k.0);
            self.step(ps, Event::Right(key));
        }

        /// Open `page` from its opener's row, as a press would (the slide runs).
        pub(super) fn push_page(&mut self, ps: &plx_media::route::PlaybackSession, page: MorePage) {
            self.step(ps, Event::Enter(match page {
                MorePage::Quality => MoreRow::OpenQuality,
                MorePage::Speed => MoreRow::OpenSpeed,
            }));
        }

        /// Run the card's motion to rest (a slide or a resize), a frame at a time.
        pub(super) fn settle(&mut self) {
            let m = plx_base::fontcov::advances::ShippedMeasure;
            for _ in 0..600 {
                if self.menu.state().motion == Motion::Rest {
                    return;
                }
                self.menu.step(Event::Tick { dt: 1.0 / 60.0, measure: &m, focus: None }, &mut |_| None);
            }
            panic!("the motion never settles");
        }
    }

    /// The actions of the rows showing, in table order, read back off a built menu by identity (the
    /// root's Quality drill-in commits nothing and is not one).
    fn order(st: &MoreMenuState) -> Vec<Action> {
        (0..st.menu.form().table.n_rows() as usize)
            .filter_map(|i| match st.menu.form().id_at(i) {
                Some(MoreRow::Act(a)) => Some(*a),
                _ => None,
            })
            .collect()
    }

    /// A menu ROOT built from the row list, without a `PlaybackSession` beyond the default one.
    fn menu(forced: bool, current: plx_media::route::Quality) -> MoreMenuState {
        let ps = plx_media::route::PlaybackSession::default();
        let rows = rows_for(forced);
        let core = MoreCore { rows, forced, current, speed: plx_media::player::RATE_NORMAL, speeds: plx_media::player::PLAY_RATES.to_vec(), folded: Vec::new() };
        let mut menu = Menu::new(host(crate::player_hud::classic::ctrl_row_hit_rect(), 0), Class::Action, plx_ui::table_screen::BAND_BASE);
        menu.form_mut().set_or_open(root_form(&ps, &core.rows, forced, current, plx_media::player::RATE_NORMAL, &plx_media::player::PLAY_RATES, &[]), None);
        MoreMenuState { core, menu }
    }

    /// The same menu with the Quality page pushed (and its slide skipped).
    fn quality_page(current: plx_media::route::Quality) -> MoreMenuState {
        let ps = plx_media::route::PlaybackSession::default();
        let mut st = menu(false, current);
        st.focus_key(MoreRow::OpenQuality.key().0);
        assert_eq!(st.on_ok(&ps), Out::Moved);
        st.settle();
        st
    }

    /// The text a row reads out as its value (a Nav row's current setting).
    pub(super) fn value_of(st: &mut MoreMenuState, id: MoreRow) -> Option<String> {
        let i = st.menu.form().index_of(&id)?;
        st.menu.form_mut().table.row_mut(i as i32).and_then(|r| r.value.clone())
    }

    /// Is the row `id` drawn with the picker's leading check?
    fn checked(st: &mut MoreMenuState, id: MoreRow) -> bool {
        let i = st.menu.form().index_of(&id).expect("a row");
        st.menu.form_mut().table.row_mut(i as i32).is_some_and(|r| r.checked)
    }

    use plx_media::route::Quality;

    /// Pressing each row commits ITS action, addressed by identity — never by a position: an
    /// Options row on the root and a rung on the Quality page.
    #[test]
    fn a_focused_row_commits_its_own_action() {
        plx_media::speed_audio::force_speed_decoder_for_test(true); // a host test loads no FFmpeg
        let ps = plx_media::route::PlaybackSession::default();
        let mut st = menu(false, Quality::P1080);
        for a in rows_for(false).into_iter().filter(|a| !matches!(a, Action::SetQuality(_))) {
            st.focus_key(a.key().0);
            assert_eq!(st.on_ok(&ps), Out::Picked(a));
        }
        let mut st = quality_page(Quality::P1080);
        for a in rows_for(false).into_iter().filter(|a| matches!(a, Action::SetQuality(_))) {
            st.focus_key(a.key().0);
            // exactly the action the flat ladder reported: `app::playback::apply_more_action` is
            // the one place a `SetQuality` is performed (`route::set_quality`)
            assert_eq!(st.on_ok(&ps), Out::Picked(a), "{a:?}");
        }
    }

    /// **The root is the Quality drill-in then Options; the rungs live on the page.** The root no
    /// longer lists a rung, the page lists the ladder in order and nothing else.
    #[test]
    fn the_root_leads_with_the_quality_row_and_the_page_holds_the_ladder() {
        plx_media::speed_audio::force_speed_decoder_for_test(true); // a host test loads no FFmpeg
        let st = menu(false, Quality::Original);
        let ids: Vec<Option<MoreRow>> =
            (0..st.menu.form().table.n_rows() as usize).map(|i| st.menu.form().id_at(i).copied()).collect();
        assert_eq!(ids[0], Some(MoreRow::OpenQuality), "Quality leads");
        assert!(ids.contains(&Some(MoreRow::Act(Action::ToggleStats))));
        assert!(
            !ids.iter().any(|i| matches!(i, Some(MoreRow::Act(Action::SetQuality(_))))),
            "no rung on the root: {ids:?}"
        );
        let ladder: Vec<Action> =
            plx_media::route::available_quality_ladder().iter().map(|q| Action::SetQuality(*q)).collect();
        assert_eq!(order(&quality_page(Quality::Original)), ladder);
    }

    /// **The root's Quality row reads out the CURRENT choice** — "Auto · Original" (an untouched
    /// file under Auto), "Original", "1080p" (the short name; the page keeps "1080p · 8 Mbps") —
    /// as its value, and the page checks that rung and no other.
    #[test]
    fn the_quality_row_reads_the_current_rung_and_the_page_checks_it() {
        plx_media::speed_audio::force_speed_decoder_for_test(true); // a host test loads no FFmpeg
        for q in plx_media::route::QUALITY_LADDER {
            let mut st = menu(false, q);
            let want = if q == plx_media::route::Quality::Auto {
                format!("{} \u{b7} {}", q.label(), plx_media::route::Quality::Original.label())
            } else {
                q.short_label()
            };
            assert_eq!(value_of(&mut st, MoreRow::OpenQuality), Some(want), "{q:?}");
            assert!(
                !checked(&mut st, MoreRow::OpenQuality),
                "a drill-in row carries no check — a mark says where you are, a word says what is set"
            );
            let mut st = quality_page(q);
            for rung in plx_media::route::QUALITY_LADDER {
                let id = MoreRow::Act(Action::SetQuality(rung));
                if st.menu.form().index_of(&id).is_some() {
                    assert_eq!(checked(&mut st, id), rung == q, "{rung:?} while {q:?} is current");
                }
            }
        }
    }

    /// **Push focuses the checked rung; pop restores focus on the Quality row** (by identity), the
    /// title band comes and goes with the page, and BACK at the root pops nothing.
    #[test]
    fn push_lands_on_the_checked_rung_and_pop_restores_the_quality_row() {
        plx_media::speed_audio::force_speed_decoder_for_test(true); // a host test loads no FFmpeg
        let ps = plx_media::route::PlaybackSession::default();
        for q in plx_media::route::available_quality_ladder() {
            let mut st = menu(false, *q);
            assert!(!st.pop(&ps), "the root has nothing to pop");
            st.focus_key(MoreRow::OpenQuality.key().0);
            assert_eq!(st.on_ok(&ps), Out::Moved);
            assert_eq!(st.page(), Some(MorePage::Quality));
            assert_eq!(st.menu.form().selected_id(), Some(&MoreRow::Act(Action::SetQuality(*q))), "initial focus = the checked rung");
            assert_eq!(st.menu.form().table.title(), Some("Quality"));
            assert!(st.menu.pointer_held(), "a push slides");
            assert!(st.pop(&ps));
            assert_eq!(st.page(), None);
            assert_eq!(st.menu.form().selected_id(), Some(&MoreRow::OpenQuality), "pop restores focus on the Quality row");
            assert_eq!(st.menu.form().table.title(), None);
        }
    }

    /// RIGHT on the Quality row enters it, on an Options row does nothing; the page's own RIGHT
    /// does nothing either (there is no deeper page and no tab to switch to).
    #[test]
    fn right_enters_the_quality_row_and_nothing_else() {
        plx_media::speed_audio::force_speed_decoder_for_test(true); // a host test loads no FFmpeg
        let ps = plx_media::route::PlaybackSession::default();
        let mut st = menu(false, Quality::P720);
        st.focus_key(Action::ToggleStats.key().0);
        st.on_right(&ps);
        assert_eq!(st.page(), None, "RIGHT on an Options row stays put");
        st.focus_key(MoreRow::OpenQuality.key().0);
        st.on_right(&ps);
        assert_eq!(st.page(), Some(MorePage::Quality));
        st.on_right(&ps);
        assert_eq!(st.page(), Some(MorePage::Quality), "RIGHT on a rung does nothing");
    }

    /// **`prepare` queues the OPENING page's strings with no `update`, once** — the root for the
    /// ordinary entry, the Quality page for a quality entry.
    #[test]
    fn prepare_queues_the_opening_page_once_without_an_update() {
        plx_media::speed_audio::force_speed_decoder_for_test(true); // a host test loads no FFmpeg
        use plx_ui::fixture::FixtureMeasure as M;
        let _serial = plx_base::testlock::serial();
        let ps = plx_media::route::PlaybackSession::default();
        for st in [MoreMenuState::new(&ps), MoreMenuState::new_quality(&ps)] {
            plx_gfx::text::reset_prewarm_for_test();
            st.menu().prepare(&M);
            assert!(plx_gfx::text::prewarm_pending(), "prepare queued the opening page");
            plx_gfx::text::clear_prewarm();
            st.menu().prepare(&M);
            assert!(!plx_gfx::text::prewarm_pending(), "an unchanged layout was walked again");
        }
    }

    /// The failure screen's entry opens ON the Quality page, the active rung focused, no slide; a
    /// Force Direct Play entry (no ladder) and the ordinary entry open the root.
    #[test]
    fn the_quality_entry_opens_on_the_page_and_the_ordinary_entry_on_the_root() {
        plx_media::speed_audio::force_speed_decoder_for_test(true); // a host test loads no FFmpeg
        let _serial = plx_base::testlock::serial();
        let ps = plx_media::route::PlaybackSession::default();
        let st = MoreMenuState::new(&ps);
        assert_eq!(st.page(), None);
        let st = MoreMenuState::new_quality(&ps);
        assert_eq!(st.page(), Some(MorePage::Quality));
        assert!(!st.menu.pointer_held(), "the entry is not a slide");
        assert_eq!(st.menu.form().table.title(), Some("Quality"));
        let active = MoreRow::Act(Action::SetQuality(plx_media::route::quality()));
        if st.menu.form().index_of(&active).is_some() {
            assert_eq!(st.menu.form().selected_id(), Some(&active), "the active rung is focused");
        }
    }

    /// **The Speed page is Plex Web's seven speeds, slowest first, with the current one marked.**
    /// It is offered on the root under Force Direct Play too — a speed changes how fast the
    /// picture plays, not what the server sends — and a row commits its own speed.
    #[test]
    fn the_speed_page_offers_the_seven_speeds_with_the_current_one_checked() {
        plx_media::speed_audio::force_speed_decoder_for_test(true); // a host test loads no FFmpeg
        let ps = plx_media::route::PlaybackSession::default();
        for forced in [false, true] {
            let mut root = MoreMenuState::new(&ps);
            let form = root_form(&ps, &rows_for(forced), forced, plx_media::route::Quality::Auto, 1000, &plx_media::player::PLAY_RATES, &[]);
            root.menu.form_mut().set(form, None);
            assert!(
                root.menu.form().index_of(&MoreRow::OpenSpeed).is_some(),
                "the root offers Playback speed (forced={forced})"
            );
        }
        let mut st = MoreMenuState::new(&ps);
        st.core.speed = 1500;
        st.focus_key(MoreRow::OpenSpeed.key().0);
        st.push_page(&ps, MorePage::Speed);
        assert_eq!(st.page(), Some(MorePage::Speed));
        assert_eq!(st.menu.form().table.title(), Some("Playback speed"));
        let ids: Vec<MoreRow> = (0..st.menu.form().table.n_rows() as usize).filter_map(|i| st.menu.form().id_at(i).copied()).collect();
        let want: Vec<MoreRow> =
            plx_media::player::PLAY_RATES.iter().map(|r| MoreRow::Act(Action::SetSpeed(*r))).collect();
        assert_eq!(ids, want, "the seven speeds, slowest first");
        assert_eq!(
            st.menu.form().selected_id(),
            Some(&MoreRow::Act(Action::SetSpeed(1500))),
            "the page opens on the current speed"
        );
        let i = st.menu.form().index_of(&MoreRow::Act(Action::SetSpeed(750))).unwrap();
        match st.menu.form().activate(i) {
            Some(Activation::Action(got)) => assert_eq!(got, Action::SetSpeed(750)),
            _ => panic!("a speed row must commit its own speed"),
        }
        assert!(st.pop(&ps), "BACK returns to the root");
        assert_eq!(st.menu.form().selected_id(), Some(&MoreRow::OpenSpeed), "on the row that opened it");
    }

    /// **`open_more {page: speed}` opens the Speed page itself** — no drill-in press, no slide —
    /// on the current speed, and BACK returns to the root on the row that would have opened it. A
    /// playback that takes one speed only has no Speed page to open, so it is the ordinary root
    /// (`speed_offer`'s own tests grade when that is).
    #[test]
    fn the_menu_opens_straight_on_the_speed_page_when_it_is_offered() {
        plx_media::speed_audio::force_speed_decoder_for_test(true); // a host test loads no FFmpeg
        let ps = plx_media::route::PlaybackSession::default();
        let mut st = MoreMenuState::new_on(&ps, Some(MorePage::Speed));
        assert_eq!(st.core.speeds.len(), plx_media::player::PLAY_RATES.len(), "the fixture offers every speed");
        assert_eq!(st.page(), Some(MorePage::Speed));
        assert_eq!(st.menu.form().table.title(), Some("Playback speed"));
        assert_eq!(
            st.menu.form().selected_id(),
            Some(&MoreRow::Act(Action::SetSpeed(st.core.speed))),
            "on the current speed"
        );
        assert!(st.pop(&ps), "BACK returns to the root");
        assert_eq!(st.menu.form().selected_id(), Some(&MoreRow::OpenSpeed), "on the row that opens Speed");

        assert_eq!(MoreMenuState::new_on(&ps, None).page(), None, "the root is the root");
    }

    /// **No offer, no row**: where a playback can play only 1x (segmented HLS, an undecodable audio
    /// codec, a television that failed PCM — `player::speed_offer`), the root has no Playback speed
    /// row, and an open Speed page goes back to the root when the offer goes away under it.
    #[test]
    fn the_speed_row_follows_the_offer() {
        plx_media::speed_audio::force_speed_decoder_for_test(true); // a host test loads no FFmpeg
        let ps = plx_media::route::PlaybackSession::default();
        let rows = rows_for(false);
        for offer in [vec![], vec![1000]] {
            let mut root = MoreMenuState::new(&ps);
            root.menu.form_mut().set(root_form(&ps, &rows, false, Quality::Auto, 1000, &offer, &[]), None);
            assert!(root.menu.form().index_of(&MoreRow::OpenSpeed).is_none(), "offer {offer:?}");
        }
        let mut st = MoreMenuState::new(&ps);
        st.focus_key(MoreRow::OpenSpeed.key().0);
        st.push_page(&ps, MorePage::Speed);
        assert_eq!(st.page(), Some(MorePage::Speed));
        let (forced, rows, q) = (st.core.forced, st.core.rows.clone(), st.core.current);
        assert!(st.refresh_to(&ps, forced, rows, q, 1000, Vec::new()));
        assert_eq!(st.page(), None, "the page went with its offer");
        assert!(st.menu.form().index_of(&MoreRow::OpenSpeed).is_none());
    }

    /// The names: Normal at 1x, the multiplier with no trailing zeros otherwise, the way Plex Web
    /// spells them (it writes `.5x`; we keep the leading zero and the real multiplication sign).
    #[test]
    fn speeds_are_named_like_plex_webs() {
        plx_media::speed_audio::force_speed_decoder_for_test(true); // a host test loads no FFmpeg
        let names: Vec<String> = plx_media::player::PLAY_RATES.iter().map(|r| speed_label(*r)).collect();
        assert_eq!(names, ["0.5×", "0.75×", "Normal", "1.25×", "1.5×", "1.75×", "2×"]);
    }

    /// Every speed row has its own focus key, clear of every other row's.
    #[test]
    fn every_speed_row_has_a_distinct_key() {
        plx_media::speed_audio::force_speed_decoder_for_test(true); // a host test loads no FFmpeg
        let mut keys: Vec<u32> = plx_media::player::PLAY_RATES.iter().map(|r| Action::SetSpeed(*r).key().0).collect();
        keys.extend([MoreRow::OpenQuality.key().0, MoreRow::OpenSpeed.key().0]);
        keys.extend([Action::None, Action::ToggleStats, Action::SendDiagnostics].iter().map(|a| a.key().0));
        keys.extend(plx_media::route::QUALITY_LADDER.iter().map(|q| Action::SetQuality(*q).key().0));
        let mut sorted = keys.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), keys.len(), "a key is shared: {keys:?}");
    }

    /// Reordering the rows moves no key: every row keeps the key (and so the action) it had, and
    /// the press still commits the row's own action.
    #[test]
    fn a_reordered_menu_resolves_every_key_to_the_same_action() {
        plx_media::speed_audio::force_speed_decoder_for_test(true); // a host test loads no FFmpeg
        let ps = plx_media::route::PlaybackSession::default();
        let mut rows = rows_for(false);
        rows.reverse();
        let core = MoreCore {
            rows: rows.clone(),
            forced: false,
            current: Quality::Auto,
            speed: plx_media::player::RATE_NORMAL,
            speeds: plx_media::player::PLAY_RATES.to_vec(),
            folded: Vec::new(),
        };
        let mut menu = Menu::new(host(crate::player_hud::classic::ctrl_row_hit_rect(), 0), Class::Action, plx_ui::table_screen::BAND_BASE);
        menu.form_mut().set(quality_form(&ps, &rows, Quality::Auto), None);
        let st = MoreMenuState { core, menu };
        for a in rows_for(false).into_iter().filter(|a| matches!(a, Action::SetQuality(_))) {
            let i = st.menu.form().index_of_key(a.key()).expect("every row keeps its key");
            assert_eq!(st.menu.form().id_at(i), Some(&MoreRow::Act(a)));
            match st.menu.form().activate(i) {
                Some(Activation::Action(got)) => assert_eq!(got, a),
                _ => panic!("{a:?} must commit its own action"),
            }
        }
    }

    /// **Quality must stay entirely ahead of Options in the row list** ([`rows_for`]): the root
    /// builds the Quality drill-in before the Options section, and the list is what both forms are
    /// built from, so a rung added on the wrong side of the split would reach the Options section.
    #[test]
    fn every_quality_rung_sits_ahead_of_the_stats_toggle() {
        plx_media::speed_audio::force_speed_decoder_for_test(true); // a host test loads no FFmpeg
        let rows = rows_for(false);
        let stats_i = rows
            .iter()
            .position(|a| *a == Action::ToggleStats)
            .expect("the toggle is always in the menu");
        for (i, a) in rows.iter().enumerate() {
            if matches!(a, Action::SetQuality(_)) {
                assert!(
                    i < stats_i,
                    "{a:?} at row {i} must come before Stats for nerds at row {stats_i}"
                );
            }
        }
    }

    /// **A composition that converts nothing offers no Quality row either**: no Feed, and no URI resolver that certifies a
    /// converted output (here none at all; `plx_media`'s `a_quality_pick_no_composed_backend_can_carry_is_refused_and_leaves_the_route_alone`
    /// covers the Tizen resolver that is installed and certifies none). A rung picked there would persist a ceiling the playing
    /// original cannot honour and queue a Retranscode no pump consumes, so the ladder is not offered. A Feed (webOS) keeps it.
    #[test]
    fn a_composition_that_converts_nothing_offers_no_quality_row() {
        plx_media::speed_audio::force_speed_decoder_for_test(true); // a host test loads no FFmpeg
        let mut ps = plx_media::route::PlaybackSession::default();
        ps.install_backends(plx_media::route::BackendSet::URI);
        let st = MoreMenuState::new(&ps);
        assert!(!st.core.rows.iter().any(|a| matches!(a, Action::SetQuality(_))), "{:?}", st.core.rows);
        assert!(!quality_offered(&st.core.rows, st.core.forced));
        ps.install_backends(plx_media::route::BackendSet::BOTH);
        let st = MoreMenuState::new(&ps);
        assert!(quality_offered(&st.core.rows, st.core.forced), "a composition with a Feed keeps the ladder");
    }

    /// **Force Direct Play leaves no Quality row and no page**: no rung can change what plays under
    /// it, so none is offered and a quality entry lands on the root's first row, Playback speed.
    #[test]
    fn forced_direct_play_offers_no_quality_row_and_lands_on_the_first_option() {
        plx_media::speed_audio::force_speed_decoder_for_test(true); // a host test loads no FFmpeg
        let ps = plx_media::route::PlaybackSession::default();
        let forced = rows_for(true);
        assert!(
            !forced.iter().any(|a| matches!(a, Action::SetQuality(_))),
            "forced direct play must not offer a quality rung: {forced:?}"
        );
        assert_eq!(forced.first(), Some(&Action::ToggleStats));
        assert!(!quality_offered(&forced, true));
        assert!(!quality_offered(&[Action::ToggleStats], false), "an empty ladder offers no page to open");
        assert!(quality_offered(&rows_for(false), false));
        for q in plx_media::route::QUALITY_LADDER {
            let mut st = menu(true, q);
            assert!(st.menu.form().index_of(&MoreRow::OpenQuality).is_none());
            assert_eq!(st.sel(), 0);
            // the root's first row is Playback speed, which Force Direct Play still offers
            assert_eq!(st.menu.form().selected_id(), Some(&MoreRow::OpenSpeed), "a vanished rung opens on the root's first row");
            assert_eq!(st.on_ok(&ps), Out::Moved);
            assert_eq!(st.page(), Some(MorePage::Speed));
        }
        // not forced: the ladder is unchanged, in order, ahead of Options
        let open = rows_for(false);
        let ladder: Vec<Action> = open
            .iter()
            .copied()
            .filter(|a| matches!(a, Action::SetQuality(_)))
            .collect();
        let expected: Vec<Action> = plx_media::route::available_quality_ladder()
            .iter()
            .map(|q| Action::SetQuality(*q))
            .collect();
        assert_eq!(ladder, expected);
        assert_eq!(&open[ladder.len()..], &forced[..]);
    }

    /// **A live change refreshes the open Quality page IN PLACE**: a rung leaving (Auto's gate
    /// closing) or the current rung moving rebuilds the page, keeps the page open, the title band
    /// and the focused row by id (the checked rung when the focused one left).
    #[test]
    fn a_live_rung_change_refreshes_the_open_page_in_place() {
        plx_media::speed_audio::force_speed_decoder_for_test(true); // a host test loads no FFmpeg
        let ps = plx_media::route::PlaybackSession::default();
        let mut st = quality_page(Quality::P1080);
        st.focus_key(Action::SetQuality(Quality::P720).key().0);
        let before = st.menu.form().table.sel;
        // the current rung moves: the check follows, focus stays on the row the viewer was on
        let rows = st.core.rows.clone();
        assert!(st.refresh_to(&ps, false, rows.clone(), Quality::P480, plx_media::player::RATE_NORMAL, plx_media::player::PLAY_RATES.to_vec()));
        assert!(!st.refresh_to(&ps, false, rows.clone(), Quality::P480, plx_media::player::RATE_NORMAL, plx_media::player::PLAY_RATES.to_vec()), "and only once");
        assert_eq!(st.page(), Some(MorePage::Quality), "still on the page");
        assert_eq!(st.menu.form().table.title(), Some("Quality"), "the title band stays");
        assert_eq!(st.menu.form().selected_id(), Some(&MoreRow::Act(Action::SetQuality(Quality::P720))));
        assert_eq!(st.menu.form().table.sel, before);
        assert!(checked(&mut st, MoreRow::Act(Action::SetQuality(Quality::P480))));
        assert!(!checked(&mut st, MoreRow::Act(Action::SetQuality(Quality::P1080))));
        // the focused rung leaves the ladder: focus falls back to the checked rung, page stays
        let without: Vec<Action> = rows.iter().copied().filter(|a| *a != Action::SetQuality(Quality::P720)).collect();
        assert!(st.refresh_to(&ps, false, without, Quality::P480, plx_media::player::RATE_NORMAL, plx_media::player::PLAY_RATES.to_vec()));
        assert!(st.menu.form().index_of(&MoreRow::Act(Action::SetQuality(Quality::P720))).is_none());
        assert_eq!(st.page(), Some(MorePage::Quality));
        assert_eq!(st.menu.form().selected_id(), Some(&MoreRow::Act(Action::SetQuality(Quality::P480))));
    }

    /// **Quality becoming unavailable pops to the root**: Force Direct Play arriving under the open
    /// page drops the ladder, so the page goes, the band goes, the slide is cancelled and the root
    /// comes back without the Quality row.
    #[test]
    fn quality_becoming_unavailable_pops_the_page_to_the_root() {
        plx_media::speed_audio::force_speed_decoder_for_test(true); // a host test loads no FFmpeg
        let ps = plx_media::route::PlaybackSession::default();
        let mut st = quality_page(Quality::P1080);
        assert!(st.refresh_to(&ps, true, rows_for(true), Quality::P1080, plx_media::player::RATE_NORMAL, plx_media::player::PLAY_RATES.to_vec()));
        assert_eq!(st.page(), None);
        assert_eq!(st.menu.form().table.title(), None);
        assert!(!st.menu.pointer_held(), "a page that vanished does not slide away");
        assert!(st.menu.form().index_of(&MoreRow::OpenQuality).is_none());
        // the root's first row, Playback speed (Force Direct Play keeps it)
        assert_eq!(st.menu.form().selected_id(), Some(&MoreRow::OpenSpeed));
    }

    /// A key that names no row commits nothing, never a neighbour's action.
    #[test]
    fn an_unknown_key_is_none_not_a_neighbour() {
        plx_media::speed_audio::force_speed_decoder_for_test(true); // a host test loads no FFmpeg
        let mut st = menu(false, Quality::Auto);
        let before = st.sel();
        assert!(!st.focus_key(0xdead), "an unknown key reports it seated nothing");
        assert_eq!(st.sel(), before, "an unknown key moves nothing");
        assert!(st.focus_key(MoreRow::OpenQuality.key().0), "the Quality row is seatable on the root");
    }

    /// Every row's key is distinct and below the band, and the title band's pointer key is none of
    /// them.
    #[test]
    fn every_key_is_distinct_and_below_the_band() {
        plx_media::speed_audio::force_speed_decoder_for_test(true); // a host test loads no FFmpeg
        let mut all: Vec<MoreRow> = plx_media::route::QUALITY_LADDER.iter().map(|q| MoreRow::Act(Action::SetQuality(*q))).collect();
        all.extend([MoreRow::OpenQuality, MoreRow::Act(Action::ToggleStats), MoreRow::Act(Action::SendDiagnostics)]);
        let mut keys: Vec<u32> = all.iter().map(|a| a.key().0).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), all.len());
        assert!(keys.iter().all(|k| *k < plx_ui::table_screen::BAND_BASE));
        assert!(page_stack::is_title_key(page_stack::TITLE_KEY));
        assert!(keys.iter().all(|k| !page_stack::is_title_key(*k)));
    }

    /// **The replay canon tells the root from the Quality page**, and one selected row from
    /// another: the same index on the two pages must not hash alike.
    #[test]
    fn the_canon_tells_the_root_and_the_page_apart() {
        plx_media::speed_audio::force_speed_decoder_for_test(true); // a host test loads no FFmpeg
        use plx_machine::machine::Canon;
        let fp = |st: &MoreMenuState| {
            let mut c = Canon::new();
            st.canon(&mut c);
            c.finish()
        };
        let root = menu(false, Quality::P1080);
        let page = quality_page(Quality::P1080);
        assert_ne!(fp(&root), fp(&page), "depth and opener are in the canon");
        assert_eq!(fp(&page), fp(&quality_page(Quality::P1080)), "the same state hashes the same");
        let mut moved = quality_page(Quality::P1080);
        moved.focus_key(Action::SetQuality(Quality::P480).key().0);
        assert_ne!(fp(&page), fp(&moved), "the selected KEY is in the canon");
    }

    /// **A push slides the page and a pop slides it back; both rest.** The panel reports motion each
    /// frame it animates and asks for none once settled — the same bar as the track menu.
    #[test]
    fn a_push_and_a_pop_slide_and_then_rest() {
        plx_media::speed_audio::force_speed_decoder_for_test(true); // a host test loads no FFmpeg
        let _serial = plx_base::testlock::serial();
        const DT: f32 = 1.0 / 60.0;
        let ps = plx_media::route::PlaybackSession::default();
        let m = plx_base::fontcov::advances::ShippedMeasure;
        let step = |st: &mut MoreMenuState| {
            plx_machine::motion::report::frame_begin(DT);
            st.update(DT, &m, &ps);
            plx_machine::motion::report::present_moving()
        };
        let mut st = menu(false, plx_media::route::quality());
        st.update(DT, &m, &ps);
        assert!(!step(&mut st), "an opened menu is at rest");
        st.focus_key(MoreRow::OpenQuality.key().0);
        assert_eq!(st.on_ok(&ps), Out::Moved);
        assert_eq!(st.menu.state().motion, Motion::Sliding);
        assert!(step(&mut st), "the slide is motion");
        assert!(st.menu.motion().live_dx() > 0.0, "the arriving page comes in from the right");
        let (leaving, live) = st.menu.motion().alphas();
        assert!(!leaving.is_empty() && live < 1.0, "the old page leaves while the new one fades in: {leaving:?} {live}");
        let mut n = 0;
        while step(&mut st) {
            n += 1;
            assert!(n < 240, "the push never settles");
        }
        assert!(st.menu.state().motion == Motion::Rest && !st.menu.pointer_held());
        assert_eq!(st.menu.motion().live_dx(), 0.0);
        assert_eq!(st.menu.motion().alphas(), (Vec::new(), 1.0), "at rest the page is whole");
        assert!(!step(&mut st), "at rest: no frames requested");
        assert!(st.pop(&ps));
        assert_eq!(st.menu.state().motion, Motion::Sliding);
        assert!(step(&mut st), "the pop is motion too");
        assert!(st.menu.motion().live_dx() < 0.0, "the page returns from the left");
        n = 0;
        while step(&mut st) {
            n += 1;
            assert!(n < 240, "the pop never settles");
        }
        assert!(st.menu.state().motion == Motion::Rest && !st.menu.pointer_held());
        assert!(!step(&mut st));
    }

    /// **A row set that changes height animates the card** (`ui::panel_motion`): the panel opens
    /// at rest, a live change to the set (Force Direct Play dropping the Quality row, Auto's
    /// gate opening) rebuilds it and the top edge springs to the new layout while the bottom and
    /// right stay anchored, then it asks for no more frames.
    #[test]
    fn a_changed_row_set_resizes_the_card_with_a_spring_and_then_rests() {
        plx_media::speed_audio::force_speed_decoder_for_test(true); // a host test loads no FFmpeg
        let _serial = plx_base::testlock::serial();
        const DT: f32 = 1.0 / 60.0;
        let ps = plx_media::route::PlaybackSession::default();
        let m = plx_base::fontcov::advances::ShippedMeasure;
        let step = |st: &mut MoreMenuState| {
            plx_machine::motion::report::frame_begin(DT);
            st.update(DT, &m, &ps);
            plx_machine::motion::report::present_moving()
        };
        // opened under Force Direct Play: no Quality row, a short panel
        let mut st = menu(true, plx_media::route::quality());
        let short = st.menu.panel_rect(&m);
        // the open: the first step places the card AT its layout
        st.menu.step(Event::Tick { dt: DT, measure: &m, focus: None }, &mut |_| None);
        assert!(!st.menu.pointer_held(), "a freshly opened panel is at rest");
        assert_eq!(st.menu.shown_rect(&m), short);

        // Force Direct Play is switched off under the open panel: the Quality row appears, the panel grows
        assert!(st.refresh(&ps), "the row set moved");
        assert!(!st.refresh(&ps), "and only once");
        let tall = st.menu.panel_rect(&m);
        assert!(tall.h > short.h, "the Quality row makes the panel taller: {} > {}", tall.h, short.h);
        assert_eq!((tall.x + tall.w, tall.y + tall.h), (short.x + short.w, short.y + short.h), "bottom and right are the anchor");
        assert!(st.menu.pointer_held());
        assert!(step(&mut st), "the resize is motion");
        let mid = st.menu.shown_rect(&m);
        assert!(mid.h > short.h && mid.h < tall.h, "mid-resize the card is between the heights: {mid:?}");
        let mut n = 0;
        while step(&mut st) {
            n += 1;
            assert!(n < 240, "the resize never settles");
        }
        assert_eq!(st.menu.shown_rect(&m), tall, "lands exactly on the new layout");
        assert!(!st.menu.pointer_held());
        assert!(!step(&mut st), "at rest: no frames requested");
    }
}

#[cfg(test)]
mod focus_tests {
    use super::*;
    use plx_ui::a11y::Role;
    use plx_ui::form::FormTable;

    type MoreTable = FormTable<MoreRow, Action, MorePage>;
    use plx_machine::machine::{Cx, EntryId, FocusKey, FocusRead, GroupId, Host as MachineHost, InputOwner, PressRead, Tick};
    use plx_ui::screen::{At, Dir, Focusable, GroupSpec, Placed, Step};

    /// **The menu as the overlay screen answers the engine for it**: every query is the component's own
    /// ([`plx_ui::menu::Menu`]), asked the way `PlayerOverlayScreen`'s `Focusable` asks it.
    struct MoreMenuPart<'a> {
        state: &'a MoreMenuState,
        entry: EntryId,
        group: GroupId,
    }

    impl Focusable<HostFixture> for MoreMenuPart<'_> {
        fn groups(&self, cx: &Cx<'_, HostFixture>, out: &mut Vec<GroupSpec>) {
            self.state.menu().groups(self.group, cx.measure, out);
        }
        fn group_of(&self, key: &u32, _cx: &Cx<'_, HostFixture>) -> Option<GroupId> {
            self.state.menu().group_of(key, self.group)
        }
        fn neighbour(&self, key: FocusKey<u32>, dir: Dir, _cx: &Cx<'_, HostFixture>) -> Step<u32> {
            self.state.menu().neighbour(self.entry, key, dir)
        }
        fn place(&self, key: &u32, cx: &Cx<'_, HostFixture>, _at: At) -> Option<Placed> {
            self.state.menu().place(key, cx.measure)
        }
        fn reconcile(&self, want: FocusKey<u32>, _cx: &Cx<'_, HostFixture>) -> FocusKey<u32> {
            self.state.menu().reconcile(self.entry, want, self.group)
        }
        fn seat(&self, _g: GroupId, _from: Placed, _cx: &Cx<'_, HostFixture>) -> FocusKey<u32> {
            self.state.menu().seat(self.entry)
        }
        fn a11y(&self, key: &u32, _cx: &Cx<'_, HostFixture>) -> Option<plx_ui::a11y::A11y<'_>> {
            self.state.a11y(key)
        }
        fn a11y_elems(&self, g: GroupId, _cx: &Cx<'_, HostFixture>, out: &mut Vec<u32>) {
            self.state.a11y_elems(g, self.group, out)
        }
    }

    struct HostFixture;
    impl MachineHost for HostFixture {
        type Arg = plx_ui::fixture::FixtureArg;
        type Fx = plx_ui::fixture::FixtureFx;
        type Msg = plx_ui::fixture::FixtureMsg;
        type Elem = u32;
        type Views<'a> = ();
        type Init = plx_ui::fixture::FixtureInit;
        type Memory = ();
    }

    fn with_cx<R>(entry: EntryId, test: impl FnOnce(&Cx<'_, HostFixture>) -> R) -> R {
        let measure = plx_ui::fixture::FixtureMeasure;
        test(&Cx {
            views: (),
            tick: Tick::default(),
            measure: &measure,
            focus: FocusRead::default(),
            press: PressRead::default(),
            owner: InputOwner::Entry(entry),
            modal: Default::default(), page_image: Default::default(), hold_hint: Default::default(),
        })
    }

    /// A three-row menu (two synthetic quality rungs, then `Stats for nerds`) — mirrors the real
    /// menu's order (Quality leads, Options trails; this module's doc) rather than contradicting
    /// it, built the same way [`MoreMenuState::open_focused`] does but without a `PlaybackSession`
    /// — no row here reads one.
    fn three_row_menu() -> MoreMenuState {
        let mut sec = FormSection::new("Quality");
        for (a, label) in [
            (Action::SetQuality(plx_media::route::Quality::Original), "Rung A"),
            (Action::SetQuality(plx_media::route::Quality::Auto), "Rung B"),
            (Action::ToggleStats, "Stats for nerds"),
        ] {
            sec = sec.item(MoreRow::Act(a), RowKind::Button, a, Row::new(label));
        }
        let mut form = FormTable::with_class(plx_ui::table_screen::BAND_BASE, Class::Action);
        form.set(Form::new().section(sec), None);
        menu_of(form)
    }

    /// A menu showing `form`, with nothing else about it asked of a playback.
    fn menu_of(form: MoreTable) -> MoreMenuState {
        let core = MoreCore {
            rows: Vec::new(),
            forced: false,
            current: plx_media::route::Quality::Auto,
            speed: plx_media::player::RATE_NORMAL,
            speeds: plx_media::player::PLAY_RATES.to_vec(),
            folded: Vec::new(),
        };
        let mut menu = Menu::new(host(crate::player_hud::classic::ctrl_row_hit_rect(), 0), Class::Action, plx_ui::table_screen::BAND_BASE);
        *menu.form_mut() = form;
        MoreMenuState { core, menu }
    }

    /// The focus element of the row at `index` — the keys are identities, not positions.
    fn elem(st: &MoreMenuState, index: usize) -> u32 {
        st.menu.form().key_at(index).expect("a bound row").0
    }

    /// Every row of `form` as the menu projects it: `(id, role)` in row order, after the menu
    /// passes the `a11y` conformance suite (every row projects, ids are unique, the inverse finds
    /// each row and the group lists no more rows than it declares).
    fn projected(form: MoreForm) -> Vec<(String, Role)> {
        plx_media::speed_audio::force_speed_decoder_for_test(true); // a host test loads no FFmpeg
        let e = EntryId(4);
        let mut table = FormTable::with_class(plx_ui::table_screen::BAND_BASE, Class::Action);
        table.set(form, None);
        let st = menu_of(table);
        let part = MoreMenuPart { state: &st, entry: e, group: GroupId(0) };
        with_cx(e, |cx| {
            plx_ui::a11y::conformance(&part, cx);
            let mut keys = Vec::new();
            <MoreMenuPart as Focusable<HostFixture>>::a11y_elems(&part, GroupId(0), cx, &mut keys);
            keys.iter()
                .map(|k| {
                    let a = <MoreMenuPart as Focusable<HostFixture>>::a11y(&part, k, cx).expect("a row projects");
                    (a.path.to_string(), a.role)
                })
                .collect()
        })
    }

    /// **A menu's rows are named by identity** and the menu conforms: the rungs are `rung.<name>`
    /// and the switch `stats`, in row order, whatever keys the rows were given.
    #[test]
    fn rows_are_named_by_identity_and_conform() {
        plx_media::speed_audio::force_speed_decoder_for_test(true); // a host test loads no FFmpeg
        let e = EntryId(4);
        let st = three_row_menu();
        let part = MoreMenuPart { state: &st, entry: e, group: GroupId(0) };
        with_cx(e, |cx| {
            plx_ui::a11y::conformance(&part, cx);
            let at = |i: usize| {
                <MoreMenuPart as Focusable<HostFixture>>::a11y(&part, &elem(&st, i), cx)
                    .map(|a| (a.path.to_string(), a.role))
            };
            assert_eq!(at(0), Some(("rung.original".to_string(), Role::Button)));
            assert_eq!(at(1), Some(("rung.auto".to_string(), Role::Button)));
            assert_eq!(at(2), Some(("stats".to_string(), Role::Button)));
            // the title band's pointer-only key and a key of no row project nothing
            let none = |key: u32| <MoreMenuPart as Focusable<HostFixture>>::a11y(&part, &key, cx).is_none();
            assert!(none(plx_ui::page_stack::TITLE_KEY));
            assert!(none(9999));
            // the inverse is the row's key, and only for a path of this menu
            let find = |text: &str| {
                let path = A11yPath::parse_relative(text).expect("a relative path");
                <MoreMenuPart as Focusable<HostFixture>>::a11y_find(&part, &path, cx)
            };
            assert_eq!(find("rung.auto"), Some(elem(&st, 1)));
            assert_eq!(find("rung.p480"), None);
        });
    }

    /// **Each page of the real forms conforms.** The root names its drill-ins `quality` and `speed`,
    /// a folded command `fold.<command>` and the switch `stats`; the Quality page's rungs are
    /// `rung.<name>` and the Speed page's speeds `rate.<thousandths>`. A drill-in and a choice are a
    /// `Row`, a command a `Button`.
    #[test]
    fn every_page_of_the_menu_names_its_rows() {
        use plx_media::route::Quality;
        plx_media::speed_audio::force_speed_decoder_for_test(true); // a host test loads no FFmpeg
        let ps = plx_media::route::PlaybackSession::default();
        let rows = [Action::SetQuality(Quality::Auto), Action::SetQuality(Quality::P720), Action::ToggleStats];
        let folded = [FoldedRow { command: HudCommand::Previous, label: "Previous" }];
        let rate = plx_media::player::RATE_NORMAL;
        let speeds = plx_media::player::PLAY_RATES;

        let root = projected(root_form(&ps, &rows, false, Quality::Auto, rate, &speeds, &folded));
        let ids: Vec<&str> = root.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, ["quality", "speed", "fold.previous", "stats"]);
        assert_eq!(root[0].1, Role::Row, "a drill-in");
        assert_eq!(root[2].1, Role::Button, "a folded command");

        let quality = projected(quality_form(&ps, &rows, Quality::Auto));
        let ids: Vec<&str> = quality.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, ["rung.auto", "rung.p720"]);
        assert!(quality.iter().all(|(_, role)| *role == Role::Row), "a choice");

        let page = projected(speed_form(&ps, rate, &speeds));
        assert_eq!(page.len(), speeds.len());
        assert!(page.iter().any(|(id, _)| *id == format!("rate.{rate}")));
    }

    /// **UP/DOWN step by one row and clamp at both ends**, over `TableView::next_selectable`,
    /// exercised here through the real `Focusable` dispatch over `HostFixture`.
    #[test]
    fn up_down_step_by_one_and_clamp_at_both_ends() {
        plx_media::speed_audio::force_speed_decoder_for_test(true); // a host test loads no FFmpeg
        let e = EntryId(4);
        let st = three_row_menu();
        let part = MoreMenuPart { state: &st, entry: e, group: GroupId(0) };
        with_cx(e, |cx| {
            let step = |i: u32, dir: Dir| {
                match <MoreMenuPart as Focusable<HostFixture>>::neighbour(
                    &part,
                    FocusKey { entry: e, elem: i },
                    dir,
                    cx,
                ) {
                    Step::Move(k) => Some(k.elem),
                    Step::Edge => None,
                }
            };
            let (r0, r1, r2) = (elem(&st, 0), elem(&st, 1), elem(&st, 2));
            assert_eq!(step(r0, Dir::Down), Some(r1));
            assert_eq!(step(r1, Dir::Down), Some(r2));
            assert_eq!(step(r2, Dir::Down), None, "the last row does not wrap");
            assert_eq!(step(r0, Dir::Up), None, "the first row does not wrap");
            assert_eq!(step(r1, Dir::Up), Some(r0));
            // LEFT/RIGHT are swallowed — this popover is ONE column, matching the old ladder's
            // `Key::Up | Key::Down => …` arm with no Left/Right case at all.
            assert!(matches!(step(elem(&st, 1), Dir::Left), None));
        });
    }

    /// `place` reports exactly the row rect `TableView::row_frame` (and so the old `draw`) would
    /// paint at.
    #[test]
    fn place_matches_the_tables_own_row_frame() {
        plx_media::speed_audio::force_speed_decoder_for_test(true); // a host test loads no FFmpeg
        let e = EntryId(4);
        let st = three_row_menu();
        let r = st.menu.panel_rect(&plx_ui::fixture::FixtureMeasure);
        let want = st.menu.form().table.row_frame(r, 1);
        let part = MoreMenuPart { state: &st, entry: e, group: GroupId(0) };
        with_cx(e, |cx| {
            let placed = <MoreMenuPart as Focusable<HostFixture>>::place(&part, &elem(&st, 1), cx, At::Drawn);
            assert_eq!(placed.map(|p| (p.rect.x, p.rect.y, p.rect.w, p.rect.h)), want.map(|r| (r.x, r.y, r.w, r.h)));
        });
    }

    /// A cursor whose key names no row (a shorter previous row set) settles onto the menu's
    /// OPENING row — a key has no neighbour to slide to — reached through `Focusable::reconcile`.
    #[test]
    fn a_stale_cursor_settles_onto_the_opening_row() {
        plx_media::speed_audio::force_speed_decoder_for_test(true); // a host test loads no FFmpeg
        let e = EntryId(4);
        let st = three_row_menu();
        let part = MoreMenuPart { state: &st, entry: e, group: GroupId(0) };
        with_cx(e, |cx| {
            let got = <MoreMenuPart as Focusable<HostFixture>>::reconcile(
                &part,
                FocusKey { entry: e, elem: 99 },
                cx,
            );
            assert_eq!(got.elem, elem(&st, 0));
            // the menu's own cursor is where focus belongs, whatever the engine last held (a moved focus reaches the
            // cursor first, `Event::Focused`)
            let kept = <MoreMenuPart as Focusable<HostFixture>>::reconcile(
                &part,
                FocusKey { entry: e, elem: elem(&st, 2) },
                cx,
            );
            assert_eq!(kept.elem, elem(&st, 0), "the cursor wins over a live key the menu never saw");
        });
        let mut st = st;
        st.focus_key(elem(&st, 2));
        let part = MoreMenuPart { state: &st, entry: e, group: GroupId(0) };
        with_cx(e, |cx| {
            let kept = <MoreMenuPart as Focusable<HostFixture>>::reconcile(
                &part,
                FocusKey { entry: e, elem: elem(&st, 2) },
                cx,
            );
            assert_eq!(kept.elem, elem(&st, 2), "a row the cursor is on is left where it is");
        });
    }

    /// **Every row fits the panel, in every shipped language** — the root with each possible
    /// current rung read out beside its Quality row, and the Quality page (title band included).
    /// The panel hugs its widest row up to [`MENU_MAX_W`](plx_ui::table::MENU_MAX_W), and a row
    /// elides its label to what the value beside it leaves — Spanish *Estadísticas avanzadas* and
    /// Belarusian *Падрабязная статыстыка* both ended in `…` beside their *Off*, and a rung's value
    /// ("1080p · 20 Mbps", "Original", "Автаматычна") shares its row with the label *Quality*.
    /// Measured with the device's whole-pixel advances.
    #[test]
    fn every_row_fits_the_panel_in_every_language() {
        plx_media::speed_audio::force_speed_decoder_for_test(true); // a host test loads no FFmpeg
        use plx_os::i18n::{language_on_this_thread_for_test, SHIPPED};
        let ps = plx_media::route::PlaybackSession::default();
        let mut out = Vec::new();
        let rows = rows_for(false);
        let mut check = |tag: &str, form: &MoreTable| {
            out.extend(form.table.menu_cap_failure(&plx_base::fontcov::advances::ShippedMeasure, tag));
            out.extend(form.table.app_fit_failures(plx_ui::table::MENU_MAX_W, tag));
            out.extend(form.table.app_fit_failures_hugged(tag));
        };
        for language in SHIPPED {
            let _guard = language_on_this_thread_for_test(language);
            for q in plx_media::route::QUALITY_LADDER {
                let mut root = MoreMenuState::new(&ps);
                root.menu.form_mut().set(root_form(&ps, &rows, false, q, plx_media::player::RATE_NORMAL, &plx_media::player::PLAY_RATES, &[]), None);
                check(language.tag(), root.menu.form());
                let mut page = MoreMenuState::new(&ps);
                page.core.current = q;
                page.core.rows = rows.clone();
                page.focus_key(MoreRow::OpenQuality.key().0);
                page.push_page(&ps, MorePage::Quality);
                check(language.tag(), page.menu.form());
            }
            // the forced root (Options only)
            let forced = MoreMenuState::new(&ps);
            check(language.tag(), forced.menu.form());
            // the Speed page, and the root reading out the longest speed name
            for r in plx_media::player::PLAY_RATES {
                let mut root = MoreMenuState::new(&ps);
                root.menu.form_mut().set(root_form(&ps, &rows, false, plx_media::route::Quality::Auto, r, &plx_media::player::PLAY_RATES, &[]), None);
                check(language.tag(), root.menu.form());
            }
            let mut page = MoreMenuState::new(&ps);
            page.focus_key(MoreRow::OpenSpeed.key().0);
            page.push_page(&ps, MorePage::Speed);
            check(language.tag(), page.menu.form());
            // the root with every command folded, which no package can exceed, under both readings of Go to
            // (a show's and a movie's) and the longest quality readout
            let fold = |command, section| crate::hud_format::Fold { command, section };
            let every = [
                fold("previous", None),
                fold("next", None),
                fold("open_tracks", None),
                fold("toggle_section", Some("chapters")),
                fold("toggle_section", Some("info")),
                fold("from_beginning", None),
                fold("go_to_detail", None),
            ];
            let full = crate::player_hud::model::HudModel {
                controls: crate::player_hud::Controls::for_item(true, true, true),
                chapters: true,
                detail: true,
                ..Default::default()
            };
            let meta = plx_data::stores::metadata::MetadataStore::default();
            let mut folded = folded_rows(&every, &full, meta.view());
            assert_eq!(folded.len(), every.len(), "{}: every command offers a row", language.tag());
            for go in [plx_os::i18n::msg::widgets_info_go_show(), plx_os::i18n::msg::widgets_info_go_movie()] {
                folded[6].label = go;
                for q in plx_media::route::QUALITY_LADDER {
                    let mut root = MoreMenuState::new(&ps);
                    root.menu.form_mut().set(root_form(&ps, &rows, false, q, plx_media::player::RATE_NORMAL, &plx_media::player::PLAY_RATES, &folded), None);
                    check(language.tag(), root.menu.form());
                }
            }
        }
        plx_ui::table::assert_no_fit_failures(&out);
    }

    fn detail_at(st: &mut MoreMenuState, id: MoreRow) -> String {
        let i = st.menu.form().index_of(&id).expect("a row");
        st.menu.form_mut().table.row_mut(i as i32).map(|r| r.detail.clone()).unwrap()
    }

    /// The Quality row's value, and the sub-line under the checked entry on the page, for a fixture.
    fn quality_row(fixture: plx_media::route::EnhTestFixture, rung: Option<plx_media::abr::Rung>) -> (String, String) {
        let (ps, _) = plx_media::route::enhancement_test_session(plx_media::route::EnhTestFixture { rung, ..fixture });
        let current = plx_media::route::quality();
        let mut root = MoreMenuState::new_folded_with_quality(&ps, None, Vec::new(), current, crate::player_hud::classic::ctrl_row_hit_rect());
        let value = super::tests::value_of(&mut root, MoreRow::OpenQuality).expect("a value");
        let mut page = MoreMenuState::new_folded_with_quality(&ps, None, Vec::new(), current, crate::player_hud::classic::ctrl_row_hit_rect());
        page.core.rows = rows_for(false);
        page.focus_key(MoreRow::OpenQuality.key().0);
        page.push_page(&ps, MorePage::Quality);
        let checked_row = MoreRow::Act(Action::SetQuality(current));
        let note = detail_at(&mut page, checked_row);
        plx_media::route::reset_player_control_for_test(&ps);
        plx_plex::plex::reset_servers_for_test();
        plx_media::route::restore_quality(plx_media::route::Quality::Original);
        (value, note)
    }

    /// **The five situations of the UX board's verdict**, one assertion each: the row reads
    /// "{choice} · {playing}", the second part left out only when it would repeat the first, and
    /// the checked entry's sub-line appears only for a conversion the viewer did not pick by name.
    #[test]
    fn the_quality_row_reads_choice_then_what_is_playing() {
        use plx_media::route::{EnhTestFixture, Quality};
        let _g = plx_base::testlock::serial();
        let note = plx_os::i18n::msg::widgets_menu_quality_converted_note();
        let converted = plx_os::i18n::msg::widgets_menu_quality_converted();
        let original = Quality::Original.label();
        let auto = Quality::Auto.label();
        let fixture = |remux, quality| EnhTestFixture { remux, quality, ..Default::default() };
        // 1. Original, the file untouched (a direct play, and a remux that keeps the picture)
        for remux in [None, Some(true)] {
            let (v, n) = quality_row(fixture(remux, Quality::Original), None);
            assert_eq!((v.as_str(), n.contains(note)), (original.as_str(), false), "1 {remux:?}");
        }
        // 2. Original, the picture converted
        let (v, n) = quality_row(fixture(Some(false), Quality::Original), None);
        assert_eq!(v, format!("{original} \u{b7} {converted}"), "2");
        assert_eq!(n, note, "2: the sub-line sits under the checked Original");
        // 3. Auto, the file untouched
        let (v, n) = quality_row(fixture(None, Quality::Auto), None);
        assert_eq!(v, format!("{auto} \u{b7} {original}"), "3");
        assert!(!n.contains(note), "3");
        // 4. Auto on a converted rung
        let (v, n) = quality_row(fixture(Some(false), Quality::Auto), Some(plx_media::abr::Rung::P720));
        assert_eq!(v, format!("{auto} \u{b7} 720p"), "4");
        assert_eq!(n, note, "4");
        // 4b. Auto on a converted route with no rung to name (a burn under Auto, or a picture this
        // television cannot decode): "Auto · Converted", never "Auto" alone
        let (v, n) = quality_row(fixture(Some(false), Quality::Auto), None);
        assert_eq!(v, format!("{auto} \u{b7} {converted}"), "4b");
        assert_eq!(n, note, "4b");
        // burns re-encode the picture, so the row keeps "Original · Converted": Boost on with an
        // embedded subtitle (the enhancement's own burn), and a plain Original burn
        let burn_own = EnhTestFixture {
            subtitle_effect: plx_media::route::SubtitleEffect::Embedded,
            applied: plx_plex::plex::AudioEnhancements { boost_dialog: true, normalize_loudness: false },
            applied_burn: true,
            ..fixture(Some(true), Quality::Original)
        };
        let (v, n) = quality_row(burn_own, None);
        assert_eq!((v, n.as_str()), (format!("{original} \u{b7} {converted}"), note), "own burn");
        let burn_plain = EnhTestFixture {
            subtitle_effect: plx_media::route::SubtitleEffect::Embedded,
            ..fixture(Some(false), Quality::Original)
        };
        let (v, n) = quality_row(burn_plain, None);
        assert_eq!((v, n.as_str()), (format!("{original} \u{b7} {converted}"), note), "plain burn");
        // 5. a fixed 720p: the short name, no sub-line (the page keeps the full label)
        let (v, n) = quality_row(EnhTestFixture { base_present: false, ..fixture(Some(false), Quality::P720) }, None);
        assert_eq!((v.as_str(), n.contains(note)), ("720p", false), "5");
        // a switch in flight: the choice alone
        let (v, _) = quality_row(EnhTestFixture { in_flight: true, ..fixture(Some(false), Quality::Auto) }, None);
        assert_eq!(v, auto, "unsettled");
    }

    /// The converted read-out and note fit the panel in every shipped language.
    #[test]
    fn the_converted_marks_fit_in_every_language() {
        use plx_os::i18n::{language_on_this_thread_for_test, SHIPPED};
        let _g = plx_base::testlock::serial();
        let (ps, _) = plx_media::route::enhancement_test_session(plx_media::route::EnhTestFixture {
            remux: Some(false),
            ..Default::default()
        });
        let rows = rows_for(false);
        let mut out = Vec::new();
        for language in SHIPPED {
            let _guard = language_on_this_thread_for_test(language);
            let root = MoreMenuState::new(&ps);
            let mut page = MoreMenuState::new(&ps);
            page.core.rows = rows.clone();
            page.focus_key(MoreRow::OpenQuality.key().0);
            page.push_page(&ps, MorePage::Quality);
            for form in [root.menu.form(), page.menu.form()] {
                out.extend(form.table.menu_cap_failure(&plx_base::fontcov::advances::ShippedMeasure, language.tag()));
                out.extend(form.table.app_fit_failures(plx_ui::table::MENU_MAX_W, language.tag()));
                out.extend(form.table.app_fit_failures_hugged(language.tag()));
            }
        }
        plx_media::route::reset_player_control_for_test(&ps);
        plx_plex::plex::reset_servers_for_test();
        plx_media::route::restore_quality(plx_media::route::Quality::Original);
        plx_ui::table::assert_no_fit_failures(&out);
    }
}

/// **Classic's settled More panel does not move** when its placement comes from an anchor: right
/// edge `CTRL_RIGHT`, bottom edge 764, from the root and from the Quality page, however it was
/// opened (the `…` disc or the failure read-out's quality entry).
#[cfg(test)]
mod classic_placement_tests {
    use super::*;
    use plx_base::fontcov::advances::ShippedMeasure;
    use plx_ui::theme;

    fn settled() -> Vec<(&'static str, [f32; 4])> {
        plx_media::speed_audio::force_speed_decoder_for_test(true); // a host test loads no FFmpeg
        let _g = plx_base::testlock::serial();
        let ps = plx_media::route::PlaybackSession::IDLE;
        let rect = |st: &MoreMenuState| {
            let r = st.menu.panel_rect(&ShippedMeasure);
            [r.x + r.w, r.y + r.h, r.w, r.h]
        };
        let mut root = MoreMenuState::new(&ps);
        let mut out = vec![("root", rect(&root))];
        root.focus_key(MoreRow::OpenQuality.key().0);
        assert_eq!(root.on_ok(&ps), Out::Moved);
        out.push(("quality page", rect(&root)));
        out.push(("quality entry", rect(&MoreMenuState::new_quality(&ps))));
        out
    }

    #[test]
    fn classic_more_panels_settle_where_they_always_did() {
        // captured from the constants-placed panels (`CTRL_RIGHT - pw`, bottom 764): (right, bottom, width, height)
        assert_eq!(
            settled(),
            [
                ("root", [1824.0, 764.0, 470.0, 284.0]),
                ("quality page", [1824.0, 764.0, 440.0, 536.0]),
                ("quality entry", [1824.0, 764.0, 440.0, 536.0]),
            ],
        );
    }

    fn menu_at(anchor: Rect) -> MoreMenuState {
        plx_media::speed_audio::force_speed_decoder_for_test(true); // a host test loads no FFmpeg
        MoreMenuState::new_folded_with_quality(&plx_media::route::PlaybackSession::IDLE, None, Vec::new(), plx_media::route::Quality::Original, anchor)
    }

    /// The panel opens beside the anchor it was handed, whichever side that is: the same placer and
    /// the same rules as the track menu's.
    #[test]
    fn the_panel_opens_beside_the_anchor_it_was_handed() {
        use plx_ui::popover::Side;
        let _g = plx_base::testlock::serial();
        let gap = theme::layout::PLAYER_MENU_GAP;
        let high = Rect::new(1500.0, 120.0, 324.0, 64.0);
        let st = menu_at(high);
        let r = st.menu.panel_rect(&ShippedMeasure);
        assert_eq!(st.menu.side().expect("a video card has a side"), Side::Below);
        assert_eq!((r.y, r.x + r.w), (high.y + high.h + gap, high.x + high.w));

        let mid = Rect::new(1700.0, 560.0, 64.0, 64.0);
        let st = menu_at(mid);
        let r = st.menu.panel_rect(&ShippedMeasure);
        assert_eq!(st.menu.side().expect("a video card has a side"), Side::Start, "neither 478 above nor 374 below holds NEED_H");
        assert_eq!(r.x + r.w, mid.x - gap);
    }

    /// **The anchor is latched**: a push changes the panel's size, not its side or its edge by the opener.
    #[test]
    fn the_panel_does_not_move_off_its_anchor_while_it_is_open() {
        plx_media::speed_audio::force_speed_decoder_for_test(true); // a host test loads no FFmpeg
        let _g = plx_base::testlock::serial();
        let ps = plx_media::route::PlaybackSession::IDLE;
        let mut st = menu_at(Rect::new(1500.0, 300.0, 324.0, 64.0));
        let first = st.menu.panel_rect(&ShippedMeasure);
        let side = st.menu.side().expect("a video card has a side");
        assert_eq!(side, plx_ui::popover::Side::Below, "the opener is high: the panel hangs under it");
        st.focus_key(MoreRow::OpenQuality.key().0);
        assert_eq!(st.on_ok(&ps), Out::Moved);
        let pushed = st.menu.panel_rect(&ShippedMeasure);
        assert_ne!((pushed.w, pushed.h), (first.w, first.h), "the page changed the panel's size");
        assert_eq!(st.menu.side().expect("a video card has a side"), side);
        assert_eq!((pushed.x + pushed.w, pushed.y), (first.x + first.w, first.y));
    }

    /// `NEED_H` covers the tallest page, the Quality page with its whole ladder, Auto included
    /// (`route::auto_quality_ready` is a const `true`, so the shipping ladder has Auto and the page
    /// is the seven-rung 536 px one), and fits classic's room above the row, so classic opens
    /// upward. A page that outgrew `NEED_H` (a longer label that wraps, an eighth rung) would scroll
    /// an app-owned row on a side with less room.
    #[test]
    fn need_h_covers_the_quality_page_with_auto_and_fits_classics_room() {
        plx_media::speed_audio::force_speed_decoder_for_test(true); // a host test loads no FFmpeg
        let _g = plx_base::testlock::serial();
        let st = MoreMenuState::new_quality(&plx_media::route::PlaybackSession::IDLE);
        assert_eq!(st.page(), Some(MorePage::Quality));
        let auto = Action::SetQuality(plx_media::route::Quality::Auto);
        assert!(st.menu.form().index_of_key(auto.key()).is_some(), "the page measured has the Auto row");
        let tallest = st.menu.form().table.measured_height();
        assert!(tallest > 400.0, "the Quality page was measured: {tallest}");
        assert!(tallest <= NEED_H, "the Quality page is {tallest} px, taller than NEED_H {NEED_H}");
        assert!(NEED_H <= 764.0 - plx_ui::consts::SAFE.y);
    }

    /// The overscan audit's rect is the placer's whole room, the same extent the track menu's audit
    /// takes, and sits inside SAFE.
    #[test]
    fn the_overscan_rect_comes_from_the_placer() {
        let mut out = Vec::new();
        overscan_rects(&mut out);
        let r = out[0].1;
        assert_eq!((r.x + r.w, r.y + r.h, r.w), (1824.0, 764.0, plx_ui::table::MENU_MAX_W));
        assert_eq!(r.y, plx_ui::consts::SAFE.y, "the whole room above the row");
    }
}

/// **The folded rows** (`docs/hud-plugins.md`, R2): which commands More's root lists for a package, in which order,
/// for which content, and what pressing one asks.
#[cfg(test)]
mod folded_tests {
    use super::*;
    use plx_ui::form::FormTable;
    use crate::player_hud::command::{Section, TrackTab};
    use crate::player_hud::loaded::{classic_for_test, load, reference_for_test};
    use crate::player_hud::model::HudModel;
    use crate::player_hud::Controls;
    use plx_media::route::PlaybackSession;
    use std::sync::Arc;

    const MINIMAL: &str = include_str!("hud_format/fixtures/minimal/hud.json");
    const REFERENCE: &str = include_str!("hud_format/fixtures/reference/hud.json");

    fn compiled(text: &str) -> Arc<crate::hud_format::compile::Compiled> {
        load(&crate::hud_format::fixtures::with_hud_resources("reference", &text), "0.0.0-test", plx_os::i18n::english(), &plx_ui::fixture::FixtureMeasure).expect("the fixture loads").compiled
    }

    /// A frame's model for an episode with both neighbours (or a movie with none), chapters and a detail page.
    fn model(episode: bool) -> HudModel {
        HudModel { controls: Controls::for_item(episode, true, true), chapters: true, detail: true, ..HudModel::default() }
    }

    fn rows_of(package: &Arc<crate::hud_format::compile::Compiled>, model: &HudModel) -> Vec<FoldedRow> {
        folded_rows(package.folds(), model, plx_data::stores::metadata::MetadataStore::default().view())
    }

    fn commands(rows: &[FoldedRow]) -> Vec<HudCommand> {
        rows.iter().map(|r| r.command).collect()
    }

    /// The commands of the Hud rows showing, in table order, off a built menu.
    fn hud_order(st: &MoreMenuState) -> Vec<HudCommand> {
        (0..st.menu.form().table.n_rows() as usize)
            .filter_map(|i| match st.menu.form().id_at(i) {
                Some(MoreRow::Act(Action::Hud(command))) => Some(*command),
                _ => None,
            })
            .collect()
    }

    /// **Classic folds nothing, and its root is the one it always had**: no folded row for an episode or a movie,
    /// and the menu built with the (empty) list shows the same rows, in the same order with the same keys and
    /// labels, as the one built without any.
    #[test]
    fn classic_folds_nothing_and_its_root_is_unchanged() {
        plx_media::speed_audio::force_speed_decoder_for_test(true); // a host test loads no FFmpeg
        let ps = PlaybackSession::default();
        let classic = classic_for_test();
        for episode in [true, false] {
            let rows = rows_of(&classic, &model(episode));
            assert!(rows.is_empty(), "episode {episode}: {rows:?}");
            let mut with = MoreMenuState::new_folded(&ps, None, rows);
            let mut without = MoreMenuState::new(&ps);
            assert_eq!(with.keys(), without.keys(), "the same rows in the same order");
            assert!(with.keys().iter().all(|k| !(30..=43).contains(k)), "no folded key on classic's root");
            let labels = |st: &mut MoreMenuState| (0..st.menu.form().table.n_rows()).map(|i| st.menu.form_mut().table.row_mut(i).map(|r| r.label.clone())).collect::<Vec<_>>();
            assert_eq!(labels(&mut with), labels(&mut without));
            assert_eq!(with.menu.form().table.measured_height(), without.menu.form().table.measured_height());
        }
    }

    /// **A package with no previous or next folds both, a movie shows neither**: the rows are the app's own
    /// predicate (`HudCommand::enabled`), so a movie gets no dead pair.
    #[test]
    fn the_pair_folds_for_an_episode_and_a_movie_gets_neither() {
        let minimal = compiled(MINIMAL);
        let episode = commands(&rows_of(&minimal, &model(true)));
        assert!(episode.starts_with(&[HudCommand::Previous, HudCommand::Next]), "{episode:?}");
        let movie = commands(&rows_of(&minimal, &model(false)));
        assert!(!movie.contains(&HudCommand::Previous) && !movie.contains(&HudCommand::Next), "{movie:?}");
        assert!(movie.contains(&HudCommand::OpenTracks(TrackTab::Subtitles)), "the rest still folds: {movie:?}");
        // an episode with only a previous neighbour has no Next row either
        let only_previous = HudModel { controls: Controls::for_item(true, true, false), ..model(true) };
        let rows = commands(&rows_of(&minimal, &only_previous));
        assert!(rows.contains(&HudCommand::Previous) && !rows.contains(&HudCommand::Next), "{rows:?}");
    }

    /// **The reference folds `from_beginning` and `go_to_detail`**, labelled with the Info card's own button
    /// strings, and nothing else; `open_tracks` does not fold because its Subtitles tab is placed.
    #[test]
    fn the_reference_folds_the_two_info_actions() {
        let reference = reference_for_test();
        let meta = plx_data::stores::metadata::MetadataStore::default();
        let rows = rows_of(&reference, &model(true));
        assert_eq!(commands(&rows), [HudCommand::FromBeginning, HudCommand::GoToDetail]);
        let actions = crate::info_panel::actions(meta.view());
        assert_eq!(rows.iter().map(|r| r.label).collect::<Vec<_>>(), actions, "the Info actions' own strings");
        // a playing item with no page to go to has no Go to row
        let no_page = HudModel { detail: false, ..model(true) };
        assert_eq!(commands(&rows_of(&reference, &no_page)), [HudCommand::FromBeginning]);
    }

    /// **The rows sit after Quality and Speed and before Options**, in the compiler's order, and Stats for nerds
    /// is the Options row it always was (a command with its own row never folds).
    #[test]
    fn the_rows_follow_quality_and_speed_and_precede_options() {
        plx_media::speed_audio::force_speed_decoder_for_test(true); // a host test loads no FFmpeg
        let ps = PlaybackSession::default();
        let rows = rows_of(&compiled(MINIMAL), &model(true));
        let st = MoreMenuState::new_folded(&ps, None, rows.clone());
        assert_eq!(hud_order(&st), commands(&rows), "the compiler's order");
        let keys = st.keys();
        let at = |key: u32| keys.iter().position(|k| *k == key).unwrap_or_else(|| panic!("no row {key} in {keys:?}"));
        let (first, last) = (at(Action::Hud(rows[0].command).key().0), at(Action::Hud(rows[rows.len() - 1].command).key().0));
        assert!(at(MoreRow::OpenSpeed.key().0) < first, "after Speed");
        assert!(last < at(Action::ToggleStats.key().0), "before Options");
        assert_eq!(keys.iter().filter(|k| **k == Action::ToggleStats.key().0).count(), 1, "Stats for nerds is not folded twice");
        assert!(commands(&rows).iter().all(|c| *c != HudCommand::ToggleStats));
    }

    /// **A folded row commits its own command**, addressed by identity: OK on each reports `Action::Hud(command)`
    /// and the container performs that command.
    #[test]
    fn a_folded_row_reports_its_own_command() {
        plx_media::speed_audio::force_speed_decoder_for_test(true); // a host test loads no FFmpeg
        let ps = PlaybackSession::default();
        let mut st = MoreMenuState::new_folded(&ps, None, rows_of(&compiled(MINIMAL), &model(true)));
        let wanted = hud_order(&st);
        assert!(wanted.len() >= 5, "{wanted:?}");
        for command in wanted {
            assert!(st.focus_key(Action::Hud(command).key().0));
            assert_eq!(st.on_ok(&ps), Out::Picked(Action::Hud(command)), "{command:?}");
        }
    }

    /// **BACK from a page opened directly lands on the root with its folded rows**, as it lands on the root today:
    /// the page is pushed from its opener's row, so popping it restores the root the menu would have shown.
    #[test]
    fn back_from_a_page_opened_directly_shows_the_folded_root() {
        plx_media::speed_audio::force_speed_decoder_for_test(true); // a host test loads no FFmpeg
        let ps = PlaybackSession::default();
        let rows = rows_of(&compiled(MINIMAL), &model(true));
        let mut st = MoreMenuState::new_folded(&ps, Some(MorePage::Speed), rows.clone());
        assert_eq!(st.page(), Some(MorePage::Speed));
        assert!(st.pop(&ps), "the page pops");
        assert_eq!(st.page(), None);
        assert_eq!(hud_order(&st), commands(&rows));
        assert_eq!(st.sel_id(), Some(MoreRow::OpenSpeed), "focus is back on the row that opened the page");
    }

    /// Every row keeps its own focus key: no two actions of any kind share one (the replay canon writes the key).
    #[test]
    fn every_action_has_its_own_key() {
        use plx_media::route::QUALITY_LADDER;
        let mut all: Vec<Action> = vec![Action::None, Action::ToggleStats, Action::SendDiagnostics];
        all.extend(QUALITY_LADDER.iter().map(|q| Action::SetQuality(*q)));
        all.extend(plx_media::player::PLAY_RATES.iter().map(|r| Action::SetSpeed(*r)));
        all.extend([
            HudCommand::Previous,
            HudCommand::Next,
            HudCommand::OpenTracks(TrackTab::Subtitles),
            HudCommand::OpenTracks(TrackTab::Audio),
            HudCommand::ToggleSection(Section::Chapters),
            HudCommand::ToggleSection(Section::Info),
            HudCommand::FromBeginning,
            HudCommand::GoToDetail,
            HudCommand::TogglePlay,
            HudCommand::OpenMore(crate::player_hud::command::MorePage::Root),
            HudCommand::ToggleStats,
        ].map(Action::Hud));
        let mut keys: Vec<(u32, Action)> = all.iter().map(|a| (a.key().0, *a)).collect();
        keys.push((MoreRow::OpenQuality.key().0, Action::None));
        keys.push((MoreRow::OpenSpeed.key().0, Action::None));
        // the two tab addresses of `open_tracks` are one row, so one key
        keys.retain(|(_, a)| *a != Action::Hud(HudCommand::OpenTracks(TrackTab::Audio)));
        let mut seen = std::collections::BTreeMap::new();
        for (key, action) in keys {
            if let Some(other) = seen.insert(key, action) {
                panic!("key {key} is {other:?} and {action:?}");
            }
        }
    }

    /// The three numbers [`need_h`] is built from are the real table's: the root at its tallest standard shape, with
    /// zero to seven folded rows (the most a package can fold), is as tall as the formula says.
    #[test]
    fn captured_preference_quality_opens_and_refreshes_from_owner_inputs() {
        use plx_media::route::{PlaybackSession, Quality};
        let ps = PlaybackSession::IDLE;
        let mut a = MoreMenuState::new_folded_with_quality(&ps, None, Vec::new(), Quality::P720, crate::player_hud::classic::ctrl_row_hit_rect());
        let b = MoreMenuState::new_folded_with_quality(&ps, None, Vec::new(), Quality::Original, crate::player_hud::classic::ctrl_row_hit_rect());
        assert_eq!(a.core.current, Quality::P720);
        assert_eq!(b.core.current, Quality::Original);
        assert!(a.refresh_with_quality(&ps, Quality::P1080));
        assert_eq!(a.core.current, Quality::P1080);
        assert_eq!(b.core.current, Quality::Original);
        assert!(!a.refresh_with_quality(&ps, Quality::P1080));
    }

    #[test]
    fn root_height_is_the_tables() {
        plx_media::speed_audio::force_speed_decoder_for_test(true); // a host test loads no FFmpeg
        let ps = PlaybackSession::default();
        let all = [
            HudCommand::Previous,
            HudCommand::Next,
            HudCommand::OpenTracks(TrackTab::Subtitles),
            HudCommand::ToggleSection(Section::Chapters),
            HudCommand::ToggleSection(Section::Info),
            HudCommand::FromBeginning,
            HudCommand::GoToDetail,
        ];
        let rows = vec![
            Action::SetQuality(plx_media::route::Quality::Auto),
            Action::SetQuality(plx_media::route::Quality::P720),
            Action::ToggleStats,
            Action::SendDiagnostics,
        ];
        for n in 0..=all.len() {
            let folded: Vec<FoldedRow> = all[..n].iter().map(|&command| FoldedRow { command, label: "Previous Episode" }).collect();
            let mut form = FormTable::with_class(plx_ui::table_screen::BAND_BASE, Class::Action);
            let root = root_form(&ps, &rows, false, plx_media::route::Quality::Auto, plx_media::player::RATE_NORMAL, &plx_media::player::PLAY_RATES, &folded);
            form.set_or_open(root, None);
            let formula = if n == 0 { ROOT_BASE_H } else { ROOT_BASE_H + FOLDED_SECTION_H + FOLDED_ROW_H * n as f32 };
            assert_eq!(form.table.measured_height(), formula, "{n} folded rows");
            assert_eq!(need_h(n), NEED_H.max(formula), "{n}");
        }
    }
}
