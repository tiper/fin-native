//! The **item context menu** — a registered `Style::Compact` surface on the shared `ModalStack`,
//! opened by a press-and-hold on a card (or on the detail page's episode still / season tab).
//!
//! The reference is **Apple TV's card menu, not Plex's**: a panel anchored BESIDE the focused card
//! (the card and the rest of the shelf stay where they are, visible behind it), rows of
//! `[leading icon] [label]`, the focused row a filled light pill spanning the panel, and a hairline
//! separator between the navigation actions and the state actions. Plex's own version is a
//! full-screen sheet; that is deliberately not what this is.
//!
//! It exists because OK on a Continue Watching tile can **resume immediately** (the Playback
//! setting [`DeckPress::Play`]; the play glyph in the focused still's footer is the affordance that says so).
//! The hold is the *other* half of that interaction, and until it existed Go-to-Show, per-item
//! Mark-as-Watched and Play-from-Start had nowhere to live (`docs/parity-gaps.md` §1.2/§1.3, §5a).
//! Under the default, [`DeckPress::Details`], OK opens the page instead and the deck card's menu
//! leads with the Play row that press gave up (and drops the Go to Movie row it now duplicates; on
//! an episode OK opens the show, so Go to Episode stays and Go to Show goes).
//!
//! **Every card surface opens it** — a home shelf, the Library browse grid, a Search result shelf,
//! a person's filmography and the detail page's Related shelf present [`ItemMenuKind::Card`]; the
//! detail page's episode filmstrip and season tabs present [`ItemMenuKind::Episode`] /
//! [`ItemMenuKind::Season`]. The row SET differs only because a navigation row that leads to the
//! page you are standing on is not an action; everything else — the panel, the placement, the
//! choreography, the state rows — is shared.
//! The full Filmography table uses [`ItemMenuKind::WatchlistAdd`]: one Add to Watchlist row,
//! addressed by Discover catalog identity even when no server has a copy.
//!
//! **What remains excluded is excluded for a reason that does not dissolve.** A tile that is a
//! PERSON or a TAG has no rating key and no watch state at all, so every row this module can build
//! would be absent and the hold would open an empty panel: the detail page's cast headshots, and
//! Search's Cast & Crew rows (`search::Item::Tag` has no rating key). A collection has a rating
//! key but no watch state and no row here either ([`has_actions`] refuses it), so a hold on a
//! Collections tile opens the collection page as OK does.
//!
//! **Navigation owns its lifetime, phase and input scope** (restructure phase 10). It was
//! `ui/item_menu.rs` — a `Popover` plus six `static mut`s (`POP`, `TABLE`, `ACTS`, `OPENER`,
//! `ITEM`, `SID`) driven by `Route::ItemMenu { over: MenuHost }` — and every one of those statics
//! is a field of the entry's argument or of the instance now: the ROW the menu is about and the
//! anchor it hangs off travel on [`ItemMenuArg`], the built rows and their actions are the
//! screen's own. The six-variant `MenuHost` is down to the two BITS an action actually reads
//! (`loaded_episode`, `from_home`), which is all it was still deciding once the route it named
//! stopped existing.
//!
//! **The panel is `plx_ui::menu::Menu`'s; this module supplies rows.** The stack, the card's resize and the
//! page slide, the placement, the glass, the stops, the pointer hold and the dim are the menu's, chosen by its
//! [`plx_ui::menu::Host`]: a menu opened from a HERO control grows out of it, one opened from a tile stands
//! beside its card.
//!
//! **A menu opened from a HERO control grows out of it.** The Detail hero's `…` ([`ItemMenuKind::HeroOverflow`])
//! is a control, not a card: the panel is the control's own glass (`Host::Grows`, built by
//! `plx_ui::detail_layout::hero_host`), laid over it clear of the synopsis whenever the page fits, the page
//! leaves the control out of its pass and the opener lift draws it at the morph's `opener_alpha`, and BACK at
//! its rows or a pick drains the glass back into it. **Select Version and Audio & Subtitles are PAGES of that
//! glass**: their rows are `Nav` rows, so OK and RIGHT push the page onto the menu's one page stack, the card
//! springs to the page's rect and the pages slide in it, and BACK and LEFT pop to the rows with the ring on
//! the row that opened the page (LEFT at the rows is inert); every pushed page wears the header with the back
//! mark. Audio & Subtitles pushes Audio or Subtitles further in the same glass. The page content is the code
//! the standalone panels use (`plx_appkit::version_list`, `plx_appkit::preplay_tracks`), which stay the round
//! fourth-slot button's panel and the failure screen's. A pick in Select Version closes the whole menu (the
//! page selects it); a track pick keeps the menu up with the check moved. A menu opened from a TILE (every
//! other kind) has no glass of its own to grow: the panel stands beside the card (`Host::BesideGlass`), the
//! lift is whole, and its Select Version row pushes the same page onto the same stack. The branch is the
//! opener, which is `HeroOverflow`'s identity, held as the presence of its [`Hero`] state.
//!
//! It still only **reports** the chosen [`Action`]: `AppFx::ItemMenu` carries it to
//! `app::input::apply_item_action`, which does the routing, the server call and the hub refresh.
//! It never mutates playback or PMS state itself.

/// The Watchlist grid's hold-menu rows (`ItemMenuKind::Catalog`, plan §1.9).
pub mod hold;
/// The Watchlist rows of the card / episode / season menus and the Catalog menu's membership row.
pub mod watchlist;

use std::borrow::Cow;
use std::os::raw::c_int;

use plx_data::pms::{MediaVersion, PmsMovie};
use plx_media::route::DeckPress;
use crate::registry::{tile_facts, RepeatGate, PANEL_REPEAT_MS};
use crate::registry::{commit_pick, hero_face, AppFx, AppLike, AppMsg, HeroCtl, ItemMenuArg, ItemMenuKind, ItemMenuReq, PreplaySource};
use plx_ui::form::{Form, FormId, FormSection, FormTable, Rebind, RowKey, RowKind};
use plx_ui::icons::Icon;
use plx_machine::machine::{
    Canon, Cx, Edge, Effects, EntryId, FocusKey, Fx, GroupId, Handled, InputKind, Key,
    LogicalState, Machine, NavOp, MachineId, Delivery,
};
use plx_ui::a11y::{A11y, A11yPath, Role};
use plx_ui::menu::{Class, Event, Host, Menu, Out, Page};
use plx_ui::screen::{
    At, Dir, Focusable, GroupSpec, Placed, RenderStrategy, Screen, ScreenEvent, Step,
};
use plx_appkit::preplay_tracks::{self as tracks, Pick, PickPage, PickRow, PickerCore};
use plx_ui::table::Row;
use plx_ui::widgets::PosterMark;
use plx_ui::consts::{CARD_H, CARD_W, SCR_H, SCR_W};
use plx_ui::table::MENU_MIN_W;
use plx_ui::Rect;

/// What the highlighted row does on OK. Every variant carries the identity it needs, captured when
/// the menu opened — a hub refetch can re-order the catalog underneath an open panel, so nothing
/// here is an index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// open this leaf's own detail page (an episode's page, a movie's page)
    GoToItem(String),
    /// open the SHOW page with that season selected (`season <= 0` = no season to select)
    GoToShow(String, c_int),
    /// `/:/scrobble` — the ✓ row. **The verb is the ROW's, never the item's state read back**:
    /// there is no bool here to invert, because a part-watched item offers BOTH rows and has no
    /// single "other end" to derive. See [`Action::watch_write`].
    MarkWatched(String),
    /// `/:/unscrobble` — the − row. Twin of [`Action::MarkWatched`].
    MarkUnwatched(String),
    /// play this item — a leaf resumes where it left off when it has a resume point; a show or
    /// season takes `activate_card`'s Play path (open its page, fire that page's Play) — the
    /// Continue Watching card's first row while OK on that card opens the page
    /// ([`DeckPress::Details`])
    Play(String),
    /// play this leaf ignoring its resume point
    PlayFromStart(String),
    /// play the item's already-loaded trailer extra — never the parent, never a PMS fetch
    PlayTrailer {
        rk: String,
        part: String,
        vcodec: String,
        acodec: String,
        title: String,
    },
    /// hide this item from the Continue Watching deck — the server-side
    /// `removeFromContinueWatching`. **Keeps the resume point**: it is a hide, not a reset, so
    /// playing the item again picks up where it left off (see
    /// `plex::Client::remove_from_continue_watching`).
    RemoveFromDeck(String),
    /// Watchlist hold menu: open the catalog page of this discover guid (a title with no playable
    /// copy). The app routes it to `ContentArg::Catalog` (`app::input::apply_item_action`).
    GoToCatalog(String),
    /// Watchlist hold menu: take this discover guid off the Watchlist (`DELETE` on discover).
    WatchlistRemove(String),
    /// Add the title with this catalog id (the 12 bytes of its discover guid) to the Watchlist.
    /// Hex-keyed, identity captured when the menu opened; the app queues the write and owns the
    /// tab pulse. (`GoToCatalog` / `WatchlistRemove` above stay discover-guid keyed: the hex is
    /// the guid's tail, `plx_data::watchlist::decode_hex`.)
    WatchlistAdd { hex: [u8; 12] },
    /// Select Version…: turn the panel into the list of this leaf's versions. The menu performs
    /// it itself (`ItemMenuScreen::activate`); it never reaches the app.
    ChooseVersion(String),
    /// A hero action under the Detail page's … menu: the row IS the round button it replaces, so the
    /// action is that [`HeroCtl`]. The menu closes and hands it back to the page that opened it
    /// (`AppMsg::HeroCtl`), whose `activate_hero` performs it; it never reaches the app. (Select
    /// Version and Audio & Subtitles push a page of the menu instead.)
    Hero(HeroCtl),
    /// A row of the Audio & Subtitles pages the hero's `…` menu hosts: the SAME rows the standalone
    /// panel lists (`plx_appkit::preplay_tracks`), by their identity there. A drill-in row pushes its
    /// page, a track row is a pick; both are the menu's own (`ItemMenuScreen::activate_hero_menu`).
    Pick(PickRow),
    /// Select this leaf's version `media` as the file Play and Resume start. It plays nothing; the
    /// app remembers it for the profile (`plx_media::route::choose_version`). The id, never a
    /// position: it is resolved against the item when it is played
    /// (`plx_plex::plex::Metadata::media_version`).
    SelectVersion { rk: String, media: plx_plex::plex::MediaId },
}

impl Action {
    /// The view-state write this action performs, if it is one of the two that do — the item-menu
    /// twin of `detail::HeroAction::watch_write`, and the reason this enum carries two variants
    /// rather than one flag.
    ///
    /// The old shape was `MarkWatched(rk, watched)` where the bool was **what the item is NOW**, and
    /// `apply_item_action` inverted it at the press. That works for exactly as long as the menu
    /// emits one row: with this menu's part-watched PAIR both rows would carry the same bool, so one
    /// would invert to its neighbour's write and silently do the opposite of its own label. Now the
    /// glyph the user aimed at IS the verb, and nothing downstream re-reads the item.
    pub fn watch_write(&self) -> Option<plx_data::viewstate::Write> {
        match self {
            Action::MarkWatched(_) => Some(plx_data::viewstate::Write::Watched),
            Action::MarkUnwatched(_) => Some(plx_data::viewstate::Write::Unwatched),
            _ => None,
        }
    }

    /// Does the action name what it acts on? A ratingKey-keyed action needs a non-empty key; a
    /// catalog-keyed [`Action::WatchlistAdd`] carries its 12-byte id instead and has no key.
    pub fn has_subject(&self) -> bool {
        matches!(self, Action::WatchlistAdd { .. } | Action::Hero(_)) || !self.rk().is_empty()
    }

    /// The ratingKey this action acts on.
    pub fn rk(&self) -> &str {
        match self {
            Action::GoToItem(rk)
            | Action::GoToShow(rk, _)
            | Action::MarkWatched(rk)
            | Action::MarkUnwatched(rk)
            | Action::Play(rk)
            | Action::PlayFromStart(rk)
            | Action::RemoveFromDeck(rk)
            | Action::GoToCatalog(rk)
            | Action::WatchlistRemove(rk)
            | Action::ChooseVersion(rk) => rk,
            Action::SelectVersion { rk, .. } => rk,
            Action::PlayTrailer { rk, .. } => rk,
            // No ratingKey: the subject is a catalog id, or (a hero control) the page that opened the menu.
            Action::WatchlistAdd { .. } | Action::Hero(_) | Action::Pick(_) => "",
        }
    }

    fn write(&self, c: &mut Canon) {
        // The tags are the recorded shape's (`SHAPE`); 0 was the retired `None` sentinel.
        let tag = match self {
            Action::GoToItem(_) => 1,
            Action::GoToShow(..) => 2,
            Action::MarkWatched(_) => 3,
            Action::MarkUnwatched(_) => 4,
            Action::PlayFromStart(_) => 5,
            Action::RemoveFromDeck(_) => 6,
            Action::PlayTrailer { .. } => 7,
            Action::Play(_) => 8,
            Action::GoToCatalog(_) => 9,
            Action::WatchlistRemove(_) => 10,
            Action::WatchlistAdd { .. } => 11,
            Action::ChooseVersion(_) => 12,
            // Tag 13 was `PlayVersion` when it was recorded; the wire is unchanged by what a press MEANS.
            Action::SelectVersion { .. } => 13,
            Action::Hero(_) => 14,
            Action::Pick(_) => 15,
        };
        c.u32(tag).str(self.rk());
        if let Action::WatchlistAdd { hex } = self {
            c.str(&plx_data::watchlist::encode_hex(hex));
        }
        if let Action::GoToShow(_, season) = self {
            c.u32(*season as u32);
        }
        if let Action::SelectVersion { media, .. } = self {
            c.u64(media.0 as u64);
        }
        if let Action::Hero(ctl) = self {
            c.u32(ctl.elem());
        }
        if let Action::Pick(row) = self {
            c.u32(row.key().0);
        }
        if let Action::PlayTrailer {
            part,
            vcodec,
            acodec,
            title,
            ..
        } = self
        {
            c.str(part).str(vcodec).str(acodec).str(title);
        }
    }
}

/// `media:u64(PlayVersion only)` is the pinned spelling of tag 13's payload, which `Action::SelectVersion` keeps:
/// renaming what a press MEANS changes neither the canonical bytes nor the shape (`SCREEN_SHAPES_PIN`).
pub const SHAPE: &str =
    "ItemMenu{arg:ItemMenuArg,acts:[Option<Action{tag:u32,rk:str,season:u32,part:str,vcodec:str,acodec:str,title:str,hex:str(WatchlistAdd only),media:u64(PlayVersion only),ctl:u32(Hero only),pick:u32(Pick only)}>],sel:i32,depth:u32,\
     table:TableViewMotion}";

/// Is `m` an item the menu has anything to offer? A leaf or a show/season — i.e. everything the
/// home shelves carry. Kept as a predicate so the caller can decline to present an empty panel.
pub fn has_actions(m: &PmsMovie) -> bool {
    m.kind != plx_data::pms::KIND_COLLECTION && !m.rk.is_empty()
}

/// A menu row's identity: which of the seven rows it is. Hand-assigned keys, never a position —
/// the row SET varies by item kind (three item kinds, an optional `Go to Show`, an optional deck
/// row, a state group shared with two other entry points), so a position would name a different
/// row in every menu, and reordering a menu would move focus keys. A row is declared once, with
/// its action, so a row cannot exist without one and a press cannot perform its neighbour's.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ItemRow {
    GoToItem,
    GoToShow,
    MarkWatched,
    MarkUnwatched,
    PlayFromStart,
    PlayTrailer,
    RemoveFromDeck,
    Play,
    /// Watchlist hold menu: the catalog page.
    GoToCatalog,
    /// Watchlist hold menu: Remove from Watchlist (destructive).
    WatchlistRemove,
    /// Add to Watchlist (movie) / Add Show to Watchlist (episode, catalog page menu).
    WatchlistAdd,
    /// Play Version… (a leaf with two or more versions).
    PlayVersion,
    /// A hero action under the Detail page's … menu, by the control it replaces.
    Hero(HeroCtl),
    /// One version on the Play Version page, by its position in the list captured at open.
    Version(u16),
    /// A row of the Audio & Subtitles pages, by its identity there ([`PickRow`]).
    Pick(PickRow),
}

impl ItemRow {
    /// The focus element of this row (for a test, or a probe, that addresses it by identity).
    #[cfg(any(test, feature = "test-support"))]
    pub fn focus_key(self) -> u32 {
        self.key().0
    }
}

impl ItemRow {
    /// This row's accessibility name (`itemmenu.<name>`), a static word the way its key is a static
    /// number. A `ItemRow::Version` is named by its position instead (`itemmenu.version.<n>`).
    fn a11y_name(self) -> &'static str {
        match self {
            ItemRow::GoToItem => "go_to_item",
            ItemRow::GoToShow => "go_to_show",
            ItemRow::MarkWatched => "mark_watched",
            ItemRow::MarkUnwatched => "mark_unwatched",
            ItemRow::PlayFromStart => "play_from_start",
            ItemRow::PlayTrailer => "play_trailer",
            ItemRow::RemoveFromDeck => "remove_from_deck",
            ItemRow::Play => "play",
            ItemRow::GoToCatalog => "go_to_catalog",
            ItemRow::WatchlistRemove => "watchlist_remove",
            ItemRow::WatchlistAdd => "watchlist_add",
            ItemRow::PlayVersion => "play_version",
            // Named by its control in `a11y` (`itemmenu.hero.<ctl>`), which reads the control itself.
            ItemRow::Hero(_) => "hero",
            ItemRow::Version(_) => "version",
            ItemRow::Pick(_) => "pick",
        }
    }
}

impl FormId for ItemRow {
    fn key(&self) -> RowKey {
        RowKey(match self {
            ItemRow::GoToItem => 1,
            ItemRow::GoToShow => 2,
            ItemRow::MarkWatched => 3,
            ItemRow::MarkUnwatched => 4,
            ItemRow::PlayFromStart => 5,
            ItemRow::PlayTrailer => 6,
            ItemRow::RemoveFromDeck => 7,
            ItemRow::Play => 8,
            ItemRow::GoToCatalog => 9,
            ItemRow::WatchlistRemove => 10,
            ItemRow::WatchlistAdd => 11,
            ItemRow::PlayVersion => 12,
            ItemRow::Hero(ctl) => HERO_KEY_BASE + ctl.elem(),
            ItemRow::Version(n) => VERSION_KEY_BASE + u32::from(*n),
            ItemRow::Pick(row) => PICK_KEY_BASE + row.key().0,
        })
    }
}

/// The Play Version page's row keys start here, clear of the fixed rows above.
const VERSION_KEY_BASE: u32 = 100;
/// The Audio & Subtitles pages' row keys start here, clear of every other page's (a page's own keys,
/// `PickRow::key`, are below 3000).
const PICK_KEY_BASE: u32 = 10_000;
/// The hero overflow rows' keys start here: the control's own fixed identity ([`HeroCtl::elem`]) past it.
const HERO_KEY_BASE: u32 = 1000;

/// **Does OK on this Continue Watching card, while OK does not play, open the SHOW's page?** Only
/// an episode that names its show. `app::input::activate_deck_card` decides the press with it and
/// [`build_with`] decides the menu with it, so the card's press and the menu's "Go to Episode" /
/// "Go to Show" rows cannot disagree about where OK goes. A movie, season or show card, and an
/// episode whose row arrived without a `grandparentRatingKey`, keep their own page.
pub fn deck_episode_opens_show(m: &PmsMovie) -> bool {
    m.kind == 3 && !m.show_rk.is_empty()
}

/// What every item-menu builder returns: a form whose rows' actions are the whole of what a press means,
/// except a `Nav` row, which pushes the [`MenuPage`] it names (Select Version, Audio & Subtitles).
type ItemForm = Form<ItemRow, Action, MenuPage>;

/// The rows, and the action each one commits. Order is the pinned design's:
/// navigation (`Go to Episode` · `Go to Show`) — separator — state (the watch row or ROWS ·
/// `Play from Start`), adapted per item kind (`PmsMovie::kind`: 0 movie / 1 show / 2 season /
/// 3 episode; 4 collection is refused by [`has_actions`], which every caller asks first). The
/// state group is one row or two off [`state_rows`], so this list has no fixed length.
#[cfg(test)]
fn build(m: &PmsMovie, from_deck: bool) -> ItemForm {
    build_with(m, from_deck, DeckPress::Play, None)
}

/// [`build_card`] without the Watchlist's Remove row.
#[cfg(test)]
fn build_with(
    m: &PmsMovie,
    from_deck: bool,
    deck_press: DeckPress,
    trailer: Option<&plx_data::metadata::Extra>,
) -> ItemForm {
    build_card(m, from_deck, deck_press, trailer, None, None)
}

/// `deck_press` is the Continue Watching setting. It changes this menu for a card FROM the deck
/// only, and only while OK there opens the page ([`DeckPress::Details`]): the menu then leads with
/// Play (the opening row — the press that used to play now lives here) and drops the row that
/// navigates to the page OK on the card already opens (Go to Movie, and for a show or season its
/// Go to Show / Season). **An episode is the exception**: OK on a deck episode opens its SHOW
/// ([`deck_episode_opens_show`]), so there the menu keeps Go to Episode and drops Go to Show; an
/// episode with no show row has OK open its own page, and loses Go to Episode as before. Any
/// other card, and a deck card while OK plays, gets the menu it always had.
///
/// `watchlist_guid` is set for the Watchlist grid's hold menu (plan §1.9, "Card kind + Remove"):
/// the destructive Remove from Watchlist row follows the card's own rows.
fn build_card(
    m: &PmsMovie,
    from_deck: bool,
    deck_press: DeckPress,
    trailer: Option<&plx_data::metadata::Extra>,
    watchlist_guid: Option<&str>,
    face: Option<&watchlist::Face>,
) -> ItemForm {
    let leaf = m.kind == 0 || m.kind == 3;
    let opens_details = from_deck && !deck_press.press_plays();
    let mut sec = FormSection::new(""); // no header: the card behind the panel IS the title

    // ---- the deck card's own press, when OK no longer plays ----
    // First, so it is where the menu opens. It resumes when there is a resume point and the label
    // says which, in the words the hero's pill uses.
    // A show or season has no resume point of its own, so it says "Play".
    let resumes = leaf && plx_data::metadata::resume_ns(m.resume_ms, m.dur_ns / 1_000_000) > 0;
    sec = sec.item_if(
        opens_details && (leaf || m.kind == 1 || m.kind == 2),
        ItemRow::Play,
        RowKind::Button,
        Action::Play(m.rk.clone()),
        Row::new(plx_appkit::hero_layout::play_text(resumes))
        .licon(Icon::Play),
    );

    // ---- navigation: this tile's own page, then the show it belongs to ----
    // A row whose TARGET is missing is simply absent: a hub row can arrive without a
    // grandparentRatingKey, and a "Go to Show" that resolves to an empty rk would fire a blocking
    // fetch for nothing and land the user on a blank page.
    let has_show = !m.show_rk.is_empty();
    let go_item = |label: &'static str| {
        (ItemRow::GoToItem, Action::GoToItem(m.rk.clone()), Row::new(label).licon(Icon::Episode))
    };
    let go_show = |label: &'static str, rk: &str, season: c_int| {
        (ItemRow::GoToShow, Action::GoToShow(rk.to_string(), season), Row::new(label).licon(Icon::Show))
    };
    let mut nav = Vec::new();
    match m.kind {
        3 => {
            // OK on a deck episode opens the show, so the show row would only repeat the press
            // and the episode's own page is what the menu adds
            let ok_opens_show = opens_details && deck_episode_opens_show(m);
            if !opens_details || ok_opens_show {
                nav.push(go_item(plx_os::i18n::msg::browse_menu_go_episode()));
            }
            if has_show && !ok_opens_show {
                nav.push(go_show(plx_os::i18n::msg::browse_menu_go_show(), &m.show_rk, m.season_index.max(0)));
            }
        }
        // a show or season deck card whose OK opens the page: that page IS the "go to" row's
        // target, so the row would only repeat the press (Play leads instead)
        1 | 2 if opens_details => {}
        // a season has no page of its own — it IS the show page with that season selected, so one
        // row covers it; a show's own page is likewise the only navigation it has
        2 if has_show => nav.push(go_show(plx_os::i18n::msg::browse_menu_go_season(), &m.show_rk, m.season_index)),
        2 => {}
        1 => nav.push(go_show(plx_os::i18n::msg::browse_menu_go_show(), &m.rk, 0)),
        _ if opens_details => {}
        _ => nav.push(go_item(plx_os::i18n::msg::browse_menu_go_movie())),
    }
    // the divider the design groups on — only when there IS a group above it
    let had_nav = !nav.is_empty();
    for (id, action, row) in nav {
        sec = sec.item(id, RowKind::Button, action, row);
    }
    if had_nav {
        sec = sec.separator();
    }

    // ---- state ----
    // The row set comes from the shared `widgets::row_watch_state`, over the catalog row this menu
    // captured when it was presented — so it is exact for a leaf (`viewCount`/`viewOffset` off the
    // same container the shelf was built from) AND for a container, whose three states are already
    // on the row as the `unwatched`/`watched` PAIR (`pms::parse_item`: a show is "neither" exactly
    // while some of its leaves are viewed and some are not).
    let mut sec = state_rows(
        sec,
        &m.rk,
        plx_ui::widgets::row_watch_state(&tile_facts::of(m)),
        leaf,
        leaf && m.versions.len() >= 2,
        trailer.filter(|_| m.kind == 0 || m.kind == 1),
        &m.title,
    );

    // ---- and, only on a Continue Watching card, the row that takes it off the deck ----
    // Gated on the SHELF, not on the item: the action is meaningless anywhere else (nothing to
    // remove it from), and offering it on a Recently Added card would be a row that silently did
    // nothing. `pms::hub_is_continue` is what the caller passes in.
    //
    // Last, after the watched toggle, because it is the only row here that removes something from
    // view — the destructive-ish end of the group, where a mis-hit is least likely.
    //
    // This hides the item from the deck and leaves its resume point intact. Panel sizing follows
    // the translated action, so the destination remains readable before confirming.
    sec = sec.item_if(
        from_deck,
        ItemRow::RemoveFromDeck,
        RowKind::Button,
        Action::RemoveFromDeck(m.rk.clone()),
        Row::new(plx_os::i18n::msg::browse_menu_remove_deck()).licon(Icon::Close).destructive(true),
    );
    if let Some(guid) = watchlist_guid {
        sec = sec.item(
            ItemRow::WatchlistRemove,
            RowKind::Button,
            Action::WatchlistRemove(guid.to_string()),
            Row::new(plx_os::i18n::msg::watchlist_action_remove()).licon(Icon::Close).destructive(true),
        );
    } else {
        // A Watchlist tile's own Remove (above) is the membership row there; everywhere else the
        // face adds Add / Remove, last, with the one other destructive row.
        sec = watchlist::push_face(sec, face);
    }
    Form::new().section(sec)
}

/// The state group every menu ends with: the watch-state row (or ROWS), then (for a leaf) Play from
/// Start. ONE builder, because these are the rows the menu exists for and they must read identically
/// wherever it is opened.
///
/// **The tail is one row or TWO**, off the same three-state vocabulary ([`PosterMark`]) the detail
/// hero resolves (`detail::hero_watch_state`) so the two surfaces cannot describe one item
/// differently: each row states the OUTCOME its press produces, so a finished item offers only *Mark
/// as Unwatched* and one never started offers only *Mark as Watched*. A PART-WATCHED item is in
/// neither state, and a LIST can name both destinations at once, so it gets both rows — the ✓ then
/// the −, the same order the two ends of the range read in.
///
/// **The detail hero answers the same question with ONE control, and the difference is deliberate.**
/// Its watched control is a TOGGLE wearing the face of the write it would perform
/// (`detail::hero_ctls`), so a part-watched item reads ✓ there — it is not watched, and the other
/// end is one press away rather than one control away. A menu has no state of its own and is free
/// to list both; a toggle is one thing and must read as the thing it currently is. Same vocabulary,
/// two presentations of it — so do NOT "fix" this row set to match the hero's.
///
/// **Both glyphs are FILLED discs**: [`Icon::CheckCircleFill`] for the row that marks watched,
/// [`Icon::MinusCircleFill`] for the one that takes it away. A ticked circle beside "Mark as
/// Unwatched" states the outcome backwards, which is the one thing a destructive-ish row must not do
/// — and filled is what separates an ACTION from a STATE.
fn state_rows(
    sec: FormSection<ItemRow, Action, MenuPage>,
    rk: &str,
    mark: PosterMark,
    leaf: bool,
    versions: bool,
    trailer: Option<&plx_data::metadata::Extra>,
    parent_title: &str,
) -> FormSection<ItemRow, Action, MenuPage> {
    let playable = trailer.filter(|e| e.playable());
    sec.item_if(
        mark != PosterMark::Watched,
        ItemRow::MarkWatched,
        RowKind::Button,
        Action::MarkWatched(rk.to_string()),
        Row::new(plx_ui::widgets::mark_watched_verb()).licon(Icon::CheckCircleFill),
    )
    // Destructive: it throws the watch record away, so a menu never OPENS on it
    // (`TableView::opening_row`) — a watched episode opens on Play from Start instead.
    .item_if(
        mark != PosterMark::None,
        ItemRow::MarkUnwatched,
        RowKind::Button,
        Action::MarkUnwatched(rk.to_string()),
        Row::new(plx_ui::widgets::mark_unwatched_verb())
            .licon(Icon::MinusCircleFill)
            .destructive(true),
    )
    .item_if(
        leaf,
        ItemRow::PlayFromStart,
        RowKind::Button,
        Action::PlayFromStart(rk.to_string()),
        Row::new(plx_ui::widgets::play_from_start_verb()).licon(Icon::Restart),
    )
    // Present only when there is a choice to make: a single-version item plays the one it has.
    .item_if(
        versions,
        ItemRow::PlayVersion,
        RowKind::Nav(MenuPage::Versions),
        Action::ChooseVersion(rk.to_string()),
        Row::new(plx_os::i18n::msg::browse_menu_select_version()).licon(Icon::Collection),
    )
    .item_if(
        playable.is_some(),
        ItemRow::PlayTrailer,
        RowKind::Button,
        playable.map_or_else(
            || Action::PlayTrailer {
                rk: String::new(), part: String::new(), vcodec: String::new(), acodec: String::new(), title: String::new(),
            },
            |extra| Action::PlayTrailer {
                rk: extra.rk.clone(),
                part: extra.part.clone(),
                vcodec: extra.vcodec.clone(),
                acodec: extra.acodec.clone(),
                title: extra.hud_title(parent_title).to_string(),
            },
        ),
        Row::new(plx_ui::widgets::play_trailer_verb()).licon(Icon::Trailer),
    )
}

/// The DETAIL page's episode filmstrip: the same panel and the same state rows, with **no
/// navigation group**.
///
/// Both navigation rows the shelf menu offers an episode are dead ends from here. "Go to Show" is
/// the page you are standing on. "Go to Episode" is the judgement call, and it goes the same way:
/// the episode's own page carries nothing the tile the popover is anchored to is not already
/// showing — its still, title, full summary and air date are all right there. A row that navigates
/// away from a page to show less of what that page already shows is not an action.
///
/// `mark` is exact here — `detail::focused_episode` resolves it through the same `ep_state` that
/// draws the still's own state line, so the tile and the menu opened on it cannot describe one
/// episode two ways — and with no nav group there is no separator either ([`build`]'s rule: the
/// divider only exists when there is a group above it). An episode is a LEAF, so all three states are
/// reachable and a part-watched one gets the pair, exactly as a shelf card does.
fn build_episode(rk: &str, mark: PosterMark, versions: &[MediaVersion], face: Option<&watchlist::Face>) -> ItemForm {
    let sec = state_rows(FormSection::new(""), rk, mark, true, versions.len() >= 2, None, "");
    Form::new().section(watchlist::push_face(sec, face))
}

/// **The Detail hero's … menu**: one row per action the round buttons had no room for, in the row's
/// order. A row wears the SAME icon and verb as its round button (`registry::hero_face`) and a press
/// hands its [`HeroCtl`] back to the page that opened the menu, which performs it through the one path
/// its own button takes (`AppMsg::HeroCtl`). The two that have a LIST (Select Version, Audio &
/// Subtitles) are pushed as pages of this menu instead (`ItemMenuScreen::page_for`); a Select
/// Version row with no versions to list falls back to the page's own panel.
fn build_hero_overflow(rows: &[HeroCtl], page_for: &dyn Fn(HeroCtl) -> Option<MenuPage>) -> ItemForm {
    let mut sec = FormSection::new("");
    for &ctl in rows {
        // A control moves under … only if it has a round-button face (`hero::actions` puts Watchlist and
        // the watch toggle first, and they never move); `overflow_faces_are_total` pins it.
        let Some((icon, label)) = hero_face(ctl) else {
            debug_assert!(false, "{ctl:?} has no face, so it cannot be an overflow row");
            continue;
        };
        let label = label.to_str().unwrap_or_default();
        // a control with a page here (Select Version, Audio & Subtitles) is a drill-in; every other is a command
        let kind = page_for(ctl).map_or(RowKind::Button, RowKind::Nav);
        sec = sec.item(ItemRow::Hero(ctl), kind, Action::Hero(ctl), Row::new(label).licon(icon));
    }
    Form::new().section(sec)
}

/// The Watchlist grid's hold menu (plan §1.9): [`self::hold::rows_for`] bound to
/// actions. The "go" row is Detail (`rk`, the matched copy on [`ItemMenuArg::sid`]) or the catalog
/// page (`guid`); Remove is destructive, so the menu opens on the "go" row, never on it. W6's
/// catalog-only rows (Add/Remove are the whole Catalog menu today) would join here.
fn build_catalog(
    state: plx_data::watchlist::model::TileState,
    kind: plx_data::watchlist::model::Kind,
    rk: &str,
    guid: &str,
    member: plx_data::catalog::Membership,
) -> ItemForm {
    use self::hold::HoldRowId;
    let mut sec = FormSection::new("");
    for r in self::hold::rows_for(state, kind) {
        let (id, action) = match r.id {
            HoldRowId::GoToDetail => (ItemRow::GoToItem, Action::GoToItem(rk.to_string())),
            HoldRowId::GoToCatalog => (ItemRow::GoToCatalog, Action::GoToCatalog(guid.to_string())),
            // The membership row: Remove for a title on the list (every Watchlist tile), Add for
            // one that is not, both captured now.
            HoldRowId::Remove => {
                if let Some((id, action, row)) = watchlist::catalog_row(member, guid) {
                    sec = sec.item(id, RowKind::Button, action, row);
                }
                continue;
            }
        };
        sec = sec.item_if(
            true,
            id,
            RowKind::Button,
            action,
            Row::new(r.label).licon(r.icon).destructive(r.destructive),
        );
    }
    Form::new().section(sec)
}

/// A catalog credit has no PMS actions; adding it is idempotent even when already listed.
fn build_watchlist_add(hex: [u8; 12]) -> ItemForm {
    let (id, action, row) = watchlist::add_row(hex, false);
    Form::new().section(FormSection::new("").item(id, RowKind::Button, action, row))
}

/// The DETAIL page's season strip: [`build_episode`]'s row set with the one difference a season
/// makes — **no "Play from Start"**.
///
/// That row means "play THIS item from 00:00", and a season is not a thing you play: the press
/// would have to pick a leaf, which is a second decision the row does not state. Starting a season
/// from its beginning is what the first episode's own tile does, exactly and visibly. So
/// `leaf: false` — the same flag [`build`] passes for a show, for the same reason.
fn build_season(rk: &str, mark: PosterMark, face: Option<&watchlist::Face>) -> ItemForm {
    let sec = state_rows(FormSection::new(""), rk, mark, false, false, None, "");
    Form::new().section(watchlist::push_face(sec, face))
}

/// **The Select Version page**: the shared version list (`plx_appkit::version_list`), every row a
/// choice of that version for the captured item, the one `checked` (the file Play takes now) marked.
fn build_versions(rk: &str, versions: &[MediaVersion], checked: Option<plx_plex::plex::MediaId>, can_play: bool) -> ItemForm {
    // without a playback driver every row is disabled and says why (the standalone panel's rule)
    let unavailable = (!can_play).then(plx_os::i18n::msg::browse_detail_no_playback);
    Form::new().section(plx_appkit::version_list::section("", versions, ItemRow::Version, |v| Action::SelectVersion {
        rk: rk.to_string(),
        media: v.id,
    }, checked, unavailable))
}

/// The anchor a menu with no rect to hang off falls back to — a centred card. The headless trigger
/// and a host with nothing focused both land here.
///
/// Resolved by the PRESENTER, once, rather than every frame inside [`ItemMenuScreen::frame`], so
/// the panel cannot drift if a host's rect stops resolving while the menu is up — the same
/// argument the legacy `present` made for resolving it before it stored `OPENER`.
pub fn fallback_anchor() -> Rect {
    Rect::new(
        (SCR_W - CARD_W) * 0.5 - MENU_MIN_W * 0.5,
        (SCR_H - CARD_H) * 0.5,
        CARD_W,
        CARD_H,
    )
}

/// A page the menu shows over its root rows: Select Version, or one of the Audio & Subtitles pages (its root,
/// then Audio or Subtitles pushed over it). The menu's root is the EMPTY stack. It is the menu's `Dest`: a
/// row that opens one is a `Nav` row, so OK and RIGHT push it and BACK and LEFT pop it, by `ui::menu`'s one
/// rule.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MenuPage {
    Versions,
    Tracks(PickPage),
}

/// **The state only a hero control's menu has**: the content of the pages its rows open, Select Version's
/// versions and Audio & Subtitles' tracks, of the page the menu hangs off, read when the menu was built (the
/// SAME pages the standalone panels show, built by the same code, `plx_appkit::version_list` and
/// `plx_appkit::preplay_tracks`). One value, so the menu has all of it or none: a tile's menu is none of it.
/// The glass it grows out of its button as is the menu's own (`Host::Grows`).
struct Hero {
    /// The page's versions and the one Play takes now.
    versions: Vec<MediaVersion>,
    taken: Option<plx_plex::plex::MediaId>,
    /// Whether a playback driver was composed; without one every version row is disabled.
    can_play: bool,
    tracks: PickerCore,
}

impl Hero {
    fn new(sid: plx_plex::plex::ServerId, rk: &str) -> Self {
        Self {
            versions: Vec::new(),
            taken: None,
            can_play: true,
            tracks: PickerCore::new(crate::registry::empty_tracks(sid, rk)),
        }
    }
}

/// What the menu's ROOT rows are built from besides its argument, read when it opened (so a pop can build
/// them again exactly as they were): a card's cached trailer and the deck-press preference.
struct RootInputs {
    trailer: Option<plx_data::metadata::Extra>,
    deck_press: DeckPress,
}

/// **What this menu's pages are built from**, borrowed field by field of [`ItemMenuScreen`] (the
/// `source!` macro) so a step can hold the menu mutably while the page is built. The rows of every page, the
/// root's included, come out of here and nowhere else.
struct PageSource<'a> {
    arg: &'a ItemMenuArg,
    face: Option<&'a watchlist::Face>,
    root: &'a RootInputs,
    hero: Option<&'a Hero>,
}

macro_rules! source {
    ($menu:expr) => {
        PageSource { arg: &$menu.arg, face: $menu.face.as_ref(), root: &$menu.root, hero: $menu.hero.as_ref() }
    };
}

/// One step of the menu, its pages built from the menu's own fields.
macro_rules! step {
    ($menu:expr, $ev:expr) => {{
        let src = source!($menu);
        $menu.menu.step($ev, &mut |page| src.page(page))
    }};
}

/// One screen event of the menu (`Menu::on_screen`), its pages built from the menu's own fields.
macro_rules! glue {
    ($menu:expr, $ev:expr, $cx:expr, $fx:expr) => {{
        let src = source!($menu);
        $menu.menu.on_screen($ev, $cx, $menu.entry, $fx, &mut |page| src.page(page))
    }};
}

impl PageSource<'_> {
    /// The menu's own rows, by what it is about.
    fn root_form(&self) -> ItemForm {
        let arg = self.arg;
        match &arg.kind {
            ItemMenuKind::WatchlistAdd { hex } => build_watchlist_add(*hex),
            ItemMenuKind::Card { row, from_deck } => {
                // A Watchlist tile with a reachable copy is a card of that copy, and Remove joins it.
                let remove = arg.catalog.as_ref().map(|c| c.guid.as_str());
                build_card(row, *from_deck, self.root.deck_press, self.root.trailer.as_ref(), remove, self.face)
            }
            ItemMenuKind::Episode { mark, versions } => build_episode(&arg.rk, *mark, versions, self.face),
            ItemMenuKind::Season { mark, .. } => build_season(&arg.rk, *mark, self.face),
            ItemMenuKind::HeroOverflow { rows, .. } => build_hero_overflow(rows, &|ctl| self.page_for(ctl)),
            ItemMenuKind::Catalog { state } => {
                let guid = arg.catalog.as_ref().map_or("", |c| c.guid.as_str());
                let kind = arg.catalog.as_ref().map_or(plx_data::watchlist::model::Kind::Movie, |c| c.kind);
                // A Watchlist tile is on the list by definition; a face says otherwise.
                let member = self.face.map_or(plx_data::catalog::Membership::On, |f| f.member);
                build_catalog(*state, kind, &arg.rk, guid, member)
            }
        }
    }

    /// The page a hero control's row opens inside this menu, when the menu has it: Select Version when the
    /// page has versions, Audio & Subtitles when it has tracks to pick. `None`: the control's own path, which
    /// closes the menu and hands the control back to the page.
    fn page_for(&self, ctl: HeroCtl) -> Option<MenuPage> {
        let hero = self.hero?;
        match ctl {
            HeroCtl::Version => (!hero.versions.is_empty()).then_some(MenuPage::Versions),
            HeroCtl::AudioSubtitles => (!hero.tracks.is_empty()).then_some(MenuPage::Tracks(PickPage::Root)),
            _ => None,
        }
    }

    /// What Select Version lists: the versions, the one Play takes now (the check mark) and whether a
    /// playback driver can take one. A hero menu read them off its page when it opened; a tile's menu
    /// lists the versions of its item, the profile's choice else what plays unasked.
    fn versions(&self) -> (&[MediaVersion], Option<plx_plex::plex::MediaId>, bool) {
        if let Some(hero) = self.hero {
            return (&hero.versions, hero.taken, hero.can_play);
        }
        let versions: &[MediaVersion] = match &self.arg.kind {
            ItemMenuKind::Card { row, .. } => &row.versions,
            ItemMenuKind::Episode { versions, .. } => versions,
            _ => &[],
        };
        let chosen = plx_media::route::chosen_version(plx_media::route::item_sid(self.arg.sid), &self.arg.rk);
        (versions, plx_media::route::taken_version(versions, chosen), true)
    }

    /// **Select Version's page**: the rows by the code the standalone panel uses, its header, and focus opening on
    /// the check mark's row, so the viewer sees what is chosen.
    fn versions_page(&self) -> Page<ItemRow, Action, MenuPage> {
        let (versions, taken, can_play) = self.versions();
        let initial = taken.and_then(|id| versions.iter().position(|v| v.id == id)).map(|n| ItemRow::Version(n as u16));
        Page::new(build_versions(&self.arg.rk, versions, taken, can_play), plx_appkit::version_list::title()).initial(initial)
    }

    /// An Audio & Subtitles page of the picker's core ([`PickerCore::page`]) as this menu's: its rows
    /// re-identified as [`ItemRow::Pick`] and committing [`Action::Pick`], a drill-in pushing the page it names by the
    /// menu's one rule. `None` off a hero menu, and for a language the item no longer offers.
    fn pick_page(&self, page: PickPage) -> Option<Page<ItemRow, Action, MenuPage>> {
        let built = self.hero?.tracks.page(page)?;
        let form = built.form.map(
            |id| Some(ItemRow::Pick(*id)),
            |pick, id, kind, ()| Rebind {
                key: id.key(),
                kind: match kind {
                    RowKind::Nav(page) => RowKind::Nav(MenuPage::Tracks(page)),
                    RowKind::Toggle => RowKind::Toggle,
                    RowKind::Choice => RowKind::Choice,
                    RowKind::Button => RowKind::Button,
                },
                action: Action::Pick(*pick),
            },
        );
        Some(Page::new(form, built.title).initial(built.initial.map(ItemRow::Pick)))
    }

    /// `page` ready to show (`None`: the root): its rows, its header and where focus opens. The size class is
    /// the menu's, compact, whatever page shows (`ui::menu::Class::Action`: one per glass).
    fn page(&self, page: Option<MenuPage>) -> Option<Page<ItemRow, Action, MenuPage>> {
        Some(match page {
            None => Page::root(self.root_form()),
            Some(MenuPage::Versions) => self.versions_page(),
            // the Audio & Subtitles root is named as the row that opened it is; the pages under it name themselves
            Some(MenuPage::Tracks(PickPage::Root)) => {
                let mut page = self.pick_page(PickPage::Root)?;
                page.title = tracks::title().to_string();
                page
            }
            Some(MenuPage::Tracks(page)) => self.pick_page(page)?,
        })
    }
}

pub struct ItemMenuScreen {
    entry: EntryId,
    arg: ItemMenuArg,
    /// **The panel**: the rows and the action each one commits, declared together ([`ItemForm`], the row SET
    /// varies by item kind, so a row is found by its identity, [`ItemRow`], never by a position), the pages
    /// pushed over them (Select Version, Audio & Subtitles), the card's resize and the pages' slide, and
    /// where it stands: a glass that grows out of its control on a hero menu, glass beside the card on a
    /// tile's.
    menu: Menu<MenuPage, ItemRow, Action>,
    /// The cadence a HELD direction walks this panel's list at. `app/run.rs`'s client-side repeat
    /// timer (`App::held_key`) did this at 110 ms for exactly this menu and, before phase 9, for
    /// the player's four panels; the dispatcher delivers the hardware's ~50 ms `Edge::Repeat`
    /// instead, so the surface applies the cadence itself — the same [`PANEL_REPEAT_MS`] those
    /// four take, reused rather than copied.
    repeat: RepeatGate,
    built: bool,
    /// The Watchlist subject captured when the menu opened; `None` omits the membership row
    /// (plex.tv unreachable, or the item has no catalog guid). See [`watchlist::Face`].
    face: Option<watchlist::Face>,
    root: RootInputs,
    /// A hero control's menu: its page content. `None` for a tile's menu.
    hero: Option<Hero>,
}

/// Play Trailer only when the already-loaded Detail is this movie/show and already has a trailer.
/// The menu never talks to PMS.
fn cached_trailer(
    sid: plx_plex::plex::ServerId,
    m: &PmsMovie,
    meta: plx_data::metadata::MetadataView<'_>,
) -> Option<plx_data::metadata::Extra> {
    if m.kind != 0 && m.kind != 1 {
        return None;
    }
    let d = meta.current()?;
    if !plx_plex::plex::same_item((d.sid, d.rk.as_str()), (sid, m.rk.as_str())) {
        return None;
    }
    d.trailer().cloned().filter(|e| e.playable())
}

impl ItemMenuScreen {
    pub fn new(entry: EntryId, arg: ItemMenuArg) -> Self {
        let hero = matches!(arg.kind, ItemMenuKind::HeroOverflow { .. }).then(|| Hero::new(arg.sid, &arg.rk));
        // a hero control's menu grows out of it; a tile's stands beside the card, which stays whole above the dim.
        // A short list of one-line actions: BODY labels, not menu-size HEADLINE.
        let host = if hero.is_some() {
            plx_ui::detail_layout::hero_host(arg.anchor)
        } else {
            Host::BesideGlass { anchor: arg.anchor }
        };
        Self {
            entry,
            arg,
            menu: Menu::new(host, Class::Action, crate::registry::BAND),
            repeat: RepeatGate::IDLE,
            built: false,
            face: None,
            root: RootInputs { trailer: None, deck_press: DeckPress::Play },
            hero,
        }
    }

    /// The Watchlist subject the app captured at open (identity and membership as read THEN).
    pub fn with_watchlist(mut self, face: Option<watchlist::Face>) -> Self {
        self.face = face;
        self
    }

    /// The panel's table and cursor.
    pub fn form(&self) -> &FormTable<ItemRow, Action, MenuPage> {
        self.menu.form()
    }

    /// The rows, built ONCE at `Mount`. A hub refetch can re-order the catalog underneath an open
    /// panel, so nothing here is rebuilt while the menu is up — which is also why every [`Action`]
    /// carries the identity it needs rather than an index.
    fn build_rows(&mut self, meta: plx_data::metadata::MetadataView<'_>, deck_press: DeckPress) {
        if self.built {
            return;
        }
        self.built = true;
        self.root = RootInputs {
            trailer: match &self.arg.kind {
                ItemMenuKind::Card { row, .. } => cached_trailer(self.arg.sid, row, meta),
                _ => None,
            },
            deck_press,
        };
        // the pages a hero control's rows open, read once off the page the menu hangs off (as the standalone
        // panels read theirs at mount): by the PAGE's key, not the item Play acts on, which on a show page
        // is its on-deck episode
        if let (Some(hero), ItemMenuKind::HeroOverflow { page, .. }) = (&mut self.hero, &self.arg.kind) {
            (hero.versions, hero.taken) = crate::registry::hero_page_versions(meta, self.arg.sid, page);
            hero.tracks = PickerCore::new(crate::registry::page_preplay_tracks(meta, self.arg.sid, page));
        }
        step!(self, Event::ReplaceRoot { slide: None });
    }

    /// Where focus starts, and where it falls back to when the key it had is gone — the table's
    /// opening row, which skips the separator and never lands on a destructive row while any other
    /// is on offer.
    #[cfg(test)]
    fn opening(&self) -> u32 {
        self.menu.form().opening_key().map_or(0, |k| k.0)
    }

    /// Install `form` as the root page, as the mount does.
    #[cfg(test)]
    fn set_root(&mut self, form: ItemForm) {
        let mut form = Some(form);
        self.menu.step(Event::ReplaceRoot { slide: None }, &mut |page| if page.is_none() { form.take().map(Page::root) } else { None });
    }

    /// The page showing, laid out by the menu's one placement rule for its host.
    #[cfg(test)]
    fn frame(&self, measure: &dyn plx_machine::machine::Measure) -> Rect {
        self.menu.panel_rect(measure)
    }

    fn dismiss<H: AppLike>(&self, fx: &mut Effects<'_, H>) {
        fx.push(Fx::Nav(NavOp::Dismiss(self.entry)));
    }

    /// What the menu's answer to a step asks of the surface: a dismissal closes it (a hero menu's glass drains
    /// into its button), a pick acts.
    fn run<H: AppLike>(&mut self, out: Out<Action>, elem: Option<u32>, fx: &mut Effects<'_, H>) {
        match out {
            Out::Stay | Out::Moved | Out::Edge => {}
            Out::Dismiss => self.dismiss(fx),
            Out::Picked(act) if self.hero.is_some() => self.activate_hero_menu(act, elem, fx),
            Out::Picked(act) => self.commit(act, fx),
        }
    }

    /// A row of a hero control's menu that is no page: any control without one closes the menu and is
    /// handed back to the page that owns the hero (its `activate_hero` is the one path: Go to Show, Play from
    /// Start, Trailer, ...); a pick on a page acts.
    fn activate_hero_menu<H: AppLike>(&mut self, act: Action, elem: Option<u32>, fx: &mut Effects<'_, H>) {
        let ItemMenuKind::HeroOverflow { opener, .. } = &self.arg.kind else { return };
        let opener = *opener;
        match act {
            Action::Hero(ctl) => {
                self.dismiss(fx);
                fx.push(Fx::Deliver(
                    MachineId::Instance(opener),
                    Delivery::Screen(ScreenEvent::App(AppMsg::HeroCtl { ctl, anchor: self.arg.anchor })),
                ));
            }
            // A pick in Select Version closes the whole menu and reports the choice to the page, which
            // selects it (the standalone panel's own commit); focus returns to the `…` with the glass.
            Action::SelectVersion { media, .. } => {
                self.dismiss(fx);
                fx.push(Fx::Deliver(
                    MachineId::Instance(opener),
                    Delivery::Screen(ScreenEvent::App(AppMsg::SelectVersion(media))),
                ));
            }
            Action::Pick(pick) => {
                if let Some(row) = elem.and_then(|e| self.menu.id_of(e)).copied() {
                    self.pick(pick, row, fx);
                }
            }
            _ => self.dismiss(fx),
        }
    }

    /// A track row of an Audio & Subtitles page (a drill-in is a page the menu pushed): picked exactly as
    /// the standalone panel picks it (the page rebuilds with the check moved, the menu stays up, the
    /// store keeps the pick for the next start and the loop commits it to the server).
    fn pick<H: AppLike>(&mut self, pick: PickRow, row: ItemRow, fx: &mut Effects<'_, H>) {
        let what = tracks::pick_of(pick);
        if matches!(what, Pick::Open(_)) {
            return;
        }
        let Some(hero) = &mut self.hero else { return };
        let Some(sel) = hero.tracks.select(what) else { return };
        let picked = hero.tracks.tracks().clone();
        step!(self, Event::Refresh { keep: Some(row) });
        commit_pick(fx, picked, sel, PreplaySource::Page);
    }

    /// A tile's pick: emit dismissal before the captured action. Navigation is parked until commit, while the
    /// action yields to the app runner; neither needs to read the surface after activation.
    fn commit<H: AppLike>(&mut self, act: Action, fx: &mut Effects<'_, H>) {
        // Every arm of the dispatch turns an rk into a blocking fetch, a scrobble or a play; an
        // empty one would fetch nothing and land on a blank page. `build` already refuses to offer
        // such a row — this is the belt to that braces, since the rows are data-driven off hub rows.
        self.dismiss(fx);
        if act.has_subject() {
            fx.push(Fx::App(AppFx::ItemMenu(ItemMenuReq {
                act,
                sid: self.arg.sid,
                item: match &self.arg.kind {
                    ItemMenuKind::Card { row, .. } => Some((**row).clone()),
                    _ => None,
                },
                season: match &self.arg.kind {
                    ItemMenuKind::Season { show_rk, index, .. } => Some((show_rk.clone(), *index)),
                    _ => None,
                },
                loaded_episode: self.arg.loaded_episode,
                from_home: self.arg.from_home,
            })));
        }
    }

    /// The highlighted row, for the focus probe — a READ of the cursor the engine moves.
    pub fn sel(&self) -> i32 {
        self.menu.form().table.sel
    }

    /// The server every action from this menu names. Captured rather than looked up: on a Continue
    /// Watching shelf merged across servers, resolving a bare rk against the CURRENT server is the
    /// reported bug itself (hold a friend's episode → Play from Start → our film with the same key
    /// plays, under the friend's title).
    pub fn sid(&self) -> plx_plex::plex::ServerId {
        self.arg.sid
    }

    /// The page this menu hangs off, and the element on it the panel is anchored beside and lifted
    /// back out of the dim. ONE owner: the entry's own argument, captured when the menu was
    /// presented. It was `Bridge::menu_opener`, a second copy of the same pair on the side.
    pub fn opener(&self) -> (EntryId, Option<FocusKey<u32>>) {
        (self.arg.host, self.arg.focus)
    }
}

impl<H: AppLike + crate::registry::MetadataLike + crate::registry::ScreenInputsLike> Machine<H> for ItemMenuScreen {
    type Ev = ScreenEvent<H>;
    fn step(&mut self, ev: &Self::Ev, cx: &Cx<'_, H>, fx: &mut Effects<'_, H>) -> Handled {
        if let ScreenEvent::Mount = ev {
            if let Some(hero) = &mut self.hero {
                hero.can_play = H::screen_inputs(cx).can_play();
            }
            self.build_rows(H::metadata(cx), H::screen_inputs(cx).preference::<plx_plex::plex::session::DeckPressPref>());
            // The menu is the thing the Home hold hint teaches; once ANY card screen has
            // opened one the lesson retires for this input owner, through its typed effect.
            fx.push(Fx::HoldHint(plx_machine::machine::HoldHintEvent::Learned));
        }
        // Everything else the menu's own (`Menu::on_screen`): the open and the close (a hero menu's glass runs
        // the morph back into its control however the surface was dismissed), the tick, the focus, OK and a
        // click, BACK and LEFT (pop a page) and RIGHT (enter a Nav row). At the rows BACK closes and LEFT is inert.
        if let Some(glued) = glue!(self, ev, cx, fx) {
            self.run(glued.out, glued.elem, fx);
            return glued.handled;
        }
        // **The hold-to-move cadence, applied HERE and nowhere else.** It was `app/run.rs`'s per-frame
        // `App::held_key` timer, whose LAST consumer this menu was; the dispatcher hands the surface the
        // hardware's own `Edge::Repeat` at ~50 ms, which walks a five-row list faster than anybody can read
        // it. A FRESH press is never swallowed by the cadence of the press before it: it is acted on
        // unconditionally by falling through to `Handled::No`, and `rearm` records it as the step it is.
        if let ScreenEvent::Input(input) = ev {
            if let InputKind::Key { key: Key::Up | Key::Down, edge, .. } = input.kind {
                match edge {
                    Edge::Down => self.repeat.rearm(input.at.ms),
                    Edge::Repeat if !self.repeat.ready_every(input.at.ms, PANEL_REPEAT_MS) => return Handled::Yes,
                    _ => {}
                }
            }
        }
        Handled::No
    }
}

impl plx_ui::menu::HasMenu for ItemMenuScreen {
    type Page = MenuPage;
    type Row = ItemRow;
    type Act = Action;
    fn menu(&self) -> &Menu<MenuPage, ItemRow, Action> {
        &self.menu
    }
    fn menu_entry(&self) -> EntryId {
        self.entry
    }
}

impl<H: AppLike> Focusable<H> for ItemMenuScreen {
    fn groups(&self, cx: &Cx<'_, H>, out: &mut Vec<GroupSpec>) {
        self.menu.groups(GroupId(0), cx.measure, out);
    }
    fn group_of(&self, elem: &u32, _: &Cx<'_, H>) -> Option<GroupId> {
        self.menu.group_of(elem, GroupId(0))
    }
    fn neighbour(&self, key: FocusKey<u32>, dir: Dir, _: &Cx<'_, H>) -> Step<u32> {
        self.menu.neighbour(self.entry, key, dir)
    }
    fn place(&self, elem: &u32, cx: &Cx<'_, H>, _: At) -> Option<Placed> {
        self.menu.place(elem, cx.measure)
    }
    fn reconcile(&self, want: FocusKey<u32>, _: &Cx<'_, H>) -> FocusKey<u32> {
        // a page that was just left for another (a pop): the ring goes to the row the form restored, the one
        // that opened the page, as a seat does
        self.menu.reconcile(self.entry, want, GroupId(0))
    }
    fn seat(&self, _: GroupId, _: Placed, _: &Cx<'_, H>) -> FocusKey<u32> {
        self.menu.seat(self.entry)
    }
    /// `itemmenu.<row>`: the menu's [`ItemRow`] names its rows (`ItemRow::a11y_name`; a plain word), the Play
    /// Version page's one row per version `itemmenu.version.<n>`. The label is the row's drawn
    /// words (read side only). Every row's `subject` is the ratingKey of the item the menu was opened on
    /// ([`ItemMenuArg::rk`], captured at open): a row (Play, Mark watched, Remove from Deck, a Watchlist
    /// write) acts on THAT title, so a press names it. A Catalog menu (a Watchlist tile no server holds)
    /// has no ratingKey, so its rows carry no subject.
    fn a11y(&self, key: &u32, _: &Cx<'_, H>) -> Option<A11y<'_>> {
        let form = self.menu.form();
        let index = form.index_of_key(RowKey(*key))?;
        let path = match *form.id_at(index)? {
            ItemRow::Version(n) => A11yPath::new().name("version").index(usize::from(n)),
            ItemRow::Hero(ctl) => A11yPath::new().name("hero").name(ctl.a11y_name()),
            ItemRow::Pick(pick) => match pick {
                PickRow::OpenAudio => A11yPath::new().name("tracks").name("audio"),
                PickRow::OpenSubtitles => A11yPath::new().name("tracks").name("subtitles"),
                PickRow::SubOff => A11yPath::new().name("tracks").name("subtitle").name("off"),
                PickRow::OpenOther => A11yPath::new().name("tracks").name("subtitle").name("other"),
                PickRow::OpenLang(lang) => A11yPath::new().name("tracks").name("subtitle").name("lang").index(lang.slot),
                PickRow::Audio(i) => A11yPath::new().name("tracks").name("audio").index(i),
                PickRow::Sub(i) => A11yPath::new().name("tracks").name("subtitle").index(i),
            },
            row => A11yPath::new().name(row.a11y_name()),
        };
        let a = A11y::new(path, Role::Row);
        let a = match plx_ui::geom::table_row(&form.table, index) {
            Some(row) => a.with_label(row.label.as_str()),
            None => a,
        };
        Some(if self.arg.rk.is_empty() { a } else { a.with_subject(self.arg.rk.as_str()) })
    }
    fn a11y_elems(&self, g: GroupId, _: &Cx<'_, H>, out: &mut Vec<u32>) {
        if g == GroupId(0) {
            out.extend(self.menu.focusable());
        }
    }
}

impl<H: AppLike + crate::registry::MetadataLike + crate::registry::ScreenInputsLike> Screen<H> for ItemMenuScreen {
    fn opener(&self) -> Option<(EntryId, Option<FocusKey<u32>>)> {
        Some(ItemMenuScreen::opener(self))
    }
    // The menu's own (`plx_ui::menu_screen_methods!`): a hero control's menu CONSUMES its opener (gone while the
    // glass stands, back as it drains), the droplet's hand-back (never for a tile's menu, whose close is the
    // container's), the modal dim at the class's weight (the TILE is lifted back out of it, and the lift is the
    // HOST PAGE's to draw: the dispatcher's lift pass repaints the focused element above the dim and the panel,
    // reading this surface's own `opener`), the prewarm, the pointer hold while the card or the pages move, and
    // the draw with its focus stops.
    plx_ui::menu_screen_methods!(H, "dt.item_menu");
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
    fn name(&self) -> &'static str {
        crate::registry::word::ITEM_MENU
    }
    fn state(&self) -> &dyn LogicalState {
        self
    }
    fn crumb(&self, _: &Cx<'_, H>) -> Option<Cow<'_, str>> {
        None
    }
    fn render(&self) -> RenderStrategy {
        RenderStrategy::Page
    }
}

impl LogicalState for ItemMenuScreen {
    fn write(&self, c: &mut Canon) {
        self.arg.write(c);
        let form = self.menu.form();
        let n = form.table.n_rows() as usize;
        c.seq(n);
        for i in 0..n {
            c.option(form.binding_at(i).map(|b| &b.action), |c, a| a.write(c));
        }
        c.u32(form.table.sel as u32);
        self.menu.depth_canon(c);
        form.table.write_motion(c);
    }
    fn probe(&self, out: &mut String) {
        out.push_str("item_menu");
    }
}

// ---------------------------------------------------------------------------------------
/// The row sets, the focus walk, the placement and the capture — all fourteen moved by NAME from
/// `ui/item_menu.rs` (restructure phase 10), plus the one this phase adds
/// (`the_item_menu_hold_repeats_through_the_surface_not_the_loop`).
///
/// **Two of them changed SUBJECT without changing name, and both are the phase working.** The
/// focus walk and the separator settle were stated on a live `TableView` (`move_sel`, and
/// `set_sections`' settle) because that is where the cursor lived; the walk is the focus ENGINE's
/// now, so they are stated on the screen's own `Focusable` answers — which is where the
/// separator's unfocusability actually lives (`acts[i].is_none()`) and what the engine asks. They
/// no longer take `testlock::serial()` either: nothing here drives a live `Spring`, so there is no
/// process-global dirty flag to contend for.
#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use plx_ui::consts::MARGIN_X;
    use plx_ui::screen::DrawFrame;
    use plx_ui::theme;
    use plx_ui::menu::beside as panel_at;
    use plx_ui::table::{MENU_MAX_W, MENU_MIN_W};

    const EDGE: f32 = theme::space::XL;
    use crate::registry::ContentReq;
    use plx_data::stores::StoreId;

    /// A catalog row of `kind` in a given watch state, with the flags a real `pms::parse_item`
    /// would set — which is the part that matters here, since the menu's row set is derived from
    /// `unwatched`/`watched`/`resume_ms` together rather than from one of them.
    ///
    /// A LEAF in the middle is `viewCount == 0` with a live `viewOffset`; a CONTAINER in the middle
    /// is "some leaves viewed, not all", i.e. NEITHER flag — the state a bool could not hold and the
    /// reason this fixture takes a [`PosterMark`].
    fn item(kind: c_int, mark: PosterMark) -> PmsMovie {
        let leaf = kind == 0 || kind == 3;
        let mut m = PmsMovie::default();
        m.rk = "42".to_string();
        m.kind = kind;
        m.show_rk = "7".to_string();
        m.season_index = 3;
        m.dur_ns = 100 * 60 * 1000 * 1_000_000;
        match mark {
            PosterMark::None => m.unwatched = true,
            PosterMark::Watched => m.watched = true,
            PosterMark::InProgress if leaf => {
                m.unwatched = true;
                m.resume_ms = 30 * 60 * 1000;
            }
            PosterMark::InProgress => {} // a container: neither end
        }
        assert_eq!(
            plx_ui::widgets::row_watch_state(&tile_facts::of(&m)),
            mark,
            "the fixture must build the state it names"
        );
        m
    }
    /// A built menu: the form run through a [`FormTable`], so a test reads rows back the way the
    /// screen does — by identity — instead of through a parallel vector.
    struct Built(FormTable<ItemRow, Action, MenuPage>);
    fn built(form: ItemForm) -> Built {
        // the item menus are action menus (the class `ItemMenu` builds its `Menu` with)
        let mut table = FormTable::with_class(crate::registry::BAND, Class::Action);
        table.set(form, None);
        Built(table)
    }
    fn build(m: &PmsMovie, from_deck: bool) -> Built {
        built(super::build(m, from_deck))
    }
    fn build_with(m: &PmsMovie, from_deck: bool, trailer: Option<&plx_data::metadata::Extra>) -> Built {
        built(super::build_with(m, from_deck, DeckPress::Play, trailer))
    }
    /// A menu built under the Continue Watching setting `mode`.
    fn build_deck(m: &PmsMovie, from_deck: bool, mode: DeckPress) -> Built {
        built(super::build_with(m, from_deck, mode, None))
    }
    fn build_episode(rk: &str, mark: PosterMark) -> Built {
        built(super::build_episode(rk, mark, &[], None))
    }
    fn build_season(rk: &str, mark: PosterMark) -> Built {
        built(super::build_season(rk, mark, None))
    }
    impl Built {
        /// The row ids in drawn order (the separator is not a row).
        fn ids(&self) -> Vec<ItemRow> {
            (0..self.0.table.n_rows() as usize).filter_map(|i| self.0.id_at(i).copied()).collect()
        }
        /// Every action the menu can commit, in drawn order.
        fn actions(&self) -> Vec<Action> {
            (0..self.0.table.n_rows() as usize)
                .filter_map(|i| self.0.binding_at(i).map(|b| b.action.clone()))
                .collect()
        }
        /// The action the row `id` commits.
        fn action(&self, id: ItemRow) -> Action {
            let i = self.0.index_of(&id).unwrap_or_else(|| panic!("{id:?} is not on offer"));
            self.0.binding_at(i).expect("bound").action.clone()
        }
        fn offers(&self, id: ItemRow) -> bool {
            self.0.index_of(&id).is_some()
        }
        fn is_destructive(&self, id: ItemRow) -> bool {
            self.0.index_of(&id).is_some_and(|i| self.0.table.sections[0].rows[i].destructive)
        }
    }
    fn labels(menu: &Built) -> Vec<String> {
        menu.0.table.sections[0]
            .rows
            .iter()
            .map(|r| if r.sep { "—".to_string() } else { r.label.clone() })
            .collect()
    }
    /// The write each row commits, paired with its label — the projection every row-set assertion
    /// below is really about, since a label and its verb living in two places is the bug this
    /// module's `Action` split closed.
    fn verbs(menu: &Built) -> Vec<(String, Option<plx_data::viewstate::Write>)> {
        let table = &menu.0.table;
        (0..table.n_rows() as usize)
            .map(|i| {
                let label = table.sections[0].rows[i].label.clone();
                (label, menu.0.binding_at(i).and_then(|b| b.action.watch_write()))
            })
            .collect()
    }

    /// **Under "Open details" a deck card's menu leads with Play and opens on it**, drops the Go
    /// to Movie / Episode row OK now does, and keeps every other row — and in Play mode and off
    /// the deck the menu is exactly today's.
    #[test]
    fn a_deck_card_leads_with_play_when_ok_opens_the_page() {
        // an episode: OK opens the SHOW (on the card's season), so the menu offers the episode's
        // own page instead: Play, Go to Episode, separator, state, Remove last
        let on = build_deck(&item(3, PosterMark::None), true, DeckPress::Details);
        assert_eq!(
            labels(&on),
            ["Play", "Go to Episode", "—", "Mark as Watched", "Play from Start", "Remove from Deck"]
        );
        assert_eq!(
            on.ids(),
            [ItemRow::Play, ItemRow::GoToItem, ItemRow::MarkWatched, ItemRow::PlayFromStart, ItemRow::RemoveFromDeck]
        );
        assert!(!on.offers(ItemRow::GoToShow), "OK on the card already opens that page");
        assert_eq!(on.action(ItemRow::GoToItem), Action::GoToItem("42".into()));
        assert_eq!(on.action(ItemRow::Play), Action::Play("42".into()));
        assert_eq!(
            on.0.opening_key(),
            Some(ItemRow::Play.key()),
            "the menu opens on Play"
        );

        // a movie: no navigation group at all, so no separator — not at the top, not under Play
        let movie = build_deck(&item(0, PosterMark::None), true, DeckPress::Details);
        assert_eq!(labels(&movie), ["Play", "Mark as Watched", "Play from Start", "Remove from Deck"]);
        assert!(!movie.offers(ItemRow::GoToItem) && !movie.offers(ItemRow::GoToShow));
        assert!(!labels(&movie).iter().any(|l| l == "—"), "{:?}", labels(&movie));

        // an episode with no show row keeps today's rows (OK opens ITS page, so the group is
        // gone): still no stray separator
        let mut orphan = item(3, PosterMark::None);
        orphan.show_rk.clear();
        let orphan = build_deck(&orphan, true, DeckPress::Details);
        assert_eq!(labels(&orphan), ["Play", "Mark as Watched", "Play from Start", "Remove from Deck"]);

        // an item with a resume point says so, in the hero pill's word
        let resuming = build_deck(&item(0, PosterMark::InProgress), true, DeckPress::Details);
        assert_eq!(labels(&resuming)[0], plx_appkit::hero_layout::play_label(true).to_str().unwrap());
        assert_eq!(labels(&resuming)[0], "Resume");
    }

    /// A show or season card on the deck (the Library's `*.inprogress.*` shelf can hold one) in
    /// Open-details mode: Play leads and opens the menu, and the self-navigation row OK already
    /// does is dropped — exactly as for a leaf. Play mode keeps today's rows.
    #[test]
    fn a_show_or_season_deck_card_leads_with_play_when_ok_opens_the_page() {
        for (kind, nav) in [(1, "Go to Show"), (2, "Go to Season")] {
            let m = item(kind, PosterMark::InProgress);
            let on = build_deck(&m, true, DeckPress::Details);
            let l = labels(&on);
            assert_eq!(l[0], "Play", "{kind}: {l:?}");
            assert!(!l.iter().any(|x| x == nav), "{kind}: {l:?}");
            assert!(!on.offers(ItemRow::GoToItem) && !on.offers(ItemRow::GoToShow), "{kind}");
            assert_eq!(on.action(ItemRow::Play), Action::Play("42".into()));
            assert_eq!(on.0.opening_key(), Some(ItemRow::Play.key()), "{kind}: opens on Play");
            assert_ne!(l[1], "—", "{kind}: no separator under Play: {l:?}");
            assert_ne!(l.last().unwrap(), "—", "{kind}: {l:?}");
            assert!(!l.windows(2).any(|w| w[0] == "—" && w[1] == "—"), "{kind}: {l:?}");
            // the remaining rows are the Play-mode menu's, minus its navigation group
            let play = labels(&build_deck(&m, true, DeckPress::Play));
            assert_eq!(play[0], nav, "{kind}: {play:?}");
            assert_eq!(l[1..], play[2..], "{kind}: {l:?} vs {play:?}");
            assert!(!play.iter().any(|x| x == "Play" || x == "Resume"), "{play:?}");
            // off the deck the setting is not consulted
            assert_eq!(
                labels(&build_deck(&m, false, DeckPress::Details)),
                labels(&build_deck(&m, false, DeckPress::Play)),
                "{kind}"
            );
        }
    }

    /// **The predicate the press and the menu share**: only an episode that names its show.
    #[test]
    fn only_an_episode_with_a_show_opens_the_show_from_the_deck() {
        assert!(deck_episode_opens_show(&item(3, PosterMark::None)));
        for kind in [0, 1, 2, 4] {
            assert!(!deck_episode_opens_show(&item(kind, PosterMark::None)), "kind {kind}");
        }
        let mut orphan = item(3, PosterMark::None);
        orphan.show_rk.clear();
        assert!(!deck_episode_opens_show(&orphan), "no show to open: the episode page stays");
    }

    /// The menu follows the press: a deck episode in Open-details mode leads with the episode's
    /// page and drops Go to Show, and ONLY that card — every other card and mode is unchanged,
    /// including a discovery-shelf episode (`from_deck` false), whose OK still opens its own page.
    #[test]
    fn the_menu_offers_go_to_episode_exactly_when_ok_opens_the_show() {
        let ep = item(3, PosterMark::None);
        let on = build_deck(&ep, true, DeckPress::Details);
        assert!(on.offers(ItemRow::GoToItem) && !on.offers(ItemRow::GoToShow));
        // Play mode: OK plays, the menu is the full pair
        let play = build_deck(&ep, true, DeckPress::Play);
        assert!(play.offers(ItemRow::GoToItem) && play.offers(ItemRow::GoToShow));
        // a discovery shelf's episode card: OK opens the episode, whatever the setting
        for mode in DeckPress::LADDER {
            let off = build_deck(&ep, false, mode);
            assert_eq!(
                labels(&off),
                ["Go to Episode", "Go to Show", "—", "Mark as Watched", "Play from Start"],
                "{mode:?}"
            );
        }
        // a deck movie is unchanged by this: no navigation row at all
        let movie = build_deck(&item(0, PosterMark::None), true, DeckPress::Details);
        assert!(!movie.offers(ItemRow::GoToItem) && !movie.offers(ItemRow::GoToShow));
    }

    #[test]
    fn the_play_mode_and_off_deck_menus_are_unchanged() {
        for kind in [0, 3] {
            let today = labels(&build(&item(kind, PosterMark::None), true));
            assert!(!today.iter().any(|l| l == "Play" || l == "Resume"), "{today:?}");
            assert_eq!(labels(&build_deck(&item(kind, PosterMark::None), true, DeckPress::Play)), today);
            // off the deck the setting is not consulted at all
            for mode in DeckPress::LADDER {
                assert_eq!(
                    labels(&build_deck(&item(kind, PosterMark::None), false, mode)),
                    labels(&build(&item(kind, PosterMark::None), false)),
                    "{kind} {mode:?}"
                );
            }
        }
        let play_mode = build_deck(&item(0, PosterMark::None), true, DeckPress::Play);
        assert!(play_mode.offers(ItemRow::GoToItem) && !play_mode.offers(ItemRow::Play));
        let off_deck = build_deck(&item(3, PosterMark::None), false, DeckPress::Details);
        assert!(off_deck.offers(ItemRow::GoToItem) && !off_deck.offers(ItemRow::Play));
    }

    #[test]
    fn an_episode_offers_the_pinned_row_set_in_order() {
        let menu = build(&item(3, PosterMark::None), false);
        assert_eq!(
            labels(&menu),
            [
                "Go to Episode",
                "Go to Show",
                "—",
                "Mark as Watched",
                "Play from Start"
            ]
        );
        // the separator is not a row: every BOUND row acts, and there are four of them
        assert_eq!(
            menu.ids(),
            [ItemRow::GoToItem, ItemRow::GoToShow, ItemRow::MarkWatched, ItemRow::PlayFromStart]
        );
        // "Go to Show" targets the SHOW rk + the episode's season, not the episode
        match menu.action(ItemRow::GoToShow) {
            Action::GoToShow(rk, season) => assert_eq!((rk.as_str(), season), ("7", 3)),
            _ => panic!("Go to Show must commit GoToShow"),
        }
    }

    /// The hero's … menu: one row per moved action, in the row's order, each wearing the icon of the
    /// round button it replaces and committing what that button does. Play Version opens the versions page.
    #[test]
    fn the_hero_overflow_menu_mirrors_the_round_buttons_it_replaces() {
        let rows = [HeroCtl::GoToShow, HeroCtl::Restart, HeroCtl::Version, HeroCtl::AudioSubtitles];
        let menu = built(super::build_hero_overflow(&rows, &|_| None));
        assert_eq!(menu.ids(), rows.map(ItemRow::Hero), "a row's identity is its control");
        for ctl in rows {
            assert_eq!(menu.action(ItemRow::Hero(ctl)), Action::Hero(ctl), "a row's action is its control");
        }
        // The mark and the verb are the round button's, from the one table, not literals of this menu.
        for (i, ctl) in rows.iter().enumerate() {
            let (icon, label) = hero_face(*ctl).expect("an overflowable control has a face");
            let row = &menu.0.table.sections[0].rows[i];
            assert_eq!((row.licon, row.label.as_str()), (Some(icon), label.to_str().unwrap()), "{ctl:?}");
        }
        // two rows or more: the menu never stands for a single item either
        assert!(menu.ids().len() >= 2);
    }

    /// The hold / item menu's Play from Start and Play Version rows wear the hero buttons' icons.
    #[test]
    fn the_hold_menus_play_rows_wear_the_hero_buttons_icons() {
        let menu = built(super::build_episode("e", PosterMark::InProgress, &plx_appkit::version_list::sample(), None));
        let icon = |id: ItemRow| menu.0.index_of(&id).and_then(|i| menu.0.table.sections[0].rows[i].licon);
        assert_eq!(icon(ItemRow::PlayFromStart), Some(Icon::Restart));
        assert_eq!(icon(ItemRow::PlayVersion), Some(Icon::Collection));
        assert_eq!(icon(ItemRow::MarkWatched), Some(Icon::CheckCircleFill));
        assert_eq!(icon(ItemRow::MarkUnwatched), Some(Icon::MinusCircleFill));
    }

    #[test]
    fn a_watched_leaf_offers_only_the_way_back() {
        let menu = build(&item(0, PosterMark::Watched), false); // movie, viewCount >= 1
        assert_eq!(
            labels(&menu),
            ["Go to Movie", "—", "Mark as Unwatched", "Play from Start"]
        );
        assert_eq!(
            menu.action(ItemRow::MarkUnwatched).watch_write(),
            Some(plx_data::viewstate::Write::Unwatched),
            "a finished item has one end left to be sent to"
        );
        // a movie has no show, so no second navigation row
        assert!(!menu.offers(ItemRow::GoToShow));
    }

    #[test]
    fn a_collection_has_no_item_menu_actions() {
        let collection = PmsMovie { rk: "42".into(), kind: plx_data::pms::KIND_COLLECTION,
            ..Default::default() };
        assert!(!has_actions(&collection), "a collection menu must not be openable before its page exists");
    }

    /// A show or season never offers *Play from Start* — there is no single part to start — and
    /// its watch state is the flag PAIR, not one flag: `!unwatched` alone means "some leaf was
    /// watched", which is the middle of the range and not the end of it.
    #[test]
    fn a_show_has_no_play_from_start_and_gets_the_pair_when_it_is_mid_run() {
        let menu = build(&item(1, PosterMark::InProgress), false);
        assert_eq!(
            labels(&menu),
            ["Go to Show", "—", "Mark as Watched", "Mark as Unwatched"]
        );
        assert_eq!(
            menu.action(ItemRow::MarkWatched).watch_write(),
            Some(plx_data::viewstate::Write::Watched)
        );
        assert_eq!(
            menu.action(ItemRow::MarkUnwatched).watch_write(),
            Some(plx_data::viewstate::Write::Unwatched)
        );

        // a show whose every leaf is seen is DONE, and offering to mark it watched again was the
        // old row set's other wrong answer (it read `!unwatched`, which cannot tell the two apart)
        let menu = build(&item(1, PosterMark::Watched), false);
        assert_eq!(labels(&menu), ["Go to Show", "—", "Mark as Unwatched"]);
        // …and one nobody has opened offers only the way forward
        let menu = build(&item(1, PosterMark::None), false);
        assert_eq!(labels(&menu), ["Go to Show", "—", "Mark as Watched"]);
    }

    #[test]
    fn a_row_whose_target_is_missing_is_not_offered_at_all() {
        // an episode hub row that arrived without a grandparentRatingKey: no "Go to Show" — the row
        // would resolve to an empty rk, i.e. a blocking fetch for nothing and a blank page
        let mut m = item(3, PosterMark::None);
        m.show_rk.clear();
        let menu = build(&m, false);
        assert_eq!(
            labels(&menu),
            ["Go to Episode", "—", "Mark as Watched", "Play from Start"]
        );
        assert!(!menu.offers(ItemRow::GoToShow));
        // and a SEASON with no parent has no navigation at all — so no leading separator either,
        // which would otherwise open the menu with a rule above its first row
        let mut s = item(2, PosterMark::None);
        s.show_rk.clear();
        let menu = build(&s, false);
        assert_eq!(labels(&menu), ["Mark as Watched"]);
    }

    /// **Remove from Continue Watching** is gated on the SHELF, not on the item: only a card that
    /// came from the deck has a deck to leave. Offered anywhere else it would be a row that appeared
    /// to work and silently changed nothing, since the server-side action only affects that hub.
    ///
    /// Pinned last in the group on purpose — it is the one row here that takes something out of
    /// view, so it sits where a mis-press is least likely, and the assertion below is what keeps a
    /// later row from being appended after it.
    #[test]
    fn the_remove_from_deck_row_exists_only_on_a_continue_watching_card() {
        // same card, both shelves — the ONLY difference is where it was focused
        let off_deck = build(&item(3, PosterMark::None), false);
        assert_eq!(
            labels(&off_deck),
            [
                "Go to Episode",
                "Go to Show",
                "—",
                "Mark as Watched",
                "Play from Start"
            ]
        );
        assert!(
            !off_deck.offers(ItemRow::RemoveFromDeck),
            "a card off the deck must not offer to remove it from one"
        );

        let on_deck = build(&item(3, PosterMark::None), true);
        assert_eq!(
            labels(&on_deck),
            [
                "Go to Episode",
                "Go to Show",
                "—",
                "Mark as Watched",
                "Play from Start",
                "Remove from Deck"
            ],
            "…and on the deck it is the LAST row, after the watched toggle"
        );
        assert_eq!(on_deck.ids().last(), Some(&ItemRow::RemoveFromDeck), "it is the last row");
        match on_deck.action(ItemRow::RemoveFromDeck) {
            Action::RemoveFromDeck(rk) => {
                assert_eq!(rk, "42", "…carrying the card's own ratingKey")
            }
            _ => panic!("the deck row must commit the deck removal"),
        }
        // it is an ADDITION to the group, not a replacement — the watch-state row is still there
        assert!(on_deck.offers(ItemRow::MarkWatched));
    }

    /// The detail page's filmstrip menu: the state rows, no navigation group, and therefore no
    /// leading separator. It shares `state_rows` with the shelf menu, so the row the owner asked
    /// for cannot say one thing on Home and another on the episode page — asserted by comparing the
    /// two builders' tails rather than by re-listing the labels here.
    #[test]
    fn the_filmstrip_menu_is_the_state_group_alone() {
        let menu = build_episode("42", PosterMark::None);
        assert_eq!(labels(&menu), ["Mark as Watched", "Play from Start"]);
        assert_eq!(menu.ids().len(), 2, "no separator, so every row acts");
        // the shelf menu's episode rows END with exactly these two, in this order
        let shelf = build(&item(3, PosterMark::None), false);
        let shelf_labels = labels(&shelf);
        assert_eq!(labels(&menu), shelf_labels[shelf_labels.len() - 2..]);

        // a watched episode gets the way back instead, carrying its OWN rk (the menu captured its
        // target when it was presented — nothing here may resolve through a live focus index)
        let menu = build_episode("77", PosterMark::Watched);
        assert_eq!(labels(&menu), ["Mark as Unwatched", "Play from Start"]);
        match menu.action(ItemRow::MarkUnwatched) {
            Action::MarkUnwatched(rk) => assert_eq!(rk, "77"),
            _ => panic!("the unwatched row must commit MarkUnwatched"),
        }
        match menu.action(ItemRow::PlayFromStart) {
            Action::PlayFromStart(rk) => assert_eq!(rk, "77"),
            _ => panic!("Play from Start must commit PlayFromStart"),
        }
        // and nothing navigates: both rows the shelf menu offers an episode are dead ends from the
        // page that episode already belongs to
        assert!(!menu.offers(ItemRow::GoToShow) && !menu.offers(ItemRow::GoToItem));
    }

    /// **A menu never opens with its focus on a destructive action.** *Mark as Unwatched* throws the
    /// watch record away (and propagates to the other copies), so a watched episode's menu opens on
    /// *Play from Start*; a watched season, whose one row it is, still opens on it. *Remove from
    /// Deck* is last in the shelf menu and is tagged too, so no reorder can make it the opening row.
    #[test]
    fn a_menu_never_opens_on_a_destructive_row() {
        let opening = |menu: Built| {
            let id = *menu.0.id_at(menu.0.table.sel as usize).expect("the opening row acts");
            menu.action(id)
        };
        assert!(matches!(opening(build_episode("77", PosterMark::Watched)),
            Action::PlayFromStart(_)), "a watched episode opened on Mark as Unwatched");
        assert!(matches!(opening(build_season("78", PosterMark::Watched)),
            Action::MarkUnwatched(_)), "the only row is the one on offer");
        assert!(matches!(opening(build_episode("79", PosterMark::None)),
            Action::MarkWatched(_)));
        let menu = build(&item(3, PosterMark::None), true);
        assert!(menu.offers(ItemRow::RemoveFromDeck), "rig: a deck card");
        assert!(menu.is_destructive(ItemRow::RemoveFromDeck));
        assert!(menu.is_destructive(ItemRow::MarkUnwatched) || !menu.offers(ItemRow::MarkUnwatched));
    }

    /// **The owner-reported gap, at both entry points.** An item in the MIDDLE is at neither end of
    /// the watch range, so no single toggle can express both destinations — it gets both rows, ✓
    /// then −, the order the two ends of the range read in.
    ///
    /// The pair is asserted alongside its two neighbours in the same test on purpose: the property
    /// is not "there are two rows", it is that the row SET tracks the state, and a fixture that only
    /// ever built the middle would pass with an unconditional pair.
    #[test]
    fn a_part_watched_item_offers_both_ends_of_the_range() {
        // the shelf card (an episode with a live resume point)
        let tail = |mark| {
            labels(&build(&item(3, mark), false)).split_off(3) // past "Go to Episode", "Go to Show", the separator
        };
        assert_eq!(
            tail(PosterMark::None),
            ["Mark as Watched", "Play from Start"]
        );
        assert_eq!(
            tail(PosterMark::InProgress),
            ["Mark as Watched", "Mark as Unwatched", "Play from Start"]
        );
        assert_eq!(
            tail(PosterMark::Watched),
            ["Mark as Unwatched", "Play from Start"]
        );

        // …and the detail page's filmstrip, off the same builder, so the two cannot drift
        let strip = |mark| {
            labels(&build_episode("42", mark))
        };
        assert_eq!(
            strip(PosterMark::None),
            ["Mark as Watched", "Play from Start"]
        );
        assert_eq!(
            strip(PosterMark::InProgress),
            ["Mark as Watched", "Mark as Unwatched", "Play from Start"]
        );
        assert_eq!(
            strip(PosterMark::Watched),
            ["Mark as Unwatched", "Play from Start"]
        );
    }

    /// **A RELATED tile gets the same three-state row set as every other card**, built from a row
    /// that came off the wire rather than from this module's hand-made fixture.
    ///
    /// The Related shelf was the last card surface with no context menu, excluded on the stated
    /// grounds that its tiles carried no `(ratingKey, watched)` pair — true of the old struct, never
    /// of the response. So this half runs `/related`'s real JSON through the same `pms::parse_item`
    /// the shelf now uses, into `build`, and out as labels, rather than through this module's
    /// hand-made [`item`] fixture: the fixture sets the three flags directly, so it can only prove
    /// that `build` reads them, never that the WIRE fills them in.
    ///
    /// It is deliberately only that half. The other coupling — that `fetch_related` still hands the
    /// shelf a fully parsed row instead of going back to copying three fields — cannot be seen from
    /// here, because this test calls `parse_item` itself. That one is
    /// `metadata::tests::related_rows_carry_the_watch_state_the_wire_already_had`; the two are a
    /// pair and neither is sufficient alone.
    #[test]
    fn a_related_tile_off_the_wire_gets_the_row_set_its_state_earns() {
        let row = |json: &str| {
            let body = format!(r#"{{"MediaContainer":{{"Hub":[{{"Metadata":[{json}]}}]}}}}"#);
            let mc = serde_json::from_str::<plx_plex::plex::Envelope>(&body)
                .expect("parses")
                .media_container;
            plx_data::pms::parse_item(&mc.hub[0].metadata[0], plx_plex::plex::ServerId::UNSET)
        };
        let set = |json: &str| labels(&build(&row(json), false));

        // a MOVIE, in each of the three states — one navigation row, then the state group
        let movie = |extra: &str| {
            format!(r#"{{"ratingKey":"11","type":"movie","duration":"7020000"{extra}}}"#)
        };
        assert_eq!(
            set(&movie("")),
            ["Go to Movie", "—", "Mark as Watched", "Play from Start"],
            "never started: only the way forward"
        );
        assert_eq!(
            set(&movie(r#","viewOffset":"3510000""#)),
            [
                "Go to Movie",
                "—",
                "Mark as Watched",
                "Mark as Unwatched",
                "Play from Start"
            ],
            "part-watched: both ends are reachable and both are true"
        );
        assert_eq!(
            set(&movie(r#","viewCount":2"#)),
            ["Go to Movie", "—", "Mark as Unwatched", "Play from Start"],
            "finished: only the way back"
        );

        // a SHOW — a container, so no Play from Start, and its middle state is the leaf-count one
        let show =
            |extra: &str| format!(r#"{{"ratingKey":"14","type":"show","leafCount":10{extra}}}"#);
        assert_eq!(
            set(&show("")),
            ["Go to Show", "—", "Mark as Watched"],
            "no leaf viewed"
        );
        assert_eq!(
            set(&show(r#","viewedLeafCount":3"#)),
            ["Go to Show", "—", "Mark as Watched", "Mark as Unwatched"],
            "3 of 10: neither watched nor unwatched, so BOTH verbs — the case `viewCount > 0` misses"
        );
        assert_eq!(
            set(&show(r#","viewedLeafCount":10"#)),
            ["Go to Show", "—", "Mark as Unwatched"],
            "all 10"
        );

        // and no Related row offers the deck action: that shelf is not the Continue Watching deck
        let menu = build(&row(&movie("")), false);
        assert!(!labels(&menu).iter().any(|l| l == "Remove from Deck"));
        assert!(!menu.offers(ItemRow::RemoveFromDeck));
    }

    /// **A row performs the verb its own label names**, in every state and at both entry points —
    /// which is the assertion the old shape could not make at all: the action carried what the item
    /// WAS and `app.rs` inverted it, so "what this row does" lived in another file and, on the pair,
    /// would have been the same bool twice — one row doing its neighbour's write.
    #[test]
    fn every_watch_row_performs_the_verb_its_label_names() {
        use plx_data::viewstate::Write;
        let want = |l: &str| match l {
            "Mark as Watched" => Some(Write::Watched),
            "Mark as Unwatched" => Some(Write::Unwatched),
            _ => None, // navigation, the separator, Play from Start, Remove from Deck
        };
        for mark in [
            PosterMark::None,
            PosterMark::InProgress,
            PosterMark::Watched,
        ] {
            for kind in [0, 1, 2, 3] {
                for deck in [false, true] {
                    let menu = build(&item(kind, mark), deck);
                    for (label, got) in verbs(&menu) {
                        assert_eq!(got, want(&label), "{label:?} (kind {kind}, {mark:?})");
                    }
                }
            }
            let menu = build_episode("42", mark);
            for (label, got) in verbs(&menu) {
                assert_eq!(got, want(&label), "{label:?} on the filmstrip ({mark:?})");
            }
        }
    }

    /// Every row a menu draws is a row it can act on: a row and its action are declared in ONE
    /// place ([`ItemForm`]), so the shifted-neighbour failure the old parallel vector allowed —
    /// a row added without its action, every action below it moving up one — has no spelling left.
    /// States the property over every kind, state and entry point the builders serve: each drawn,
    /// non-separator row is bound, and its action names the item the menu is about.
    #[test]
    fn every_drawn_row_is_bound_to_an_action() {
        for mark in [
            PosterMark::None,
            PosterMark::InProgress,
            PosterMark::Watched,
        ] {
            for kind in [0, 1, 2, 3] {
                for deck in [false, true] {
                    let menu = build(&item(kind, mark), deck);
                    let drawn = menu.0.table.sections[0].rows.iter().filter(|r| !r.sep).count();
                    assert_eq!(menu.actions().len(), drawn, "kind {kind}, {mark:?}, deck {deck}");
                }
            }
            let menu = build_episode("42", mark);
            let drawn = menu.0.table.sections[0].rows.len();
            assert_eq!(menu.actions().len(), drawn, "filmstrip, {mark:?}");
        }
    }

    /// **The focus walk steps over the separator and stops at the ends.**
    ///
    /// It was `TableView::move_sel`'s own skip-and-clamp, exercised on a live `TableView`; the walk
    /// is the focus ENGINE's now, so the property is stated on the screen's own
    /// `Focusable::neighbour` — which is where the separator's unfocusability actually lives
    /// (`acts[i].is_none()`), and which is what the engine asks.
    #[test]
    fn the_focus_walk_steps_over_the_separator_and_stops_at_the_ends() {
        let screen = screen_with(super::build(&item(3, PosterMark::None), false));
        let key = |id: ItemRow| id.key().0;
        let step = |from: ItemRow, dir: Dir| {
            let from = key(from);
            match with_cx(|cx| {
                <ItemMenuScreen as Focusable<HostFixture>>::neighbour(
                    &screen,
                    FocusKey {
                        entry: EntryId(7),
                        elem: from,
                    },
                    dir,
                    cx,
                )
            }) {
                Step::Move(k) => Some(k.elem),
                _ => None,
            }
        };
        use ItemRow::*;
        assert_eq!(step(GoToItem, Dir::Down), Some(key(GoToShow)));
        assert_eq!(step(GoToShow, Dir::Down), Some(key(MarkWatched))); // the separator is skipped
        assert_eq!(step(MarkWatched, Dir::Down), Some(key(PlayFromStart)));
        assert_eq!(step(PlayFromStart, Dir::Down), None); // stops at the end
        assert_eq!(step(MarkWatched, Dir::Up), Some(key(GoToShow))); // skipped back over the separator
        assert_eq!(step(GoToShow, Dir::Up), Some(key(GoToItem)));
        assert_eq!(step(GoToItem, Dir::Up), None); // stops at the start
    }

    /// A focus key that names no row settles onto the opening row — the ENGINE asks `reconcile`,
    /// which answers the opening row for anything that is not one. The separator has no key at
    /// all, so there is no element a cursor could land on it with.
    #[test]
    fn a_key_naming_no_row_settles_onto_a_real_row() {
        let screen = screen_with(super::build(&item(3, PosterMark::None), false));
        let want = FocusKey {
            entry: EntryId(7),
            elem: 0xdead,
        };
        let got =
            with_cx(|cx| <ItemMenuScreen as Focusable<HostFixture>>::reconcile(&screen, want, cx));
        assert_eq!(got.elem, ItemRow::GoToItem.key().0, "…the first real row is where it lands");
        // the menu's own cursor is where focus belongs (a moved focus reaches it first, `Event::Focused`): a row it was
        // never seated on is not kept
        let live = FocusKey { entry: EntryId(7), elem: ItemRow::MarkWatched.key().0 };
        let unseated =
            with_cx(|cx| <ItemMenuScreen as Focusable<HostFixture>>::reconcile(&screen, live, cx));
        assert_eq!(unseated.elem, ItemRow::GoToItem.key().0, "the cursor wins over a live row the menu never saw");
        let mut screen = screen;
        screen.menu.step(Event::Focused(live.elem), &mut |_| None);
        assert_eq!(screen.menu.form().selected_key().map(|k| k.0), Some(live.elem));
        let kept =
            with_cx(|cx| <ItemMenuScreen as Focusable<HostFixture>>::reconcile(&screen, live, cx));
        assert_eq!(kept.elem, live.elem, "a row the cursor is on is left where it is");
    }

    /// **The menu carries the row it was opened on**, and it is the ENTRY's argument now rather
    /// than a `static mut ITEM`. That capture is the fix for a Play-from-Start that resolved its
    /// target through the HOME hub catalog (`pms::index_of_rk`) and so silently did nothing on
    /// every other card surface — and it is a stronger claim as an argument than as a global: the
    /// row cannot be re-pointed by a hub refetch rebuilding the catalog under an open panel, and
    /// the episode / season entry points carry NONE rather than the last card's.
    ///
    /// The third property the legacy test asserted — that the capture SURVIVES the close, because
    /// `on_ok` closed and the drain read the static a frame later — is gone with the static that
    /// needed it: the request carries the row, so nothing is read after the dismissal at all.
    #[test]
    fn the_menu_carries_the_row_it_was_opened_on_and_the_episode_menu_carries_none() {
        let mut m = item(0, PosterMark::None);
        m.sid = plx_plex::plex::ServerId::from_raw(3);
        m.part = "/library/parts/42/file.mkv".to_string();

        let mut screen = ItemMenuScreen::new(EntryId(7), card_arg(&m, false));
        screen.build_rows(test_store().view(), DeckPress::Play);
        let elem = first_action(&screen, |a| matches!(a, Action::PlayFromStart(_)));
        let req = commit(&mut screen, elem);
        assert_eq!(
            req.sid,
            plx_plex::plex::ServerId::from_raw(3),
            "the ROW's server, not the current one"
        );
        let carried = req.item.expect("the row the panel is about");
        assert_eq!(carried.rk, "42");
        assert_eq!(
            carried.part, "/library/parts/42/file.mkv",
            "the WHOLE row — a key alone cannot start playback"
        );
        assert!(
            !req.loaded_episode,
            "a card row is not a leaf of a loaded season"
        );

        let mut strip = ItemMenuScreen::new(
            EntryId(8),
            ItemMenuArg {
                sid: plx_plex::plex::ServerId::from_raw(3),
                rk: "77".into(),
                kind: ItemMenuKind::Episode {
                    mark: PosterMark::None,
                    versions: Vec::new(),
                },
                host: EntryId(1),
                focus: None,
                anchor: [0; 4],
                loaded_episode: true,
                from_home: false,
                catalog: None,
                watchlist: None,
            },
        );
        strip.build_rows(test_store().view(), DeckPress::Play);
        let elem = first_action(&strip, |a| matches!(a, Action::PlayFromStart(_)));
        let req = commit(&mut strip, elem);
        assert!(
            req.item.is_none(),
            "an episode menu plays through the loaded season, never a stale row"
        );
        assert!(
            req.loaded_episode,
            "…and says so, which is what routes it there"
        );
    }

    #[test]
    fn a_season_tab_menu_carries_its_season_to_the_mark_request() {
        let mut tab = ItemMenuScreen::new(
            EntryId(8),
            ItemMenuArg {
                sid: plx_plex::plex::ServerId::from_raw(3),
                rk: "season-9".into(),
                kind: ItemMenuKind::Season {
                    mark: PosterMark::None,
                    show_rk: "show-1".into(),
                    index: 4,
                },
                host: EntryId(1),
                focus: None,
                anchor: [0; 4],
                loaded_episode: false,
                from_home: false,
                catalog: None,
                watchlist: None,
            },
        );
        tab.build_rows(test_store().view(), DeckPress::Play);
        let elem = first_action(&tab, |a| matches!(a, Action::MarkWatched(_)));
        let req = commit(&mut tab, elem);
        assert!(req.item.is_none(), "the tab is no catalog row");
        assert_eq!(
            req.season,
            Some(("show-1".to_string(), 4)),
            "the season's address is what scopes the write to its episodes"
        );
    }

    #[test]
    fn translated_action_menus_measure_complete_verbs_and_keep_safe_anchors() {
        use plx_os::i18n::{LocaleContext, Preference};
        use plx_machine::machine::Measure;
        struct MenuMeasure;
        impl Measure for MenuMeasure {
            fn width(&self, text: &std::ffi::CStr, size: i32, bold: bool) -> f32 {
                assert!(!bold, "compact action labels keep BODY regular");
                text.to_string_lossy().chars().count() as f32 * size as f32 * 0.6
            }
            fn cap_h(&self, size: i32) -> f32 { size as f32 * 0.7 }
            fn line_h(&self, size: i32) -> f32 { size as f32 * 1.2 }
        }
        let measure = MenuMeasure;
        for preference in plx_os::i18n::SHIPPED {
            let locale = LocaleContext::resolve(preference, None, None, None, None);
            let mut form: FormTable<ItemRow, Action, MenuPage> = FormTable::with_class(crate::registry::BAND, Class::Action);
            form.set(Form::new().section(FormSection::new("")
                .item(ItemRow::MarkWatched, RowKind::Button, Action::MarkWatched("1".into()),
                    Row::new(plx_os::i18n::msg::widgets_action_mark_watched_in(&locale)).licon(Icon::CheckCircleFill))
                .item(ItemRow::MarkUnwatched, RowKind::Button, Action::MarkUnwatched("1".into()),
                    Row::new(plx_os::i18n::msg::widgets_action_mark_unwatched_in(&locale)).licon(Icon::MinusCircleFill))
                .item(ItemRow::RemoveFromDeck, RowKind::Button, Action::RemoveFromDeck("1".into()),
                    Row::new(plx_os::i18n::msg::browse_menu_remove_deck_in(&locale)).licon(Icon::Close))), None);
            let table = &form.table;
            let width = table.measured_width(&measure);
            if preference == Preference::Be {
                assert!(width > MENU_MIN_W, "the failing Belarusian menu must exercise growth");
            }
            for x in [MARGIN_X, (SCR_W - CARD_W) * 0.5, SCR_W - MARGIN_X - CARD_W] {
                let anchor = Rect::new(x, SCR_H - 120.0, CARD_W, CARD_H);
                let frame = panel_at(anchor, width, table.measured_height());
                assert!(plx_ui::consts::inside_safe(frame));
                assert!(frame.x >= anchor.x + anchor.w || frame.x + frame.w <= anchor.x,
                    "the measured menu must remain beside a visible card");
                for row in &table.sections[0].rows {
                    let label_width = measure.width_str(&row.label, plx_ui::theme::size::BODY, false);
                    assert!(label_width <= table.label_width(row, frame.w, &measure),
                        "{preference:?} action was elided: {}", row.label);
                }
            }
        }
    }

    #[test]
    fn the_panel_sits_beside_the_card_and_never_leaves_the_screen() {
        let h = 5.0 * 60.0; // a five-row menu, roughly
                            // a card on the left of the shelf: the panel sits to its RIGHT, clear of the card
        let r = panel_at(Rect::new(MARGIN_X, 300.0, CARD_W, CARD_H), MENU_MIN_W, h);
        assert!(
            r.x >= MARGIN_X + CARD_W,
            "expected the panel right of the card, got x={}",
            r.x
        );
        // a card at the right edge: it flips LEFT rather than running off screen — and lands clear
        // of the card it belongs to, which is the whole point of anchoring beside it
        let a = Rect::new(SCR_W - 300.0, 300.0, CARD_W, CARD_H);
        let r = panel_at(a, MENU_MIN_W, h);
        assert!(
            r.x + r.w <= SCR_W - EDGE + 0.5,
            "panel ran off the right edge: x={} w={}",
            r.x,
            r.w
        );
        assert!(
            r.x + r.w <= a.x,
            "flipped panel overlaps its card: x={} w={} card.x={}",
            r.x,
            r.w,
            a.x
        );
        assert!(r.x >= EDGE - 0.5);
        // a card near the bottom keeps the whole panel on screen
        let low = panel_at(Rect::new(MARGIN_X, SCR_H - 120.0, CARD_W, CARD_H), MENU_MIN_W, h);
        assert!(
            low.y + low.h <= SCR_H - EDGE + 0.5,
            "panel ran off the bottom: y={} h={}",
            low.y,
            low.h
        );
        assert!(low.y >= EDGE - 0.5);

        // …and "on screen" means inside the OVERSCAN frame, which is why the keep-out is per axis:
        // `space::XL` 64 clears `MARGIN_Y` and misses `MARGIN_X` by 32. A panel placed against an
        // anchor has no fixed rect a table could carry, so the frame is graded on its extremes here.
        let tall = panel_at(Rect::new(MARGIN_X, 300.0, CARD_W, CARD_H), MENU_MIN_W, 4000.0);
        for (what, p) in [("flipped", r), ("low", low), ("tall", tall)] {
            assert!(
                plx_ui::consts::inside_safe(p),
                "the {what} panel leaves the safe area: ({}, {}) {}x{}",
                p.x,
                p.y,
                p.w,
                p.h
            );
        }
    }

    /// **A held direction walks the list at the panel's own cadence, not the hardware's.**
    ///
    /// `app/run.rs` did this from a per-frame `App::held_key` timer at 110 ms, and this menu was
    /// its LAST consumer — the block, the field and `HeldKey::arm` all go with it. The dispatcher
    /// hands the surface `Edge::Repeat` at the remote's own ~50 ms, so the cadence has to be
    /// applied by the surface that receives it: the same `RepeatGate` / [`PANEL_REPEAT_MS`] the
    /// player's four panels take, reused rather than copied.
    ///
    /// A swallowed repeat is `Handled::Yes` (the engine never sees it); an admitted one is
    /// `Handled::No`, which is what hands the direction to the focus engine. Red first, observed:
    /// with the `RepeatGate` arm deleted from `step`, every repeat falls through and the engine
    /// walks the list at the hardware rate — 12 admitted steps in the window below against 4.
    #[test]
    fn the_item_menu_hold_repeats_through_the_surface_not_the_loop() {
        let mut screen = screen_with(super::build(&item(3, PosterMark::None), false));
        // the fresh press: acted on unconditionally, and it re-arms the cadence from itself
        assert_eq!(feed(&mut screen, Key::Down, Edge::Down, 1000), Handled::No);
        // the hardware's own repeats, 50 ms apart
        let mut admitted = 0;
        for i in 1..=12u32 {
            if feed(&mut screen, Key::Down, Edge::Repeat, 1000 + i * 50) == Handled::No {
                admitted += 1;
            }
        }
        // Four, not the 5.5 the ratio suggests: the gate admits a step at the first repeat AT OR
        // PAST the cadence, and the hardware's 50 ms lattice means that is every third one — the
        // same rounding the loop's own 110 ms timer had against a 16 ms frame.
        assert_eq!(
            admitted, 4,
            "600 ms of hardware repeat is 4 steps at {PANEL_REPEAT_MS} ms, not 12 at the remote's 50"
        );
        // …and a FRESH press is never swallowed by the cadence of the press before it
        assert_eq!(feed(&mut screen, Key::Up, Edge::Down, 1620), Handled::No);
    }

    // ---- fixtures ---------------------------------------------------------------------------

    use crate::registry::{AppMsg, PageMemory};
    use plx_ui::fixture::FixtureMeasure;
    use plx_machine::machine::{FocusRead, Host, InputEvent, InputOwner, PressRead, Source, Tick};
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
        type Views<'a> = &'a crate::registry::ScreenInputs;
        type Init = Arg;
        type Memory = PageMemory;
    }
    thread_local! {
        // TEST ONLY: see `screens::detail::tests`'s `TEST_METADATA` for why this lives here
        // rather than being threaded as a parameter.
        static TEST_METADATA: std::cell::UnsafeCell<plx_data::stores::metadata::MetadataStore> =
            std::cell::UnsafeCell::new(plx_data::stores::metadata::MetadataStore::default());
    }

    fn test_store() -> &'static mut plx_data::stores::metadata::MetadataStore {
        TEST_METADATA.with(|cell| unsafe { &mut *cell.get() })
    }

    impl crate::registry::MetadataLike for HostFixture {
        fn metadata<'a>(_cx: &Cx<'a, Self>) -> plx_data::metadata::MetadataView<'a> {
            test_store().view()
        }
    }

    impl crate::registry::ScreenInputsLike for HostFixture {
        fn screen_inputs<'a>(cx: &Cx<'a, Self>) -> &'a crate::registry::ScreenInputs { cx.views }
    }

    /// One Activate (a click or OK on the row with this key) as the dispatcher delivers it.
    fn activate(s: &mut ItemMenuScreen, elem: u32, fx: &mut Effects<'_, HostFixture>) {
        with_cx(|cx| {
            Machine::<HostFixture>::step(s, &ScreenEvent::Activate(elem), cx, fx);
        });
    }

    fn with_cx<R>(test: impl FnOnce(&Cx<'_, HostFixture>) -> R) -> R {
        let measure = FixtureMeasure;
        let inputs = crate::registry::ScreenInputs::default();
        test(&Cx {
            views: &inputs,
            tick: Tick::default(),
            measure: &measure,
            focus: FocusRead {
                current: None,
                ..Default::default()
            },
            press: PressRead::default(),
            modal: Default::default(), page_image: Default::default(), hold_hint: Default::default(),
            owner: InputOwner::Entry(EntryId(7)),
        })
    }

    fn card_arg(m: &PmsMovie, from_deck: bool) -> ItemMenuArg {
        ItemMenuArg {
            sid: m.sid,
            rk: m.rk.clone(),
            kind: ItemMenuKind::Card {
                row: Box::new(m.clone()),
                from_deck,
            },
            host: EntryId(1),
            focus: None,
            anchor: [0; 4],
            loaded_episode: false,
            from_home: true,
            catalog: None,
            watchlist: None,
        }
    }

    /// A mounted screen whose row set is already built, so a test can state a property about the
    /// walk or the cadence without going through the container.
    fn screen_with(form: ItemForm) -> ItemMenuScreen {
        let mut s = ItemMenuScreen::new(EntryId(7), card_arg(&item(3, PosterMark::None), false));
        s.built = true;
        s.set_root(form);
        s
    }

    #[test]
    fn a_credit_menu_only_adds_its_captured_catalog_identity() {
        let mut arg = card_arg(&PmsMovie::default(), false);
        arg.sid = plx_plex::plex::ServerId::UNSET;
        arg.kind = ItemMenuKind::WatchlistAdd { hex: [0xab; 12] };
        arg.from_home = false;
        let mut screen = ItemMenuScreen::new(EntryId(7), arg.clone());
        screen.build_rows(test_store().view(), DeckPress::Play);
        assert_eq!(screen.form().table.n_rows(), 1);
        let elem = first_action(&screen, |a| matches!(a, Action::WatchlistAdd { .. }));
        let req = commit(&mut screen, elem);
        assert_eq!(req.act, Action::WatchlistAdd { hex: [0xab; 12] });
        assert!(req.item.is_none() && req.season.is_none() && !req.loaded_episode);

        let mut other = arg.clone();
        other.kind = ItemMenuKind::WatchlistAdd { hex: [0xcd; 12] };
        assert!(arg != other, "two credits cannot reuse the same menu identity");
        assert_ne!(arg.hash(), other.hash(), "the captured catalog identity is canonical");
    }

    /// The focus key of the first row whose action satisfies `want`.
    fn first_action(s: &ItemMenuScreen, want: impl Fn(&Action) -> bool) -> u32 {
        (0..s.form().table.n_rows() as usize)
            .find(|i| s.form().binding_at(*i).is_some_and(|b| want(&b.action)))
            .and_then(|i| s.form().key_at(i))
            .expect("the row set offers this action")
            .0
    }

    /// Commit a row and return the ONE request it emitted.
    fn commit(s: &mut ItemMenuScreen, elem: u32) -> ItemMenuReq {
        let mut out = Vec::new();
        let mut present = plx_machine::present::Present::new();
        let mut fx = Effects::new(
            &mut out,
            plx_machine::machine::MachineId::Instance(plx_machine::machine::InstanceId(8)),
            &mut present,
        );
        activate(s, elem, &mut fx);
        let mut req = None;
        let mut dismissed = false;
        for e in out {
            match e.fx {
                Fx::App(AppFx::ItemMenu(r)) => {
                    assert!(dismissed, "dismissal must precede yielding the action to the app runner");
                    assert!(req.is_none(), "one commit emits exactly one action");
                    req = Some(r);
                }
                Fx::Nav(NavOp::Dismiss(_)) => dismissed = true,
                _ => {}
            }
        }
        assert!(
            dismissed,
            "every commit dismisses, exactly as the legacy `on_ok` closed first"
        );
        req.expect("a row that acts reports its action")
    }

    /// Step the screen with one event through a throwaway `Effects`.
    fn step_ev(s: &mut ItemMenuScreen, ev: &ScreenEvent<HostFixture>) -> Handled {
        let mut out = Vec::new();
        let mut present = plx_machine::present::Present::new();
        let mut fx = Effects::new(
            &mut out,
            plx_machine::machine::MachineId::Instance(plx_machine::machine::InstanceId(8)),
            &mut present,
        );
        with_cx(|cx| <ItemMenuScreen as Machine<HostFixture>>::step(s, ev, cx, &mut fx))
    }

    fn feed(s: &mut ItemMenuScreen, key: Key, edge: Edge, ms: u32) -> Handled {
        let ev = ScreenEvent::<HostFixture>::Input(InputEvent {
            at: Tick { ms, dt_us: 0 },
            source: Source::Sdl,
            kind: InputKind::Key {
                key,
                sym: 0,
                wcode: 0,
                edge,
                at_edge: false,
            },
        });
        step_ev(s, &ev)
    }

    #[test]
    fn census_f2_mount_uses_owner_policy_and_open_rows_stay_frozen() {
        let _lock = plx_base::testlock::serial();
        let ambient = plx_media::route::deck_press();
        let mut details = ItemMenuScreen::new(EntryId(7), card_arg(&item(0, PosterMark::None), true));
        let mut play = ItemMenuScreen::new(EntryId(8), card_arg(&item(0, PosterMark::None), true));
        let drive = |screen: &mut ItemMenuScreen, mode, ev| {
            let inputs = crate::registry::ScreenInputs {
                preferences: Some(plx_plex::plex::session::ProfilePrefs { deck_press: Some(mode), ..Default::default() }),
                ..Default::default()
            };
            with_cx(|cx| {
                let cx = Cx { views: &inputs, tick: cx.tick, measure: cx.measure,
                    focus: Default::default(), press: Default::default(), owner: cx.owner,
                    modal: cx.modal, page_image: cx.page_image, hold_hint: cx.hold_hint };
                let mut out = Vec::new();
                let mut present = plx_machine::present::Present::new();
                let mut fx = Effects::new(&mut out, plx_machine::machine::MachineId::Instance(plx_machine::machine::InstanceId(8)), &mut present);
                <ItemMenuScreen as Machine<HostFixture>>::step(screen, &ev, &cx, &mut fx);
            });
        };
        plx_media::route::restore_deck_press(DeckPress::Play);
        drive(&mut details, DeckPress::Details, ScreenEvent::Mount);
        drive(&mut play, DeckPress::Play, ScreenEvent::Mount);
        assert!(matches!(details.form().binding_at(0).unwrap().action, Action::Play(_)), "owner Details must offer Play, irrespective of ambient Play");
        assert!(matches!(play.form().binding_at(0).unwrap().action, Action::GoToItem(_)));
        let selected = details.form().table.sel;
        plx_media::route::restore_deck_press(DeckPress::Details);
        drive(&mut details, DeckPress::Play, ScreenEvent::Mount);
        drive(&mut play, DeckPress::Details, ScreenEvent::Mount);
        assert!(matches!(details.form().binding_at(0).unwrap().action, Action::Play(_)));
        assert!(matches!(play.form().binding_at(0).unwrap().action, Action::GoToItem(_)));
        assert_eq!(details.form().table.sel, selected);
        plx_media::route::restore_deck_press(ambient);
    }

    /// **Opening the menu from ANY card screen retires Home's "Hold OK for options" hint**: the
    /// mount is the one door every presenter goes through, so the lesson is marked there.
    #[test]
    fn mounting_the_menu_retires_the_hold_hint() {
        let mut read = plx_machine::machine::HoldHintRead::default();
        let mut screen = ItemMenuScreen::new(EntryId(7), card_arg(&item(0, PosterMark::None), false));
        assert!(!read.learned);
        let mut out = Vec::new();
        let mut present = plx_machine::present::Present::new();
        let mut fx = Effects::new(&mut out,
            plx_machine::machine::MachineId::Instance(plx_machine::machine::InstanceId(8)), &mut present);
        with_cx(|cx| {
            let context = Cx { views: cx.views, tick: cx.tick, measure: cx.measure,
                focus: Default::default(), press: Default::default(), owner: cx.owner,
                modal: cx.modal, page_image: cx.page_image, hold_hint: read };
            <ItemMenuScreen as Machine<HostFixture>>::step(&mut screen, &ScreenEvent::Mount, &context, &mut fx);
        });
        drop(fx);
        let events: Vec<_> = out.iter().filter_map(|effect| match &effect.fx {
            plx_machine::machine::Fx::HoldHint(event) => Some(*event),
            _ => None,
        }).collect();
        assert_eq!(events, vec![plx_machine::machine::HoldHintEvent::Learned]);
        for event in events { read.apply(event); }
        assert!(read.learned);
    }

    fn extra() -> plx_data::metadata::Extra {
        plx_data::metadata::Extra {
            rk: "99".into(),
            part: "/library/parts/trailer".into(),
            vcodec: "h264".into(),
            acodec: "aac".into(),
            title: "Official Trailer".into(),
            subtype: "trailer".into(),
            extra_type: 1,
            dur_ms: 120_000,
            bitrate: 2_500,
            thumb: String::new(),
        }
    }

    #[test]
    fn a_movie_menu_without_a_trailer_keeps_today_s_labels() {
        let menu = build(&item(0, PosterMark::None), false);
        assert_eq!(
            labels(&menu),
            ["Go to Movie", "—", "Mark as Watched", "Play from Start"]
        );
        assert!(!menu.offers(ItemRow::PlayTrailer));
    }

    #[test]
    fn a_movie_menu_with_a_trailer_offers_play_trailer_for_the_extra() {
        let extra = extra();
        let movie = item(0, PosterMark::None);
        let menu = build_with(&movie, false, Some(&extra));
        assert!(labels(&menu).contains(&"Play Trailer".to_string()));
        match menu.action(ItemRow::PlayTrailer) {
            Action::PlayTrailer { rk, part, .. } => {
                assert_eq!(rk, "99");
                assert_eq!(part, "/library/parts/trailer");
                assert_ne!(rk.as_str(), movie.rk.as_str());
            }
            other => panic!("expected PlayTrailer, got {other:?}"),
        }
    }

    #[test]
    fn a_show_menu_with_a_trailer_offers_play_trailer() {
        let menu = build_with(&item(1, PosterMark::None), false, Some(&extra()));
        assert!(labels(&menu).contains(&"Play Trailer".to_string()));
    }

    #[test]
    fn episode_and_season_menus_never_offer_play_trailer() {
        for kind in [2, 3] {
            let menu = build_with(&item(kind, PosterMark::None), false, Some(&extra()));
            assert!(
                !labels(&menu).contains(&"Play Trailer".to_string()),
                "kind {kind}"
            );
            assert!(!menu.offers(ItemRow::PlayTrailer));
        }
    }

    #[test]
    fn play_trailer_is_cache_only_on_the_loaded_detail() {
        let _g = plx_base::testlock::serial();
        plx_data::metadata::set_current_for_test(test_store().state_mut(), None);
        assert!(
            cached_trailer(plx_plex::plex::ServerId::UNSET, &item(0, PosterMark::None), test_store().view()).is_none(),
            "no loaded Detail → no row"
        );

        plx_data::metadata::set_current_for_test(test_store().state_mut(), Some(plx_data::metadata::Detail {
            sid: plx_plex::plex::ServerId::UNSET,
            rk: "42".into(),
            kind: "movie".into(),
            extras: vec![extra()],
            ..Default::default()
        }));
        let hit = cached_trailer(plx_plex::plex::ServerId::UNSET, &item(0, PosterMark::None), test_store().view()).unwrap();
        assert_eq!(hit.rk, "99");

        let mut other = item(0, PosterMark::None);
        other.rk = "other".into();
        assert!(
            cached_trailer(plx_plex::plex::ServerId::UNSET, &other, test_store().view()).is_none(),
            "a related tile of a different item must not steal the loaded trailer"
        );
        plx_data::metadata::set_current_for_test(test_store().state_mut(), None);
    }

    #[test]
    fn play_trailer_falls_back_to_the_card_title() {
        let mut extra = extra();
        extra.title.clear();
        let mut movie = item(0, PosterMark::None);
        movie.title = "The Movie".into();
        let menu = build_with(&movie, false, Some(&extra));
        match menu.action(ItemRow::PlayTrailer) {
            Action::PlayTrailer { title, rk, part, .. } => {
                assert_eq!(title, "The Movie");
                assert_eq!(rk, "99");
                assert_eq!(part, "/library/parts/trailer");
            }
            other => panic!("expected PlayTrailer, got {other:?}"),
        }
    }

    #[test]
    fn play_trailer_action_carries_the_extra_identity() {
        let act = Action::PlayTrailer {
            rk: "9".into(),
            part: "/p".into(),
            vcodec: "h264".into(),
            acodec: "aac".into(),
            title: "T".into(),
        };
        assert_eq!(act.rk(), "9");
        match act {
            Action::PlayTrailer { part, title, .. } => {
                assert_eq!(part, "/p");
                assert_eq!(title, "T");
            }
            _ => unreachable!(),
        }
    }

    /// **Every action row of every item menu fits its panel, in every shipped language.** The
    /// panel grows to its content (the menu's `menu_panel_width` clamps `measured_width` between [`MENU_MIN_W`] and
    /// [`MENU_MAX_W`]), so what this guards is the CEILING: no verb may be longer than the widest
    /// panel the screen will ever draw. Built through the real [`build_with`], [`build_episode`]
    /// and [`build_season`] over every item kind, watch state, Continue Watching and trailer
    /// combination, in the same compact table `build_rows` seats, measured with the device's
    /// whole-pixel advances. No row carries server text; the parent title only rides an action.
    #[test]
    fn every_action_row_fits_the_widest_panel_in_every_language() {
        use plx_os::i18n::{language_on_this_thread_for_test, SHIPPED};
        // The panel hugs its content, so it always fits itself; the widest it may ever grow to
        // ([`MENU_MAX_W`], the shared cap) is the width a verb can actually be held to.
        let widest = MENU_MAX_W;
        let measure = plx_base::fontcov::advances::ShippedMeasure;
        let marks = [PosterMark::None, PosterMark::InProgress, PosterMark::Watched];
        let trailer = plx_data::metadata::Extra { rk: "9".into(), part: "/p".into(), ..Default::default() };
        // A row captures its text when it is BUILT, so the menus are rebuilt inside each language.
        let all_menus = || {
            let mut menus: Vec<(String, Built)> = Vec::new();
            menus.push(("filmography watchlist".into(), built(build_watchlist_add([0xab; 12]))));
            for kind in 0..=3 {
                for mark in marks {
                    for from_deck in [false, true] {
                        for with_trailer in [false, true] {
                            let t = with_trailer.then_some(&trailer);
                            menus.push((format!("card kind={kind} {mark:?} deck={from_deck} trailer={with_trailer}"),
                                build_with(&item(kind, mark), from_deck, t)));
                        }
                    }
                }
            }
            for mark in marks {
                menus.push((format!("episode {mark:?}"), build_episode("1", mark)));
                menus.push((format!("season {mark:?}"), build_season("1", mark)));
            }
            menus
        };
        let mut out = Vec::new();
        for language in SHIPPED {
            let _guard = language_on_this_thread_for_test(language);
            for (name, menu) in all_menus() {
                let table = &menu.0.table;
                out.extend(table.menu_cap_failure(&measure, &format!("{} {name}", language.tag())));
                out.extend(table.app_fit_failures(widest, &format!("{} {name}", language.tag())));
                out.extend(table.app_fit_failures_hugged(&format!("{} {name}", language.tag())));
            }
        }
        plx_ui::table::assert_no_fit_failures(&out);
    }

    fn sample_versions() -> Vec<MediaVersion> {
        plx_appkit::version_list::sample()
    }

    fn versioned(kind: c_int) -> PmsMovie {
        PmsMovie { versions: sample_versions(), ..item(kind, PosterMark::None) }
    }

    /// The row exists exactly when there is a choice: a movie or episode with two or more
    /// versions, on a card and on the filmstrip. One version, or a show or season (which plays a
    /// leaf it has not chosen yet), offers none.
    #[test]
    fn play_version_is_offered_only_for_a_leaf_with_a_choice() {
        for kind in [0, 3] {
            assert!(!build(&item(kind, PosterMark::None), false).offers(ItemRow::PlayVersion), "kind {kind}");
            let one = PmsMovie { versions: sample_versions()[..1].to_vec(), ..item(kind, PosterMark::None) };
            assert!(!build(&one, false).offers(ItemRow::PlayVersion), "kind {kind}");
            assert!(build(&versioned(kind), false).offers(ItemRow::PlayVersion), "kind {kind}");
            assert!(build(&versioned(kind), true).offers(ItemRow::PlayVersion), "kind {kind} on the deck");
        }
        for kind in [1, 2] {
            let container = PmsMovie { versions: sample_versions(), ..item(kind, PosterMark::None) };
            assert!(!build(&container, false).offers(ItemRow::PlayVersion), "kind {kind}");
        }
        assert!(!built(super::build_episode("1", PosterMark::None, &[], None)).offers(ItemRow::PlayVersion));
        assert!(built(super::build_episode("1", PosterMark::None, &sample_versions(), None)).offers(ItemRow::PlayVersion));
    }

    /// Select Version… does not leave the panel: it becomes the version list, and a version row
    /// reports a SELECTION of THAT version's id for the row the menu captured.
    #[test]
    fn select_version_lists_the_versions_and_a_row_selects_its_id() {
        let m = versioned(3);
        let mut s = ItemMenuScreen::new(EntryId(7), card_arg(&m, false));
        s.built = true;
        s.set_root(build_card(&m, false, DeckPress::Play, None, None, None));
        let choose = first_action(&s, |a| matches!(a, Action::ChooseVersion(_)));
        let mut out = Vec::new();
        let mut present = plx_machine::present::Present::new();
        let mut fx = Effects::new(
            &mut out,
            plx_machine::machine::MachineId::Instance(plx_machine::machine::InstanceId(8)),
            &mut present,
        );
        activate(&mut s, choose, &mut fx);
        assert!(out.is_empty(), "the menu turns itself into the list: no request, no dismiss");
        let rows: Vec<Action> =
            (0..s.form().table.n_rows() as usize).filter_map(|i| s.form().binding_at(i).map(|b| b.action.clone())).collect();
        assert_eq!(
            rows,
            [
                Action::SelectVersion { rk: "42".into(), media: plx_plex::plex::MediaId(3082) },
                Action::SelectVersion { rk: "42".into(), media: plx_plex::plex::MediaId(3086) },
            ],
            "one row per version, in the server's order",
        );
        let checked: Vec<bool> = s.form().table.sections.iter().flat_map(|sec| sec.rows.iter()).filter(|r| !r.is_note()).map(|r| r.checked).collect();
        assert_eq!(checked.iter().filter(|c| **c).count(), 1, "exactly one row wears the check: the file Play takes now");
        let marked = checked.iter().position(|c| *c).expect("a marked row") as u16;
        assert_eq!(s.opening(), ItemRow::Version(marked).focus_key(), "focus reseats on the marked row");
        let second = first_action(&s, |a| matches!(a, Action::SelectVersion { media, .. } if media.0 == 3086));
        let req = commit(&mut s, second);
        assert_eq!(req.act, Action::SelectVersion { rk: "42".into(), media: plx_plex::plex::MediaId(3086) });
        assert_eq!(req.item.map(|r| r.rk), Some("42".into()), "the captured row rides with it");
    }

    /// **The Watchlist hold menu** is `hold::rows_for` bound to actions: the go row
    /// names the matched rk or the guid, Remove names the guid, and only Remove is destructive.
    #[test]
    fn the_catalog_menu_binds_the_hold_rows() {
        use plx_data::watchlist::model::TileState;
        for state in [
            TileState::Available,
            TileState::ComingSoon,
            TileState::ComingSoonMatched,
            TileState::NotOnAnyServer,
            TileState::ServerOffline,
            TileState::Unknown,
        ] {
            let menu = built(super::build_catalog(state, plx_data::watchlist::model::Kind::Movie, "42", "plex://movie/abc", plx_data::catalog::Membership::On));
            let go_detail = matches!(state, TileState::Available | TileState::ComingSoonMatched);
            assert_eq!(menu.ids(), vec![
                if go_detail { ItemRow::GoToItem } else { ItemRow::GoToCatalog },
                ItemRow::WatchlistRemove,
            ], "{state:?}");
            assert_eq!(
                menu.action(if go_detail { ItemRow::GoToItem } else { ItemRow::GoToCatalog }),
                if go_detail { Action::GoToItem("42".into()) } else { Action::GoToCatalog("plex://movie/abc".into()) },
            );
            assert_eq!(menu.action(ItemRow::WatchlistRemove), Action::WatchlistRemove("plex://movie/abc".into()));
            assert!(menu.is_destructive(ItemRow::WatchlistRemove));
            assert!(!menu.is_destructive(ItemRow::GoToItem) && !menu.is_destructive(ItemRow::GoToCatalog));
        }
    }

    /// **A playable Watchlist title's hold menu is the PMS card menu plus Remove** (plan §1.9): the
    /// card's own rows (Go to, Mark watched, Play from Start) and the destructive Remove last.
    #[test]
    fn a_playable_catalog_hold_menu_is_the_card_menu_plus_remove() {
        let movie = item(0, PosterMark::None);
        let menu = built(super::build_card(&movie, false, DeckPress::Play, None, Some("plex://movie/abc"), None));
        let ids = menu.ids();
        for row in [ItemRow::GoToItem, ItemRow::MarkWatched, ItemRow::PlayFromStart] {
            assert!(ids.contains(&row), "the card's {row:?} row is there: {ids:?}");
        }
        assert_eq!(ids.last(), Some(&ItemRow::WatchlistRemove), "Remove is last");
        assert_eq!(menu.action(ItemRow::WatchlistRemove), Action::WatchlistRemove("plex://movie/abc".into()));
        assert!(menu.is_destructive(ItemRow::WatchlistRemove));
        let plain = built(super::build_with(&movie, false, DeckPress::Play, None));
        assert!(!plain.offers(ItemRow::WatchlistRemove), "a plain card menu has no Remove");
    }

    /// The go row of a show's hold menu reads "Go to Show" where a movie's reads "Go to Movie".
    #[test]
    fn the_catalog_menus_go_row_names_the_kind() {
        use plx_data::watchlist::model::{Kind, TileState};
        let _guard = plx_os::i18n::language_on_this_thread_for_test(plx_os::i18n::Preference::En);
        let label = |kind| {
            let menu = built(super::build_catalog(TileState::Available, kind, "42", "plex://movie/abc", plx_data::catalog::Membership::On));
            menu.0.table.sections.iter().flat_map(|s| s.rows.iter()).next().map(|r| r.label.to_string())
        };
        assert_ne!(label(Kind::Show), label(Kind::Movie));
    }

    /// The Remove row's label fits the widest panel in every shipped language.
    #[test]
    fn the_catalog_menu_fits_in_every_language() {
        use plx_data::watchlist::model::TileState;
        use plx_os::i18n::{language_on_this_thread_for_test, SHIPPED};
        let measure = plx_base::fontcov::advances::ShippedMeasure;
        let mut out = Vec::new();
        for language in SHIPPED {
            let _guard = language_on_this_thread_for_test(language);
            for state in [TileState::Available, TileState::ComingSoon] {
                let menu = built(super::build_catalog(state, plx_data::watchlist::model::Kind::Movie, "1", "g", plx_data::catalog::Membership::On));
                let name = format!("{} catalog {state:?}", language.tag());
                let table = &menu.0.table;
                out.extend(table.menu_cap_failure(&measure, &name));
                out.extend(table.app_fit_failures(MENU_MAX_W, &name));
                out.extend(table.app_fit_failures_hugged(&name));
            }
        }
        plx_ui::table::assert_no_fit_failures(&out);
    }

    /// A leaf with three versions, its card menu built (so Play Version is a row).
    fn leaf_menu_with_versions() -> ItemMenuScreen {
        let m = versioned(3);
        let mut s = ItemMenuScreen::new(EntryId(7), card_arg(&m, false));
        s.built = true;
        s.set_root(build_card(&m, false, DeckPress::Play, None, None, None));
        s
    }

    /// Press Play Version: the menu's second page, one row per version.
    fn open_play_version_page(s: &mut ItemMenuScreen) {
        let choose = first_action(s, |a| matches!(a, Action::ChooseVersion(_)));
        let mut out = Vec::new();
        let mut present = plx_machine::present::Present::new();
        let mut fx = Effects::new(
            &mut out,
            plx_machine::machine::MachineId::Instance(plx_machine::machine::InstanceId(8)),
            &mut present,
        );
        activate(s, choose, &mut fx);
    }

    /// The item menu on a leaf with versions, both pages (the rows, then Play Version's), run
    /// through the id conformance (`AppArg::ItemMenu`; the coverage table calls this).
    pub(crate) fn item_menu_conforms() -> plx_ui::a11y::Namespaces {
        let mut s = leaf_menu_with_versions();
        let mut namespaces = vec![with_cx(|cx| plx_ui::a11y::conform::<HostFixture, _>(&s, cx))];
        open_play_version_page(&mut s);
        namespaces.push(with_cx(|cx| plx_ui::a11y::conform::<HostFixture, _>(&s, cx)));
        namespaces
    }

    /// **The accessibility ids:** every row a menu can show is named by its `ItemRow` (the menu is
    /// `itemmenu.<row>`), the Play Version page's rows by position (`itemmenu.version.<n>`), and
    /// the whole view conforms: every row projects, the ids are unique, and each id finds its row.
    #[test]
    fn rows_are_named_by_their_item_row_and_conform() {
        use plx_ui::a11y::A11yPath;
        use plx_ui::a11y::conform;
        let mut s = leaf_menu_with_versions();
        with_cx(|cx| {
            conform::<HostFixture, _>(&s, cx);
            let play = A11yPath::parse_relative("play_version").expect("a plain row name");
            let key = Focusable::<HostFixture>::a11y_find(&s, &play, cx).expect("a leaf with two versions offers Play Version");
            assert_eq!(key, ItemRow::PlayVersion.focus_key());
            let a = Focusable::<HostFixture>::a11y(&s, &key, cx).unwrap();
            assert_eq!(a.role, plx_ui::a11y::Role::Row);
            assert!(a.label.is_some_and(|l| !l.is_empty()), "the row's drawn words");
        });

        open_play_version_page(&mut s);
        with_cx(|cx| {
            conform::<HostFixture, _>(&s, cx);
            for n in 0..2u16 {
                let id = A11yPath::parse_relative(&format!("version.{n}")).unwrap();
                let key = Focusable::<HostFixture>::a11y_find(&s, &id, cx).expect("one row per version");
                assert_eq!(key, ItemRow::Version(n).focus_key());
            }
        });
    }

    /// Every row's `(id, subject)` of the menu as it stands, in listing order.
    fn row_subjects(s: &ItemMenuScreen) -> Vec<(String, Option<String>)> {
        with_cx(|cx| {
            let mut keys = Vec::new();
            Focusable::<HostFixture>::a11y_elems(s, GroupId(0), cx, &mut keys);
            keys.iter()
                .map(|key| {
                    let a = Focusable::<HostFixture>::a11y(s, key, cx).expect("a bound row projects");
                    (a.path.to_string(), a.subject.map(|c| c.into_owned()))
                })
                .collect()
        })
    }

    /// **A row names the title it acts on.** Play, Mark watched, Remove from Deck and the Watchlist rows
    /// all act on the item the menu was opened on, so a harness press (`press:itemmenu.<row>@<rk>`) has
    /// something to compare: every row carries the menu's own ratingKey as its subject, on every kind of
    /// menu and on the Play Version page too, and a Catalog menu (no ratingKey: its title is a Watchlist
    /// guid no server holds) carries none rather than a made-up one.
    #[test]
    fn every_row_names_the_menus_item_as_its_subject() {
        let mut card = leaf_menu_with_versions();
        let rows = row_subjects(&card);
        assert!(rows.len() >= 3, "the menu has rows: {rows:?}");
        for (id, subject) in &rows {
            assert_eq!(subject.as_deref(), Some("42"), "{id}");
        }
        open_play_version_page(&mut card);
        let versions = row_subjects(&card);
        assert!(versions.iter().any(|(id, _)| id == "version.0"), "{versions:?}");
        for (id, subject) in &versions {
            assert_eq!(subject.as_deref(), Some("42"), "{id}");
        }

        let arg = |rk: &str, kind: ItemMenuKind| ItemMenuArg {
            sid: plx_plex::plex::ServerId::from_raw(3),
            rk: rk.into(),
            kind,
            host: EntryId(1),
            focus: None,
            anchor: [0; 4],
            loaded_episode: false,
            from_home: false,
            catalog: None,
            watchlist: None,
        };
        let mut strip = ItemMenuScreen::new(
            EntryId(8),
            arg("77", ItemMenuKind::Episode { mark: PosterMark::None, versions: Vec::new() }),
        );
        let mut tab = ItemMenuScreen::new(
            EntryId(8),
            arg("season-9", ItemMenuKind::Season { mark: PosterMark::None, show_rk: "show-1".into(), index: 4 }),
        );
        for (menu, want) in [(&mut strip, "77"), (&mut tab, "season-9")] {
            menu.build_rows(test_store().view(), DeckPress::Play);
            let rows = row_subjects(menu);
            assert!(!rows.is_empty());
            for (id, subject) in &rows {
                assert_eq!(subject.as_deref(), Some(want), "{id}");
            }
        }

        let mut catalog = ItemMenuScreen::new(
            EntryId(8),
            arg("", ItemMenuKind::Catalog { state: plx_data::watchlist::model::TileState::NotOnAnyServer }),
        );
        catalog.build_rows(test_store().view(), DeckPress::Play);
        let rows = row_subjects(&catalog);
        assert!(!rows.is_empty());
        for (id, subject) in &rows {
            assert_eq!(*subject, None, "{id}: a Catalog menu has no ratingKey to name");
        }
    }

    // ---- the hero control's morph ------------------------------------------------------------

    /// The Detail hero's `…` menu, built over a `…` button at `(600, 760)`.
    fn hero_menu() -> ItemMenuScreen {
        let rows = vec![HeroCtl::GoToShow, HeroCtl::Restart, HeroCtl::Version, HeroCtl::AudioSubtitles];
        let mut arg = card_arg(&item(3, PosterMark::None), false);
        arg.anchor = [600.0f32, 760.0, 64.0, 64.0].map(f32::to_bits);
        arg.kind = ItemMenuKind::HeroOverflow { rows, opener: plx_machine::machine::InstanceId(4), page: arg.rk.clone() };
        let mut menu = ItemMenuScreen::new(EntryId(7), arg);
        menu.build_rows(test_store().view(), DeckPress::Play);
        menu
    }

    /// The glass the surface drew this frame: the panel ground (a `rect_rimmed`, tag 8, on a host with
    /// no frozen field).
    fn glass(menu: &mut ItemMenuScreen) -> Rect {
        with_cx(|cx| {
            plx_ui::draw_census::capture(|| {
                let mut f = DrawFrame::new(cx, plx_ui::Painter::recording());
                Screen::<HostFixture>::draw(menu, &mut f);
            })
        })
        .into_iter()
        .find_map(|(tag, r)| (tag == 8).then_some(r))
        .unwrap_or_default()
    }

    fn tick(menu: &mut ItemMenuScreen, ms: &mut u32, n: usize) {
        for _ in 0..n {
            *ms += 16;
            step_ev(menu, &ScreenEvent::Tick(Tick { ms: *ms, dt_us: 16_667 }));
        }
    }

    /// **The hero's `…` menu GROWS out of the button** (`Popover::morphing` + `open_from`, the path
    /// every header menu takes), and BACK (any dismissal) runs the morph home. Red while the panel
    /// was laid beside the button at its final rect on its first frame and only appeared: three
    /// frames in, the glass is still the button's size or a little over; the settled panel is exactly
    /// `frame()`, and it COVERS the button (the opener is consumed: the lift draws it at the morph's
    /// `opener_alpha`, gone while the glass stands, whole again once the droplet has drained into it).
    #[test]
    fn the_hero_menu_grows_out_of_its_button_settles_on_its_frame_and_runs_home_on_back() {
        let _guard = plx_base::testlock::serial();
        let mut menu = hero_menu();
        step_ev(&mut menu, &ScreenEvent::Mount);
        let settled = menu.frame(&FixtureMeasure);
        let pill = plx_ui::detail_layout::hero_pill(menu.arg.anchor);
        assert!(
            settled.x <= pill.x && settled.y <= pill.y && settled.x + settled.w >= pill.x + pill.w && settled.y + settled.h >= pill.y + pill.h,
            "the settled panel {settled:?} covers the button it grew out of {pill:?}"
        );
        assert_eq!(Screen::<HostFixture>::opener_alpha(&menu), 1.0, "frame 0 is the button whole");
        assert!(Screen::<HostFixture>::scrim(&menu).ramp.is_some(), "the dim follows the morph");
        let mut ms = 0;
        tick(&mut menu, &mut ms, 3);
        let early = glass(&mut menu);
        assert!(
            early.w < settled.w - 20.0 && early.h < settled.h - 20.0,
            "three frames in the glass is still growing out of the button ({early:?}), not laid at {settled:?}"
        );
        tick(&mut menu, &mut ms, 6);
        assert!(Screen::<HostFixture>::opener_alpha(&menu) < 0.5, "the button is consumed as the glass detaches from it");
        tick(&mut menu, &mut ms, 90);
        let end = glass(&mut menu);
        assert!(
            (end.x - settled.x).abs() < 0.5 && (end.y - settled.y).abs() < 0.5
                && (end.w - settled.w).abs() < 0.5 && (end.h - settled.h).abs() < 0.5,
            "the settled panel is the frame {settled:?}, drew {end:?}"
        );
        assert_eq!(Screen::<HostFixture>::opener_alpha(&menu), 0.0, "the button stays consumed while the menu stands");
        assert!(!Screen::<HostFixture>::handed_back(&menu));
        // BACK: the dispatcher delivers `Closing`; the glass drains back into the button
        step_ev(&mut menu, &ScreenEvent::Closing);
        tick(&mut menu, &mut ms, 3);
        let closing = glass(&mut menu);
        assert!(closing.w < settled.w - 20.0, "three frames into the close the glass is narrowing into the button ({closing:?})");
        assert!(!Screen::<HostFixture>::handed_back(&menu), "the droplet is still draining");
        tick(&mut menu, &mut ms, 60);
        assert!(Screen::<HostFixture>::handed_back(&menu), "the droplet has drained into the button: the surface retires");
        assert_eq!(Screen::<HostFixture>::opener_alpha(&menu), 1.0, "the button is whole again");
    }

    /// **A pointer held down waits while the card moves or the pages slide** (`Screen::pointer_held`, as
    /// the standalone panel's does): idle at the rows, held while a pushed page slides in, idle again once
    /// it has settled. Red while the menu answered `false` throughout.
    #[test]
    fn a_pointer_waits_while_a_page_slides() {
        let _guard = plx_base::testlock::serial();
        let mut menu = hero_menu_with_pages();
        step_ev(&mut menu, &ScreenEvent::Mount);
        let mut ms = 0;
        tick(&mut menu, &mut ms, 90);
        assert!(!Screen::<HostFixture>::pointer_held(&menu), "the open menu at rest takes pointers");
        press_row(&mut menu, ItemRow::Hero(HeroCtl::Version));
        assert!(Screen::<HostFixture>::pointer_held(&menu), "a page is sliding in");
        tick(&mut menu, &mut ms, 90);
        assert!(!Screen::<HostFixture>::pointer_held(&menu), "the page has settled");
        press_back(&mut menu);
        assert!(Screen::<HostFixture>::pointer_held(&menu), "the page slides back out");
    }

    /// **A page pushed into a menu takes the menu's size class** (`TableView::compact`, one per glass):
    /// Select Version and Audio & Subtitles, in a hero `…` menu and in a tile's menu, lay out BODY regular
    /// like the rows beneath them, on the way in and again after BACK. Red while a page was built full
    /// size (HEADLINE bold).
    #[test]
    fn pages_pushed_into_a_menu_lay_out_body_regular() {
        let _guard = plx_base::testlock::serial();
        let body_regular = (plx_ui::theme::size::BODY, false);
        let mut hero = hero_menu_with_pages();
        assert_eq!(hero.form().table.label_style(), body_regular, "the rows");
        for ctl in [HeroCtl::Version, HeroCtl::AudioSubtitles] {
            press_row(&mut hero, ItemRow::Hero(ctl));
            assert_eq!(depth(&hero), 1);
            assert_eq!(hero.form().table.label_style(), body_regular, "{ctl:?} page of the hero menu");
            if ctl == HeroCtl::AudioSubtitles {
                press_row(&mut hero, ItemRow::Pick(PickRow::OpenAudio));
                assert_eq!(hero.form().table.label_style(), body_regular, "the Audio page, one deeper");
                press_back(&mut hero);
            }
            press_back(&mut hero);
            assert_eq!(hero.form().table.label_style(), body_regular, "back on the rows");
        }
        let m = versioned(3);
        let mut tile = ItemMenuScreen::new(EntryId(7), card_arg(&m, false));
        tile.build_rows(test_store().view(), DeckPress::Play);
        press_row(&mut tile, ItemRow::PlayVersion);
        assert_eq!(depth(&tile), 1);
        assert_eq!(tile.form().table.label_style(), body_regular, "Select Version in a tile's menu");
        press_back(&mut tile);
        assert_eq!(tile.form().table.label_style(), body_regular, "the tile's rows");
    }

    /// **A page with nothing behind it is not pushed**: a hero menu whose page offers no tracks hands
    /// Audio & Subtitles back to the control's own path (the panel), as a page with no versions does Select
    /// Version, never an empty list of rows with nothing to pick.
    #[test]
    fn a_page_with_nothing_to_pick_is_not_pushed() {
        let _guard = plx_base::testlock::serial();
        let mut menu = hero_menu();
        let hero = menu.hero.as_mut().expect("a hero menu");
        assert!(hero.versions.is_empty() && hero.tracks.is_empty(), "the page offered nothing");
        assert_eq!(source!(menu).page_for(HeroCtl::Version), None);
        assert_eq!(source!(menu).page_for(HeroCtl::AudioSubtitles), None);
        let out = press_row(&mut menu, ItemRow::Hero(HeroCtl::AudioSubtitles));
        assert_eq!(depth(&menu), 0, "no page was pushed");
        assert!(dismisses(&out), "the control's own path: the menu closes and the page opens its panel");
    }

    /// **A tile's Select Version is a page of the same stack**: OK pushes it, BACK returns to the tile's
    /// rows with focus on the row that opened it (it closed the menu before), and a version row still
    /// reports its selection.
    #[test]
    fn a_tiles_select_version_is_a_page_and_back_returns_to_its_rows() {
        let _guard = plx_base::testlock::serial();
        let m = versioned(3);
        let mut menu = ItemMenuScreen::new(EntryId(7), card_arg(&m, false));
        menu.build_rows(test_store().view(), DeckPress::Play);
        let rows: Vec<Option<Action>> = (0..menu.form().table.n_rows() as usize).map(|i| menu.form().binding_at(i).map(|b| b.action.clone())).collect();
        let out = press_row(&mut menu, ItemRow::PlayVersion);
        assert!(out.is_empty(), "the menu turns itself into the list: no request, no dismiss");
        assert_eq!(depth(&menu), 1);
        assert!(Screen::<HostFixture>::pointer_held(&menu), "the page slides in");
        let out = press_back(&mut menu);
        assert!(!dismisses(&out), "BACK on the page returns to the rows");
        assert_eq!(depth(&menu), 0);
        assert_eq!(selected(&menu), Some(ItemRow::PlayVersion.key().0), "the ring is on the row that opened the page");
        let again: Vec<Option<Action>> = (0..menu.form().table.n_rows() as usize).map(|i| menu.form().binding_at(i).map(|b| b.action.clone())).collect();
        assert_eq!(again, rows, "the rows are the ones the menu opened with");
        assert!(dismisses(&press_back(&mut menu)), "BACK at the rows closes");
    }

    /// The `…` menu with the page's data loaded as `build_rows` reads it from the metadata store: three
    /// versions (the first the one Play takes) and a two-audio, one-subtitle item.
    fn hero_menu_with_pages() -> ItemMenuScreen {
        let mut menu = hero_menu();
        let stream = |id: i64, lang: &str, code: &str| plx_data::metadata::Stream {
            id,
            lang: lang.into(),
            lang_code: code.into(),
            ..plx_data::metadata::Stream::default()
        };
        let versions = sample_versions();
        let taken = versions.first().map(|v| v.id);
        let hero = menu.hero.as_mut().expect("a hero menu");
        hero.versions = versions;
        hero.taken = taken;
        hero.tracks = PickerCore::new(plx_data::metadata::PreplayTracks {
            sid: plx_plex::plex::ServerId::from_raw(1),
            rk: "42".into(),
            kind: "movie".into(),
            show_rk: String::new(),
            part: "/library/parts/9001/1/file.mkv".into(),
            audio: vec![stream(101, "English", "eng"), stream(102, "French", "fra")],
            subs: vec![stream(201, "English", "eng")],
            pick: None,
        });
        // the page content arrived after the mount built the rows: build them again, as the mount would have
        step!(menu, Event::Refresh { keep: None });
        menu
    }

    /// **Other languages is a drill-in in this menu too**, and its pages keep the aliases the shared builder
    /// declares: a language that shrinks to one track under the viewer (the chosen track is a sidecar the app
    /// cannot draw, so it is listed while chosen) is now a direct row, and the pop out of its page lands on it.
    #[test]
    fn other_languages_drill_in_and_a_shrunk_language_keeps_its_pop_alias() {
        let mut menu = hero_menu_with_pages();
        let stream = |id: i64, lang: &str, code: &str| plx_data::metadata::Stream {
            id,
            lang: lang.into(),
            lang_code: code.into(),
            ..plx_data::metadata::Stream::default()
        };
        let mut pgs = stream(303, "French", "fra");
        pgs.external = true;
        pgs.codec = "pgs".into();
        menu.hero.as_mut().expect("a hero menu").tracks = PickerCore::new(plx_data::metadata::PreplayTracks {
            sid: plx_plex::plex::ServerId::from_raw(1),
            rk: "42".into(),
            kind: "movie".into(),
            show_rk: String::new(),
            part: "/library/parts/9001/1/file.mkv".into(),
            audio: vec![stream(101, "English", "eng")],
            subs: vec![stream(201, "English", "eng"), stream(302, "French", "fra"), pgs],
            pick: None,
        });
        let other = |menu: &ItemMenuScreen| source!(menu).pick_page(PickPage::OtherLanguages).expect("the page").form;
        fn hero(menu: &mut ItemMenuScreen) -> &mut PickerCore {
            &mut menu.hero.as_mut().expect("a hero menu").tracks
        }
        hero(&mut menu).select(Pick::Subtitle(Some(2))).expect("the sidecar is chosen");
        let open = other(&menu)
            .slots()
            .iter()
            .flatten()
            .map(|b| b.id)
            .find(|id| matches!(id, ItemRow::Pick(PickRow::OpenLang(_))))
            .expect("French holds two tracks while the sidecar is chosen: a drill-in");
        assert_eq!(source!(menu).pick_page(PickPage::Subtitles).expect("the page").form.slots().iter().flatten().count(), 3, "Off, English, Other languages");
        hero(&mut menu).select(Pick::Subtitle(Some(1))).expect("the embedded track picks");
        assert_eq!(other(&menu).stands_for(&open), ItemRow::Pick(PickRow::Sub(1)), "the pop lands on the language's direct row");
    }

    /// Run `f` against a fresh effect sink and answer what it emitted.
    fn emitted(f: impl FnOnce(&mut Effects<'_, HostFixture>)) -> Vec<plx_machine::machine::Stamped<HostFixture>> {
        let mut out = Vec::new();
        let mut present = plx_machine::present::Present::new();
        let mut fx = Effects::new(&mut out, MachineId::Instance(plx_machine::machine::InstanceId(8)), &mut present);
        f(&mut fx);
        drop(fx);
        out
    }

    fn press_row(menu: &mut ItemMenuScreen, row: ItemRow) -> Vec<plx_machine::machine::Stamped<HostFixture>> {
        let elem = row.key().0;
        assert!(menu.form().index_of_key(RowKey(elem)).is_some(), "{row:?} is on the page showing");
        emitted(|fx| activate(menu, elem, fx))
    }

    fn press_back(menu: &mut ItemMenuScreen) -> Vec<plx_machine::machine::Stamped<HostFixture>> {
        emitted(|fx| {
            let out = step!(menu, Event::Back);
            menu.run::<HostFixture>(out, None, fx)
        })
    }

    fn dismisses(out: &[plx_machine::machine::Stamped<HostFixture>]) -> bool {
        out.iter().any(|e| matches!(e.fx, Fx::Nav(NavOp::Dismiss(EntryId(7)))))
    }

    fn depth(menu: &ItemMenuScreen) -> usize {
        menu.menu.depth()
    }

    fn selected(menu: &ItemMenuScreen) -> Option<u32> {
        menu.form().selected_key().map(|k| k.0)
    }

    /// **A hero control with no page in the menu closes it and is handed back to the page that owns
    /// the hero** (Go to Show, Play from Start: the page's own `activate_hero`), anchored on the rect
    /// the menu hung off; nothing reaches the app runner. (Select Version and Audio & Subtitles have
    /// pages here: `the_pages_push_...`.)
    #[test]
    fn every_other_overflow_row_closes_the_menu_and_hands_its_control_back_to_the_page() {
        let anchor = [600.0f32, 760.0, 64.0, 64.0].map(f32::to_bits);
        for want in [HeroCtl::GoToShow, HeroCtl::Restart] {
            let mut menu = hero_menu_with_pages();
            let out = press_row(&mut menu, ItemRow::Hero(want));
            assert!(dismisses(&out), "{want:?}: the menu closes");
            assert!(
                out.iter().any(|e| matches!(
                    &e.fx,
                    Fx::Deliver(
                        MachineId::Instance(plx_machine::machine::InstanceId(4)),
                        Delivery::Screen(ScreenEvent::App(AppMsg::HeroCtl { ctl, anchor: got })),
                    ) if *ctl == want && *got == anchor
                )),
                "{want:?}: the opening page is handed the control on the menu's anchor"
            );
            assert!(!out.iter().any(|e| matches!(e.fx, Fx::App(_))), "nothing reaches the app runner");
            assert_eq!(depth(&menu), 0, "{want:?}: no page");
        }
        // with nothing to list, Select Version has no page here either: the page's own panel is the way
        let mut bare = hero_menu();
        let out = press_row(&mut bare, ItemRow::Hero(HeroCtl::Version));
        assert!(dismisses(&out) && depth(&bare) == 0, "no versions: the control is handed back");
    }

    /// **OK on Select Version / Audio & Subtitles PUSHES the page into the same glass; BACK returns to
    /// the menu's rows with focus on the row that opened it; BACK again drains the menu.** The A&S
    /// pages push further (Subtitles over its root) and each BACK pops one.
    #[test]
    fn the_pages_push_into_the_same_glass_and_back_returns_to_the_row_that_opened_them() {
        let _guard = plx_base::testlock::serial();
        let mut menu = hero_menu_with_pages();
        step_ev(&mut menu, &ScreenEvent::Mount);
        let root_rows = menu.form().table.n_rows();
        let (v, a) = (ItemRow::Hero(HeroCtl::Version), ItemRow::Hero(HeroCtl::AudioSubtitles));

        // Select Version: pushed, not dismissed; the page is the version list
        let out = press_row(&mut menu, v);
        assert!(!dismisses(&out) && out.is_empty(), "OK pushes a page and emits nothing");
        assert_eq!(depth(&menu), 1);
        assert_eq!(menu.menu.focusable().count(), sample_versions().len(), "one row per version");
        assert_eq!(selected(&menu), Some(ItemRow::Version(0).key().0), "it opens on the version Play takes");
        // BACK pops to the rows, focus on the row that opened the page
        let out = press_back(&mut menu);
        assert!(!dismisses(&out), "BACK pops a page before it closes the menu");
        assert_eq!((depth(&menu), menu.form().table.n_rows()), (0, root_rows));
        assert_eq!(selected(&menu), Some(v.key().0), "focus is on Select Version");

        // Audio & Subtitles: its root, then Subtitles over it: two pages in the one stack
        press_row(&mut menu, a);
        assert_eq!(depth(&menu), 1);
        assert!(menu.form().index_of_key(RowKey(ItemRow::Pick(PickRow::OpenSubtitles).key().0)).is_some(), "the root lists Audio and Subtitles");
        press_row(&mut menu, ItemRow::Pick(PickRow::OpenSubtitles));
        assert_eq!(depth(&menu), 2, "Subtitles pushes further in the same glass");
        assert_eq!(menu.form().table.title(), Some(plx_os::i18n::msg::widgets_tracks_subtitles()));
        press_back(&mut menu);
        assert_eq!((depth(&menu), selected(&menu)), (1, Some(ItemRow::Pick(PickRow::OpenSubtitles).key().0)), "back to the A&S root on Subtitles");
        press_back(&mut menu);
        assert_eq!((depth(&menu), selected(&menu)), (0, Some(a.key().0)), "back to the menu on Audio & Subtitles");
        let out = press_back(&mut menu);
        assert!(dismisses(&out), "BACK at the rows drains the menu into its button");
    }

    /// **A pick in Select Version closes the whole menu** and reports the choice to the page (which
    /// selects it, as the standalone panel's pick does); nothing else reaches the loop.
    #[test]
    fn a_version_pick_closes_the_whole_menu_and_reports_it_to_the_page() {
        let mut menu = hero_menu_with_pages();
        step_ev(&mut menu, &ScreenEvent::Mount);
        press_row(&mut menu, ItemRow::Hero(HeroCtl::Version));
        let second = sample_versions()[1].id;
        let out = press_row(&mut menu, ItemRow::Version(1));
        assert!(dismisses(&out), "the pick closes the menu from its page");
        assert!(
            out.iter().any(|e| matches!(
                &e.fx,
                Fx::Deliver(
                    MachineId::Instance(plx_machine::machine::InstanceId(4)),
                    Delivery::Screen(ScreenEvent::App(AppMsg::SelectVersion(media))),
                ) if *media == second
            )),
            "the page is told which version"
        );
        assert!(!out.iter().any(|e| matches!(e.fx, Fx::App(_))));
    }

    /// With no playback driver every version row is disabled: OK does nothing and the menu stays.
    #[test]
    fn a_version_row_is_disabled_without_a_playback_driver_and_the_menu_stays_up() {
        let mut menu = hero_menu_with_pages();
        step_ev(&mut menu, &ScreenEvent::Mount);
        menu.hero.as_mut().unwrap().can_play = false;
        press_row(&mut menu, ItemRow::Hero(HeroCtl::Version));
        assert_eq!(depth(&menu), 1);
        let out = press_row(&mut menu, ItemRow::Version(1));
        assert!(out.is_empty(), "OK on a disabled row does nothing");
    }

    /// **A track pick stays on its page with the check moved** and writes what the standalone panel
    /// writes: the store keeps the pick and the loop commits it, for a Detail page (`Source::Page`).
    #[test]
    fn a_track_pick_moves_the_check_and_commits_as_the_standalone_panel_does() {
        let mut menu = hero_menu_with_pages();
        step_ev(&mut menu, &ScreenEvent::Mount);
        press_row(&mut menu, ItemRow::Hero(HeroCtl::AudioSubtitles));
        press_row(&mut menu, ItemRow::Pick(PickRow::OpenAudio));
        assert_eq!((depth(&menu), selected(&menu)), (2, Some(ItemRow::Pick(PickRow::Audio(0)).key().0)), "Audio opens on the checked track");
        let out = press_row(&mut menu, ItemRow::Pick(PickRow::Audio(1)));
        assert!(!dismisses(&out), "a track pick keeps the menu up");
        assert_eq!(depth(&menu), 2);
        assert_eq!(menu.hero.as_ref().unwrap().tracks.now().audio, 1, "the choice is carried forward");
        assert!(out.iter().any(|e| matches!(&e.fx, Fx::App(AppFx::Store(StoreId::Metadata, _)))), "the store keeps the pick");
        assert!(
            out.iter().any(|e| matches!(&e.fx, Fx::App(AppFx::Content(ContentReq::CommitPreplayPick { source: PreplaySource::Page, .. })))),
            "and the loop commits it to the server"
        );
    }

    /// **The card is retargeted from page to page, never re-presented**: after a push the glass is
    /// still the rows' rect on the next frame and springs to the page's rect, the `…` stays consumed
    /// and the surface never hands itself back; a pop springs it back. The popover's own phase is
    /// untouched by either.
    #[test]
    fn a_page_change_retargets_the_glass_in_place_and_never_re_presents_it() {
        let _guard = plx_base::testlock::serial();
        let mut menu = hero_menu_with_pages();
        step_ev(&mut menu, &ScreenEvent::Mount);
        let mut ms = 0;
        tick(&mut menu, &mut ms, 90);
        let rows_rect = glass(&mut menu);
        assert!(rows_rect.w > 100.0, "rig: the menu stands");
        press_row(&mut menu, ItemRow::Hero(HeroCtl::Version));
        let page_rect = menu.frame(&FixtureMeasure);
        assert!(page_rect != menu.hero.as_ref().map(|_| rows_rect).unwrap(), "the page is its own size");
        assert_eq!(glass(&mut menu), rows_rect, "the push frame is still the old rect: no jump");
        for n in 1..=60 {
            tick(&mut menu, &mut ms, 1);
            let g = glass(&mut menu);
            assert!(g.w > 100.0 && g.h > 100.0, "frame {n}: there is glass");
            assert_eq!(Screen::<HostFixture>::opener_alpha(&menu), 0.0, "frame {n}: the button stays consumed");
            assert!(!Screen::<HostFixture>::handed_back(&menu), "frame {n}: the surface is never handed back");
        }
        let settled = glass(&mut menu);
        assert!((settled.x - page_rect.x).abs() < 0.5 && (settled.h - page_rect.h).abs() < 0.5 && (settled.w - page_rect.w).abs() < 0.5,
            "it settles on the page's rect ({settled:?} vs {page_rect:?})");
        press_back(&mut menu);
        tick(&mut menu, &mut ms, 90);
        let back = glass(&mut menu);
        assert!((back.h - rows_rect.h).abs() < 0.5 && (back.w - rows_rect.w).abs() < 0.5, "a pop returns the card to the rows' rect");
        // and the close from a page drains into the button from where the card is
        press_row(&mut menu, ItemRow::Hero(HeroCtl::Version));
        tick(&mut menu, &mut ms, 90);
        step_ev(&mut menu, &ScreenEvent::Closing);
        tick(&mut menu, &mut ms, 3);
        assert!(glass(&mut menu).w < menu.frame(&FixtureMeasure).w - 20.0, "draining from the page's rect");
        tick(&mut menu, &mut ms, 60);
        assert!(Screen::<HostFixture>::handed_back(&menu) && Screen::<HostFixture>::opener_alpha(&menu) == 1.0);
    }

    /// BACK as the surface receives it: pops a page when there is one, else closes.
    #[test]
    fn the_back_key_pops_a_page_before_it_closes_the_menu() {
        let _guard = plx_base::testlock::serial();
        let mut menu = hero_menu_with_pages();
        step_ev(&mut menu, &ScreenEvent::Mount);
        press_row(&mut menu, ItemRow::Hero(HeroCtl::Version));
        let _ = feed(&mut menu, Key::Back, Edge::Down, 10);
        assert_eq!(depth(&menu), 0, "the first BACK pops the page");
    }

    /// **A tile's hold menu is unchanged**: no morph (the opener is a card, not a hero control), the
    /// panel BESIDE the card at its final rect on its first frame, the dim the container's own appear
    /// (no ramp of the menu's), the card's lift whole, and nothing handed back before the container's
    /// spring says so.
    #[test]
    fn a_tile_hold_menu_keeps_the_appear_and_the_card_whole() {
        let _guard = plx_base::testlock::serial();
        let anchor = Rect::new(400.0, 300.0, 300.0, 170.0);
        let mut arg = card_arg(&item(3, PosterMark::None), true);
        arg.anchor = [anchor.x, anchor.y, anchor.w, anchor.h].map(f32::to_bits);
        let mut menu = ItemMenuScreen::new(EntryId(7), arg);
        menu.build_rows(test_store().view(), DeckPress::Play);
        step_ev(&mut menu, &ScreenEvent::Mount);
        let beside = panel_at(anchor, menu.form().table.measured_width(&FixtureMeasure), menu.form().table.measured_height());
        assert_eq!(menu.frame(&FixtureMeasure), beside, "the panel is placed beside the card, not over it");
        let mut ms = 0;
        for settle in [0, 3, 90] {
            tick(&mut menu, &mut ms, settle);
            assert_eq!(glass(&mut menu), beside, "the glass is the panel at its rect from the first frame");
            assert_eq!(Screen::<HostFixture>::opener_alpha(&menu), 1.0, "the card stays whole above the dim");
            assert!(!Screen::<HostFixture>::handed_back(&menu));
            let scrim = Screen::<HostFixture>::scrim(&menu);
            assert!(scrim.ramp.is_none(), "the dim is the container's own appear");
            assert_eq!(scrim.alpha, plx_ui::theme::underlay::DIM_COMPACT);
        }
        step_ev(&mut menu, &ScreenEvent::Closing);
        tick(&mut menu, &mut ms, 90);
        assert!(!Screen::<HostFixture>::handed_back(&menu), "a tile's close is the container's spring, never the droplet's");
    }
}
