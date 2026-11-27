//! The Sort & Filter value (plan §1.3, §1.10): what the pill summarises and the popover edits.
//! Filtering and ordering are client-side over the stored rows, so a change makes no request.
use super::page::WatchTile;
use plx_data::watchlist::model::{Kind, TileState};
pub use plx_data::watchlist::order::SortKey;
use plx_data::watchlist::order::{sorted_filtered, FilterSpec, Show, SortSpec};
use plx_os::i18n::msg;
use plx_ui::form::{Form, FormId, FormSection, RowKey, RowKind};
use plx_ui::menu::{Class, GrowWidth, Host, Menu, Morph};
use plx_ui::table::Row;
use plx_ui::Rect;
use std::convert::Infallible;

/// The Type section: `Movies & Shows` / `Movies` / `TV Shows`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum TypeFilter {
    #[default]
    All,
    Movies,
    Shows,
}

/// The Show section: `All` / `Released` / `On your servers`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum ShowFilter {
    #[default]
    All,
    Released,
    OnYourServers,
}

/// The whole Sort & Filter value. `Default` is the page's opening order: the service's own
/// (newest added first), every type, every row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WatchlistOptionsState {
    pub type_: TypeFilter,
    pub sort: SortKey,
    pub descending: bool,
    pub show: ShowFilter,
}

/// A fingerprint of the value, for what is built from it (the header row's key).
impl std::hash::Hash for WatchlistOptionsState {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.type_.hash(state);
        std::mem::discriminant(&self.sort).hash(state);
        self.descending.hash(state);
        self.show.hash(state);
    }
}

impl Default for WatchlistOptionsState {
    fn default() -> Self {
        Self { type_: TypeFilter::All, sort: SortKey::DateAdded, descending: true, show: ShowFilter::All }
    }
}

impl WatchlistOptionsState {
    /// The data layer's spelling of this value.
    pub fn specs(&self) -> (SortSpec, FilterSpec) {
        let kind = match self.type_ {
            TypeFilter::All => None,
            TypeFilter::Movies => Some(Kind::Movie),
            TypeFilter::Shows => Some(Kind::Show),
        };
        let show = match self.show {
            ShowFilter::All => Show::All,
            ShowFilter::Released => Show::Released,
            ShowFilter::OnYourServers => Show::OnYourServers,
        };
        (SortSpec { key: self.sort, desc: self.descending }, FilterSpec { kind, show })
    }

    /// The canonical bytes of this value (the menu argument's and the menu screen's state hash).
    pub fn write_canon(&self, c: &mut plx_machine::machine::Canon) {
        c.u32(match self.type_ {
            TypeFilter::All => 0,
            TypeFilter::Movies => 1,
            TypeFilter::Shows => 2,
        })
        .u32(match self.sort {
            SortKey::Title => 0,
            SortKey::DateAdded => 1,
            SortKey::Release => 2,
            SortKey::Rating => 3,
        })
        .bool(self.descending)
        .u32(match self.show {
            ShowFilter::All => 0,
            ShowFilter::Released => 1,
            ShowFilter::OnYourServers => 2,
        });
    }

    /// The value the store's specs stand for: the inverse of [`specs`](Self::specs).
    pub fn from_specs(sort: SortSpec, filter: FilterSpec) -> Self {
        Self {
            type_: match filter.kind {
                None => TypeFilter::All,
                Some(Kind::Movie) => TypeFilter::Movies,
                Some(Kind::Show) => TypeFilter::Shows,
            },
            sort: sort.key,
            descending: sort.desc,
            show: match filter.show {
                Show::All => ShowFilter::All,
                Show::Released => ShowFilter::Released,
                Show::OnYourServers => ShowFilter::OnYourServers,
            },
        }
    }

    /// The sort control's criterion, the one text it draws: the current SORT, by its menu label
    /// ("Date added", "Title"), the same fact the Library's sort control names. A filter (Type,
    /// Show) never shows here; it shows as the header's "N of M" read-out. The direction is not
    /// spelled: the open menu shows it as the checked sort row's value word, and the control has
    /// to share the header line with the title and the count in the longest language.
    pub fn summary(&self) -> String {
        OptionsRow::Sort(self.sort).label().to_string()
    }

    /// `rows` filtered and ordered by this value, exactly as the data layer does it
    /// (`order::sorted_filtered`); "On your servers" reads each tile's committed state.
    pub fn apply(&self, rows: &[WatchTile]) -> Vec<WatchTile> {
        let (sort, filter) = self.specs();
        let data: Vec<_> = rows.iter().map(|t| t.row.clone()).collect();
        let playable = |i: usize| matches!(rows[i].state, TileState::Available | TileState::ComingSoonMatched);
        sorted_filtered(&data, &sort, &filter, &playable).into_iter().map(|i| rows[i].clone()).collect()
    }
}

/// A sort key's own first direction, the one choosing it from another key lands on: Title reads
/// A to Z, the others newest, highest and most recent first. Date added descending is the
/// service's own order and the page's default.
pub fn natural_desc(key: SortKey) -> bool {
    !matches!(key, SortKey::Title)
}

/// One selectable row of the Sort & Filter menu: its identity is its meaning, never its position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OptionsRow {
    Type(TypeFilter),
    Sort(SortKey),
    Show(ShowFilter),
}

impl OptionsRow {
    /// Every row in draw order: Show, Sort, Filter (boards WatchlistOptions: 3 + 4 + 3 rows).
    pub const ALL: [OptionsRow; 10] = [
        OptionsRow::Type(TypeFilter::All),
        OptionsRow::Type(TypeFilter::Movies),
        OptionsRow::Type(TypeFilter::Shows),
        OptionsRow::Sort(SortKey::DateAdded),
        OptionsRow::Sort(SortKey::Title),
        OptionsRow::Sort(SortKey::Release),
        OptionsRow::Sort(SortKey::Rating),
        OptionsRow::Show(ShowFilter::All),
        OptionsRow::Show(ShowFilter::Released),
        OptionsRow::Show(ShowFilter::OnYourServers),
    ];

    /// The row's accessibility name (`watchlist_menu.<name>`): its meaning as a static word, never
    /// its position (the order is draw order and may change).
    pub fn a11y_name(self) -> &'static str {
        match self {
            OptionsRow::Type(TypeFilter::All) => "type_all",
            OptionsRow::Type(TypeFilter::Movies) => "type_movies",
            OptionsRow::Type(TypeFilter::Shows) => "type_shows",
            OptionsRow::Sort(SortKey::DateAdded) => "sort_date_added",
            OptionsRow::Sort(SortKey::Title) => "sort_title",
            OptionsRow::Sort(SortKey::Release) => "sort_release",
            OptionsRow::Sort(SortKey::Rating) => "sort_rating",
            OptionsRow::Show(ShowFilter::All) => "show_all",
            OptionsRow::Show(ShowFilter::Released) => "show_released",
            OptionsRow::Show(ShowFilter::OnYourServers) => "show_on_servers",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            OptionsRow::Type(TypeFilter::All) => msg::watchlist_filter_type_all(),
            OptionsRow::Type(TypeFilter::Movies) => msg::watchlist_filter_type_movies(),
            OptionsRow::Type(TypeFilter::Shows) => msg::watchlist_filter_type_shows(),
            // The shared table's name for the fact, the one the Library's Sort section wears.
            OptionsRow::Sort(key) => key.fact().label(),
            OptionsRow::Show(ShowFilter::All) => msg::watchlist_filter_show_all(),
            OptionsRow::Show(ShowFilter::Released) => msg::watchlist_filter_show_released(),
            OptionsRow::Show(ShowFilter::OnYourServers) => msg::watchlist_filter_show_on_servers(),
        }
    }

    /// Whether `state` has this choice (the leading check).
    pub fn is_chosen(self, state: &WatchlistOptionsState) -> bool {
        match self {
            OptionsRow::Type(t) => state.type_ == t,
            OptionsRow::Sort(k) => state.sort == k,
            OptionsRow::Show(s) => state.show == s,
        }
    }

    /// Press this row. A different sort key lands on its [`natural_desc`]; pressing the key that is
    /// already chosen flips the direction, the Library sort menu's idiom (the direction word on the
    /// active row). Returns whether `state` changed (a Type or Show press on the chosen row does not).
    pub fn apply(self, state: &mut WatchlistOptionsState) -> bool {
        let before = *state;
        match self {
            OptionsRow::Type(t) => state.type_ = t,
            OptionsRow::Show(s) => state.show = s,
            OptionsRow::Sort(k) if state.sort == k => state.descending = !state.descending,
            OptionsRow::Sort(k) => {
                state.sort = k;
                state.descending = natural_desc(k);
            }
        }
        *state != before
    }
}

/// A row's identity IS what it chooses, and its focus key is its position in [`OptionsRow::ALL`] (draw order),
/// as it always was: the form is rebuilt in place when a press changes the value, and a key names the same
/// choice before and after.
impl FormId for OptionsRow {
    fn key(&self) -> RowKey {
        RowKey(OptionsRow::ALL.iter().position(|row| row == self).unwrap_or(0) as u32)
    }
    fn a11y(&self) -> Option<&'static str> {
        Some(self.a11y_name())
    }
}

/// The menu's three sections for `state`, named as the Library's Sort & Filter panel names its own:
/// Show (the kind of title), Sort (the checked key carries the direction as a value word, "Newest
/// first", from the same table), Filter (availability). Pure, so the text-fit suite drives the
/// real builder.
pub fn options_form(state: &WatchlistOptionsState) -> Form<OptionsRow, OptionsRow, Infallible> {
    let mut type_ = FormSection::new(msg::library_header_show());
    let mut sort = FormSection::new(msg::browse_library_sort());
    let mut show = FormSection::new(msg::browse_library_filter());
    for id in OptionsRow::ALL {
        let mut row = Row::new(id.label()).checked(id.is_chosen(state));
        if let OptionsRow::Sort(key) = id {
            if let Some(axis) = key.fact().axis().filter(|_| id.is_chosen(state)) { row = row.value(axis.direction_word(state.descending)); }
        }
        match id {
            OptionsRow::Type(_) => type_ = type_.item(id, RowKind::Choice, id, row),
            OptionsRow::Sort(_) => sort = sort.item(id, RowKind::Choice, id, row),
            OptionsRow::Show(_) => show = show.item(id, RowKind::Choice, id, row),
        }
    }
    Form::new().section(type_).section(sort).section(show)
}

/// The sections [`options_form`] draws, as the table reads them (each section's heading and rows), for the
/// text-fit suite and the route tests that compare the Watchlist's words with the Library's.
#[cfg(any(test, feature = "test-support"))]
pub fn sections(state: &WatchlistOptionsState) -> Vec<plx_ui::table::Section> {
    let mut form = plx_ui::form::FormTable::new(crate::registry::BAND);
    form.open(options_form(state), None);
    std::mem::take(&mut form.table.sections)
}

/// The row a value opens on: the first row it has chosen (the Type choice), so a stray OK on a freshly opened
/// menu re-asserts what is already set.
pub fn opening_row(state: &WatchlistOptionsState) -> Option<OptionsRow> {
    OptionsRow::ALL.into_iter().find(|row| row.is_chosen(state))
}

/// **The Sort & Filter menu**: the shared [`Menu`] growing out of the header pill (DS6, DS7) over its rows, the
/// glass on the frozen ground, compact one-word choices, the pill covered top edge to top edge. It draws NO
/// pill: the pill is the control's own, painted by the host's opener lift. A page-less menu: its root is the
/// only page there is.
pub type OptionsMenu = Menu<Infallible, OptionsRow, OptionsRow>;

/// The menu for a header pill at `pill`.
pub fn options_menu(pill: Rect) -> OptionsMenu {
    // the header pill opens it: the close's blob whitens toward the pill's own focused fill
    let host = Host::Grows { pill, clear_y: None, fill: Some(plx_ui::page_header::OPENER_FILL), morph: Morph::FromPill, width: GrowWidth::Catch };
    Menu::new(host, Class::Action, crate::registry::BAND)
}

#[cfg(test)]
#[path = "options_tests.rs"]
mod options_tests;
