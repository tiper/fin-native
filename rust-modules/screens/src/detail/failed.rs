//! The page of an item that would not load, for [`super::DetailScreen`].
//!
//! A Detail page asks the store for its item when it opens. When that fetch settles with nothing
//! (the server did not answer, refused it, or no longer has the item) the store says so
//! (`plx_data::metadata::DetailSlot::Failed`) and the page is no longer "loading": it is this
//! read-out, the page-placed `Failed` verdict every page shares (`StatusOverlay::page`,
//! `ui/CLAUDE.md` rule 4: the Collection page's, Home's and the Library's untyped "can't reach"
//! verdict, so their words and glyph too) with its one control, *Try again*. BACK is the way out,
//! as everywhere. The hero's row is not drawn and owns no stop in this state: Play with no item to
//! play, and the watched toggle with no item to mark, are controls acting on nothing.
//!
//! The store cannot say WHY the fetch failed (`Client::metadata` answers `None` for a transport
//! failure, a refusal, a missing item and an unparseable body alike), so there is one wording. A
//! "no longer on this server" verdict for the item's own 404 needs both a fetch that tells the two
//! apart and a new string in every language.

use plx_machine::machine::{GroupId, Measure};
use plx_ui::widgets::{StatusKind, StatusOverlay};
use plx_ui::Rect;

/// The read-out's one control. A local id of its own, the next one above the hero's ids
/// ([`super::hero::ELEM_AUDIO_SUBTITLES`]); like them it is its own engine key
/// (`DetailScreen::is_identity_local`).
pub const RETRY_ELEM: u32 = super::hero::ELEM_AUDIO_SUBTITLES + 1;

/// The read-out's one focus group, declared in place of every other group while the page is
/// failed (`DetailScreen::groups`).
pub const RETRY_GROUP: GroupId = GroupId(9);

/// The read-out, built the same way for the draw and for the control's rect, so the pill that is
/// drawn and the rect a click lands in are one expression.
pub fn overlay<'a>() -> StatusOverlay<'a> {
    StatusOverlay::new(Rect::FULL, plx_os::i18n::msg::browse_home_failed_c(), StatusKind::Failed)
        .page(plx_ui::icons::Icon::ServerBadgeMinus)
        .action(plx_os::i18n::msg::browse_action_retry_c())
}

/// *Try again*'s pill, through the geometry the draw uses. The overlay always carries an action, so
/// the frame is always there; the page-filling rect is only the type's answer to a case that cannot
/// arise.
pub fn retry_rect(measure: &dyn Measure) -> Rect {
    overlay().action_frame_measured(measure).unwrap_or(Rect::FULL)
}
