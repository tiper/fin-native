//! The library page header (redesign spec, "Header"): the library names, the Library options and the
//! page's controls (Collections › and Recommended › as places, a divider, then the sort control:
//! icon, the chosen criterion, a down chevron, with "Sort & Filter" as its unshown name
//! ([`HeaderItem::sort_control`], which the Collections page's Sort and the Watchlist's wear too); on the Collections page, the
//! crumb and its Sort) on the ui
//! [`PageHeader`], one focus row at y 148.
//!
//! This file DECLARES the row: which names, which controls, which element each focusable item is.
//! The row is built here, once per sync, as a ui [`HeaderRow`]; its layout, paint, motion and focus
//! group are the ui's (`plx_ui::page_header`), the same ones the Collections, Recommended and
//! Watchlist pages wear. The names are packed against the room the right cluster leaves
//! (`PageHeader::layout`'s `left_budget`, sized from the cluster's widest case), so the names never
//! shift when a value changes, the Collections answer (unknown, none, a count of up to three digits)
//! included; a library whose count needs a fourth digit widens its own budget once; the
//! controls inside the cluster stand by what they draw, one gap apart. The row's MOTION (`header_capsules`, `header_pop`) lives on the screen, not in the
//! row: a library switch replaces the whole row, and the pill must travel across it.
use super::*;
use plx_machine::machine::Measure;
use plx_ui::page_header::{self, item, HeaderFocus, HeaderItem, HeaderRow, MoreOpener, NameSlot, NamesIn, PageHeader, PageHeaderSpec, Slot};

/// What the read-out's filtered count was taken under: the library listing (the key `unfiltered`
/// carries) and the filter terms in force. A count is only ever shown for the key it was counted under.
#[derive(Clone, PartialEq)]
pub(super) struct FilterKey {
    library: (LibrarySectionIdentity, u32, plx_data::browse::LibraryType),
    genre: Option<String>,
    unwatched: bool,
}

/// A control that opens a page (Collections, Recommended): the row draws a chevron after it. `name`
/// is its accessibility name (`header.collections`), a word and never its label, which is the
/// language's.
fn place(name: &'static str, label: impl Into<String>) -> HeaderItem {
    HeaderItem { place: true, a11y: Some(name), ..item(label) }
}

/// The hairline between the places and the sort control. Not a stop.
fn divider() -> HeaderItem {
    HeaderItem { divider: true, ..Default::default() }
}

/// **The header's right cluster** as the screen's state says it, and in its widest case (every value
/// at its longest), as pure data: the ONE builder of both, so a page that must show what the Library
/// header would do (Settings' Library order preview) builds them through it and cannot drift from
/// the real header. `sorts` are the labels of every sort the library offers and `sort` the one in
/// force (the widest sizes the sort control's slot for the longest of them).
pub(super) fn header_controls(
    readout: Option<String>, collections: CollectionsControl, listing: Listing, sort: &str, sorts: &[&str], measure: &dyn Measure,
) -> (Vec<HeaderItem>, Vec<HeaderItem>) {
    use plx_os::i18n::msg;
    let locale = plx_os::i18n::current();
    let mut right = Vec::new();
    let mut widest = Vec::new();
    if let Some(readout) = readout {
        let item = HeaderItem { label: readout, readout: true, ..Default::default() };
        widest.push(item.clone());
        right.push(item);
    }
    // The control is on the row only with its count: an unknown answer lays out as none (see
    // [`CollectionsControl`]), so "Collections" is never drawn bare. The slot is the widest
    // count the cluster's BUDGET allows for (THREE digits; a library whose count needs more
    // widens only its own slot, here and in the widest case). It is no air inside the row: the
    // control is placed by the text it draws, so the gap to "Recommended" is the row's one gap
    // whatever the count is. The budget reserves it for EVERY answer, a library with none
    // included: a cold cache reads Unknown for a round trip and then Hidden, and the names
    // must not repack between the two (the room they pack into is measured once and never
    // moves).
    let slot_of = |count: u32| Some(locale.number(i64::from(count.max(999))));
    let control = |value, slot| HeaderItem { value, value_slot: slot, ..place("collections", msg::library_header_collections()) };
    match collections {
        CollectionsControl::Count(count) => {
            let slot = slot_of(count);
            right.push(HeaderItem { selected_plate: listing == Listing::Collections,
                ..control(Some(locale.number(i64::from(count))), slot.clone()) });
            widest.push(control(slot.clone(), slot));
        }
        CollectionsControl::Unknown | CollectionsControl::Hidden => {
            let slot = slot_of(0);
            widest.push(control(slot.clone(), slot));
        }
    }
    right.push(HeaderItem { selected_plate: listing == Listing::Recommended,
        ..place("recommended", msg::library_header_recommended()) });
    widest.push(place("recommended", msg::library_header_recommended()));
    // The divider and the sort control draw what the criterion says, with no slot for it: the
    // control ends at the row's edge and the places slide as the criterion's width changes. The
    // widest criterion only sizes the room the library names may use.
    right.push(divider());
    widest.push(divider());
    let longest = sorts.iter().copied().chain([sort])
        .map(|label| (page_header::cluster_width(&[HeaderItem::sort_control("", label)], measure), label))
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .map_or("", |(_, label)| label).to_owned();
    right.push(HeaderItem::sort_control(msg::library_header_sort_filter(), sort));
    widest.push(HeaderItem::sort_control(msg::library_header_sort_filter(), longest));
    (right, widest)
}

/// **Does a library header of `kind` reserve a slot for a visiting name?** Whenever the roster can
/// need one: more than two favourites, or a library of the kind that is not a favourite (one visit
/// to it must never shift or displace the shortcuts). The ONE decision, read from the directory by
/// the real header and by every picture of it.
pub(crate) fn visit_slot(directory: plx_data::stores::browse::DirectoryView<'_>, kind: SecKind, favourites: usize) -> bool {
    favourites > 2 || directory.sections().iter().any(|section| section.kind == kind && !section.row.pinned)
}

/// **A picture of the Library header's names row for a library of `kind`** (Settings' Library order
/// preview): the same assembly the real header runs ([`page_header::names_row`]) against the same
/// controls at rest (no filter, the Collections answer not in, the grid on Title; the right cluster
/// of a real library differs only by a read-out or a server sort longer than the shared table's),
/// the same visit-slot decision ([`visit_slot`], from `directory`) and the same name fit
/// ([`page_header::fit_name`], which the caller's `label_at` reads), differing only in what the
/// picture is for: the ⋯ is drawn when it holds folded names ([`MoreOpener::WhenFolded`]).
pub(crate) fn names_preview<K: Clone + PartialEq>(
    pills: &[K], current: &K, kind: SecKind, directory: plx_data::stores::browse::DirectoryView<'_>, measure: &dyn Measure,
    label_at: impl Fn(&K, f32) -> String,
) -> page_header::NamesRow<K> {
    // a page that cannot know the listing's sorts takes every sort the shared table knows as the
    // candidates for the widest (a real library offers a subset plus the server's own)
    let sorts: Vec<&str> = plx_data::sortfacts::SortFact::ALL.iter().map(|fact| fact.label()).collect();
    let (right, widest) = header_controls(None, CollectionsControl::Unknown, Listing::All, plx_data::sortfacts::SortFact::Title.label(), &sorts, measure);
    let names = NamesIn {
        pills, current: Some(current), visit_slot: visit_slot(directory, kind, pills.len()), more: MoreOpener::WhenFolded, right, widest: &widest,
    };
    page_header::names_row(names, measure, label_at)
}

impl LibraryScreen {
    /// Every focusable element of the header row in focus order: favorite shortcuts, options, the
    /// current visit when present, then the controls (on Collections, the crumb's root, then Sort).
    pub(super) fn header_elems(&self) -> Vec<u32> {
        self.header.row().map_or_else(Vec::new, |row| row.stops.iter().map(|(elem, _)| *elem).collect())
    }

    pub(super) fn is_header_elem(&self, elem: u32) -> bool {
        self.header_focus().is_some_and(|focus| focus.contains(elem))
    }

    /// The Sort & Filter control's value: the sort in force, a queued edit included. The scroll
    /// capsule carries the same string. Borrowed from the listing (or the catalogue), so the capsule,
    /// which asks every frame it is up, allocates nothing for it.
    pub(super) fn sort_value<'a, H: LibraryLike>(&self, cx: &Cx<'a, H>) -> &'a str {
        let listing = H::listing(cx);
        let queued = self.pending.grid().filter(|(target, _)| target.matches(listing)).map(|(_, action)| action);
        let sort = match queued {
            Some(GridAction::Sort { key, .. }) => listing.sorts().iter().find(|sort| &sort.key == key),
            _ => listing.sorts().get(listing.sort_index()),
        };
        sort.map_or(plx_os::i18n::msg::browse_library_title(), |sort| sort.label())
    }

    /// Is a genre or Unwatched filter in force (a queued edit included)? Only then does the row
    /// carry the "312 of 1,284" read-out.
    pub(super) fn filter_active<H: LibraryLike>(&self, cx: &Cx<'_, H>) -> bool {
        let listing = H::listing(cx);
        let queued = self.pending.grid().filter(|(target, _)| target.matches(listing)).map(|(_, action)| action);
        let genre = match queued {
            Some(GridAction::Genre { id }) => id.is_some(),
            _ => listing.genre().is_some(),
        };
        let unwatched = match queued {
            Some(GridAction::Unwatched { desired }) => *desired,
            _ => listing.unwatched(),
        };
        genre || unwatched
    }

    /// What a filtered total belongs to: this listing's library and the filter terms it was asked
    /// with. `None` while the listing has no identity or the page no section.
    fn filter_key<H: LibraryLike>(&self, cx: &Cx<'_, H>) -> Option<FilterKey> {
        let listing = H::listing(cx);
        let (id, section) = (listing.id()?, self.section.clone()?);
        Some(FilterKey {
            library: (section, id.epoch, listing.library_type()),
            genre: listing.genre().map(|genre| genre.id.clone()),
            unwatched: listing.unwatched(),
        })
    }

    /// Remember the totals the read-out is made of: the data layer publishes only the filtered
    /// total, and the read-out needs the unfiltered one beside it.
    ///
    /// **The filtered total is held across a reload that keeps its terms.** A sort pick forgets the
    /// listing (`reset_listing`: loading, no total) under the same genre and Unwatched terms, and
    /// the count of what those terms match has not changed; so the read-out keeps saying it, and
    /// stays on the row to glide with its neighbours instead of leaving and coming back when the
    /// listing lands. The held count is dropped the first time the page sees any other library or
    /// other terms, and when the listing lands unfiltered, so it is never shown for a filter it
    /// was not counted under.
    fn note_totals<H: LibraryLike>(&mut self, cx: &Cx<'_, H>) {
        let listing = H::listing(cx);
        let Some(key) = self.filter_key(cx) else { return };
        if listing.total() < 0 || listing.fetch() != SecFetch::Ready {
            if self.filtered.as_ref().is_some_and(|(held, _)| *held != key) { self.filtered = None; }
            return;
        }
        if key.genre.is_none() && !key.unwatched {
            self.unfiltered = Some((key.library, listing.total()));
            self.filtered = None;
        } else {
            if self.unfiltered.as_ref().is_some_and(|(seen, _)| *seen != key.library) { self.unfiltered = None; }
            self.filtered = Some((key, listing.total()));
        }
    }

    /// "312 of 1,284 films", once a filter is on and the unfiltered total is known. The noun is the
    /// listing's own (`LibraryType::count_label`, the words the Sources list's counts wear), so the
    /// read-out cannot be taken for a count of the Collections beside it.
    fn filtered_readout<H: LibraryLike>(&self, cx: &Cx<'_, H>) -> Option<String> {
        if !self.filter_active(cx) { return None; }
        let listing = H::listing(cx);
        let (_, total) = self.unfiltered.as_ref()?;
        // The listing's own count once it has one; while it reloads under the same terms, the count
        // held for them (see [`Self::note_totals`]).
        let matching = if listing.total() >= 0 && listing.fetch() == SecFetch::Ready {
            listing.total()
        } else {
            let (held, matching) = self.filtered.as_ref()?;
            if self.filter_key(cx).as_ref() != Some(held) { return None; }
            *matching
        };
        let locale = plx_os::i18n::current();
        let of_total = listing.library_type().count_label(self.kind, *total);
        Some(plx_os::i18n::msg::library_header_filtered_of(&locale.number(matching), &of_total))
    }

    /// The right cluster as it is now, and in its widest case (every value at its longest), which
    /// fixes the cluster's BUDGET so the room the library names may use does not change when a
    /// value does.
    pub(super) fn header_right<H: LibraryLike>(&self, cx: &Cx<'_, H>) -> (Vec<HeaderItem>, Vec<HeaderItem>) {
        let sorts: Vec<&str> = H::listing(cx).sorts().iter().map(|sort| sort.label()).collect();
        header_controls(self.filtered_readout(cx), self.collections, self.listing, self.sort_value(cx), &sorts, cx.measure)
    }

    /// A fingerprint of everything [`Self::refresh_header`] reads, cheap enough for every frame.
    fn header_key<H: LibraryLike>(&self, cx: &Cx<'_, H>) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        let listing = H::listing(cx);
        let directory = H::directory(cx);
        (std::ptr::from_ref(plx_os::i18n::current()) as usize).hash(&mut h);
        self.view_section(cx).hash(&mut h);
        self.collections.hash(&mut h);
        self.listing.hash(&mut h);
        self.listed.code().hash(&mut h);
        matches!(self.kind, SecKind::Movie).hash(&mut h);
        self.switched.hash(&mut h);
        for section in directory.sections() {
            (section.sid.map(|sid| sid.raw()), section.key, section.row.title.as_str(), section.row.pinned).hash(&mut h);
        }
        // the order the pills are PRESENTED in is what a Library order moves: `sections` stays in table order
        directory.presentation_order().hash(&mut h);
        for (id, source) in directory.sources() { (id.raw(), source.handle.as_str()).hash(&mut h); }
        (listing.id().map(|id| (id.epoch, id.section, id.query)), listing.sort_index(), listing.sort_desc(),
            listing.unwatched(), listing.genre().map(|genre| genre.id.as_str()), listing.total(),
            listing.fetch() == SecFetch::Ready, listing.sorts().len(), self.unfiltered.as_ref().map(|(_, total)| *total),
            self.filtered.as_ref().map(|(_, total)| *total)).hash(&mut h);
        match self.pending.grid().filter(|(target, _)| target.matches(listing)).map(|(_, action)| action) {
            Some(GridAction::Sort { key, desc }) => (1u8, key.as_str(), *desc).hash(&mut h),
            Some(GridAction::Unwatched { desired }) => (2u8, *desired).hash(&mut h),
            Some(GridAction::Genre { id }) => (3u8, id.as_deref()).hash(&mut h),
            Some(GridAction::LibraryType(kind)) => (4u8, kind.code()).hash(&mut h),
            None => 0u8.hash(&mut h),
        }
        h.finish()
    }

    /// Rebuild everything the header row is made of: the Collections control's state, the names
    /// that fit (`self.libraries`), and the cached geometry.
    ///
    /// **The control is the answer for THIS library, and nothing else.** [`Self::view_section`] is
    /// the library the row names (a switch's target from the moment it is chosen), so the answer
    /// read is that library's and a switch never carries the one it left: while the new library's
    /// count is not known the control is not on the row, and a count that lands later fades in
    /// through the row's [`HeaderSwap`] (the probe is warmed for every library when the directory
    /// lands, so it is usually known before the switch is drawn, and the control is then in the
    /// new row's first frame). The control the library left is not drawn on the new one at all,
    /// not even fading out ([`HeaderSwap::begin_switch`]). The one thing carried is this SAME
    /// library's own last count across a read of nothing (see below), never another's.
    pub(super) fn refresh_header<H: LibraryLike>(&mut self, cx: &Cx<'_, H>) {
        let view = self.view_section(cx);
        let of = view.and_then(|index| H::directory(cx).sections().get(index)).map(|section| (section.sid, section.key));
        let read = view.map_or(CollectionsControl::Unknown, |index| CollectionsControl::of(H::collections_count(cx, index)));
        // The probe reading nothing AGAIN for the library the row already names (its server was
        // re-pointed at a new client, so the cache missed for a round trip) is not news: the count
        // the row has stays, with its focus stop, until an answer replaces it. Across a switch it
        // never does (`of` is another library's), so a count is only ever one this library gave.
        self.collections = match (read, self.collections) {
            (CollectionsControl::Unknown, held @ CollectionsControl::Count(_)) if of == self.header_of => held,
            (read, _) => read,
        };
        self.note_totals(cx);
        // `sync` runs every frame; the row (text measurement, packing) is rebuilt only when one
        // of the things it is made of changed.
        let key = self.header_key(cx);
        if self.header.is_current(key) { return; }
        let row = if self.listing == Listing::Collections { self.collections_header(cx) } else { self.names_header(cx) };
        if let Some(old) = self.header.row() {
            // The same library's row changing fades what comes and goes; a row for ANOTHER library
            // is the new page's own, and shows nothing of the one left (see `HeaderSwap::begin_switch`).
            if of == self.header_of { self.header_swap.begin(old, &row); } else { self.header_swap.begin_switch(old, &row); }
            row.reflow_selection(old, &mut self.header_capsules);
        }
        self.header_of = of;
        self.header.install(key, row);
    }

    /// The row of the Library page itself: the library names that fit, then the controls.
    fn names_header<H: LibraryLike>(&mut self, cx: &Cx<'_, H>) -> HeaderRow {
        let (right, widest) = self.header_right(cx);
        let measure = cx.measure;

        self.libraries.clear();
        self.folded.clear();
        let Some(current) = self.view_section(cx) else {
            let page = PageHeader::new(PageHeaderSpec { right, ..Default::default() }, &widest, measure);
            let layout = page.layout(SCR_W, measure);
            return self.header_row(page, layout);
        };
        let directory = H::directory(cx);
        let mut all: Vec<(u32, usize)> = Vec::new();
        for (index, section) in directory.favorite_sections_for(current).filter(|(_, section)| section.row.pinned) {
            let Some(sid) = section.sid else { continue };
            let elem = self.keys.register(
                LibraryIdentity::Library(LibrarySectionIdentity { sid, key: section.key }),
                LIBRARY_GROUP, all.len());
            all.push((elem, index));
        }
        let sections: Vec<usize> = all.iter().map(|(_, section)| *section).collect();
        let on_view = directory.sections()[current].sid.map(|_| &current);
        let names = NamesIn {
            pills: &sections, current: on_view, visit_slot: visit_slot(directory, directory.sections()[current].kind, all.len()),
            more: MoreOpener::Always, right, widest: &widest,
        };
        let row = page_header::names_row(names, measure, |section, room| {
            let (title, owner) = self.library_name(*section, cx);
            page_header::fit_name(title, owner, room, measure)
        });
        self.folded = row.folded.iter().filter_map(|section| u32::try_from(*section).ok()).collect();
        for slot in &row.slots {
            match slot {
                // The menu belongs to the fixed shortcuts. Keep its opener in that same slot while
                // the active visit comes and goes beside it.
                NameSlot::More => self.libraries.push((MORE, usize::MAX)),
                NameSlot::Pill(section) => {
                    let elem = match all.iter().find(|(_, shortcut)| shortcut == section) {
                        Some((elem, _)) => *elem,
                        None => {
                            let (sid, key) = (directory.sections()[*section].sid, directory.sections()[*section].key);
                            let Some(sid) = sid else { continue };
                            self.keys.register(LibraryIdentity::Library(LibrarySectionIdentity { sid, key }), LIBRARY_GROUP, self.libraries.len())
                        }
                    };
                    self.libraries.push((elem, *section));
                }
            }
        }
        self.header_row(row.page, row.layout)
    }

    /// The built row: which element each focusable item of `layout` is. The names and options are
    /// `self.libraries`, the controls are [`Self::toolbar_elems`] (the first focusable right item is
    /// the first control, a read-out before it is no stop), the crumb's root is [`CRUMB_ROOT`] and
    /// its page name, the title, is none.
    fn header_row(&self, page: PageHeader, layout: page_header::HeaderLayout) -> HeaderRow {
        let controls = self.toolbar_elems();
        let stops = layout.items.iter().enumerate().filter(|(_, rect)| rect.focusable).filter_map(|(index, rect)| {
            let elem = match rect.slot {
                Slot::Left(i) => self.libraries.get(i).map(|(elem, _)| *elem),
                Slot::Overflow => Some(MORE),
                // The n-th FOCUSABLE right item is the n-th control: a read-out before them and the
                // divider among them are no stops.
                Slot::Right(i) => {
                    let rank = page.spec().right[..i].iter().filter(|it| it.focusable).count();
                    controls.get(rank).copied()
                }
                Slot::CrumbRoot => Some(CRUMB_ROOT),
                Slot::CrumbPage => None,
            };
            elem.map(|elem| (elem, index))
        }).collect();
        HeaderRow::new(page, layout, stops)
    }

    /// The Collections page's row: the crumb (its root is a stop, its page name the title) in
    /// place of the library names, and Sort when the server declares more than one.
    fn collections_header<H: LibraryLike>(&mut self, cx: &Cx<'_, H>) -> HeaderRow {
        self.libraries.clear();
        // Only the collections' own declared sorts count: until the store has switched the section
        // to the collections listing, `sorts()` is still the films' list.
        let declared = if self.listed == plx_data::browse::LibraryType::Collections { H::listing(cx).sorts().len() } else { 0 };
        let library = self.view_section(cx).map(|index| self.library_label(index, cx)).unwrap_or_default();
        let spec = super::collections_page::header_spec(&library, self.kind, declared, self.sort_value(cx));
        self.collections_sort = super::collections_page::sort_visible(declared);
        let page = PageHeader::new(spec, &[], cx.measure);
        let layout = page.layout(SCR_W, cx.measure);
        self.header_row(page, layout)
    }

    /// The row as one focus group of this page, at its scroll: what the engine's `Focusable`
    /// questions about the header are answered from (`None` before the row is built).
    pub(super) fn header_focus(&self) -> Option<HeaderFocus<'_>> {
        self.header.row().map(|row| row.focus(self.entry, LIBRARY_GROUP, (self.scroll.pos, self.scroll_target), content_clip()))
    }

    /// Where the header control or name `elem` is, in screen space at the drawn or target scroll.
    pub(super) fn header_rect(&self, elem: u32, at: At) -> Option<Rect> {
        self.header_focus()?.place(elem, at).map(|placed| placed.rect)
    }

    /// One frame of the row's motion: the focus capsule travels to the focused control, the
    /// selection plate lands under it. Focus off the row (or a menu holding it) closes the pop.
    pub(super) fn step_header<H: LibraryLike>(&mut self, cx: &Cx<'_, H>, dt: f32) {
        self.header_swap.step(dt);
        let Some(row) = self.header.row() else { return };
        row.step(self.entry, cx.focus.current, &mut self.header_capsules, &mut self.header_pop, dt);
    }

    /// Paint the row at the drawn scroll. The row scrolls away with the page: only its pill and
    /// the selected plate are chrome of its own.
    pub(super) fn draw_header<H: LibraryLike>(&self, f: &DrawFrame<'_, '_, H>) {
        let Some(row) = self.header.row() else { return };
        let top = layout::HEADER_Y - self.scroll.pos;
        // The row rides the scroll under the top fade, and keeps out of the band the returning
        // chrome is drawn in while the scroll is still bringing it home (`Handoff::clearance`).
        let p = f.painter.alpha(f.page_alpha * self.page_fade.alpha() * self.handoff.clearance(top));
        // The pill's outset (2 px) is far inside the cast's padding the band is grown by, so the band needs no pop.
        if !super::draw::document_band_visible(p, top, page_header::HEIGHT, 1.0) { return; }
        // A control a menu has lifted out of the page (`Screen::opener`) is not drawn here at all:
        // the opener lift is its only copy ([`Self::draw_opener`]), lit above the scrim. One rule
        // for every header opener (design review S12). The dispatcher decides when it stands down
        // (`lifted_opener`: a closing menu whose focus moved off the opener), so a lift is always
        // the control's whole menu-open stretch, during which the engine's focus is on the menu's
        // rows and the row's motion HOLDS.
        let lifted = f.lifted.filter(|key| key.entry == self.entry).map(|key| key.elem);
        row.draw_with(p, self.scroll.pos, &self.header_capsules, self.header_pop.scale_with(0, f.press), lifted, &self.header_swap);
    }

    /// **The opener lift for a header control** (`Screen::redraw_focused`): `elem`, the control a
    /// menu opened from, painted as the focused one above the menu's scrim, where the page pass
    /// left it out. Nothing is drawn for an element that is no stop of this row (a card).
    ///
    /// **It paints through the row's swap**, as the page's pass does: a sort picked from the menu
    /// rebuilds the row on the frame the menu is dismissed, and until the menu is gone this lift is
    /// the only thing that draws the sort control, so a lift that ignored the swap drew the new
    /// criterion whole from the first frame and popped when it stood down mid-slide.
    pub(super) fn draw_header_opener<H: LibraryLike>(&self, f: &DrawFrame<'_, '_, H>, elem: u32) {
        let Some(row) = self.header.row() else { return };
        let p = f.painter.alpha(f.page_alpha * self.page_fade.alpha() * self.handoff.clearance(layout::HEADER_Y - self.scroll.pos));
        row.draw_opener_with(p, self.scroll.pos, self.header_pop.scale_with(0, f.press), elem, &self.header_swap);
    }
}
