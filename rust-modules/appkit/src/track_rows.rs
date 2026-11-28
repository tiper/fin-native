//! **The one place an item's tracks become menu rows**, for the in-player Tracks menu
//! ([`crate::track_menu`]) and the Detail screen's Audio & Subtitles pages
//! ([`crate::preplay_tracks`]).
//!
//! Four pages, each a [`Page`] (rows, title band and the row focus opens on) of the player's own row alphabet
//! ([`TrackRow`], [`TrackPage`]): [`audio_page`], [`subtitles_page`], [`other_languages_page`] and
//! [`language_page`]. What they are built from is plain values: the item's streams, the grouping
//! `metadata::sub_layout::sub_sections` answered (the one source of what sits under which header, which
//! languages are behind the Other languages row and in what order) and a [`TrackCx`], the context the rows are
//! for. A host that has its own alphabet adapts a page with [`plx_ui::form::Form::map`] (Detail's
//! `PickRow`), which carries the language aliases across; the player uses the page as it is.
//!
//! **The context says what exists.** [`TrackCx::Preplay`] is a viewer choosing before a playback exists: the two
//! pages hold tracks and Off and nothing else, because Forced only, Timing, Style, the remembered-subtitle note,
//! the Plex Pass DSP rows and the per-route marks on an audio row all act on, or read, a session. The builders
//! leave those out for it (they are not demoted, they are not declared). [`TrackCx::Playing`] carries what every page reads (the checked audio and subtitle); what only one page reads
//! comes in as that page's own argument ([`AudioRoute`] for the Audio page, [`SubtitleRoute`] for the Subtitles
//! page), present exactly when the context is playing. What is checked is read off the context only ([`TrackCx::checked`]), so
//! "Forced only holds the one checkmark, the forced track it drew is not also checked" is one rule for the root,
//! the Other languages page and a language page.
//!
//! **The Other languages flow is the same on both hosts**: one *Other languages* row on the Subtitles page (its
//! value the language count, or, when the checked track lives behind it, a check and that language), a page with
//! one row per language A-Z (a single-track language is a direct pick row, a several-track one a drill-in, and
//! [`other_lang_row`] is the one rule that says which), and a language page with its tracks ranked. A language is
//! identified by [`LangId`] (the Plex stream id of its first track in the item's FULL list), never by a list
//! position, so tracks leaving or arriving cannot relabel an open page; a language that changes shape under a
//! viewer (one track <-> several) is declared as an alias ([`Form::alias`]) so a pop lands on the row that
//! stands for it now.
use plx_data::metadata::sub_layout::{OtherLang, RowBadge, SubHeader, SubModel, SubRow, SubTrack};
use plx_data::metadata::track_label::{self, Kind};
use plx_data::metadata::track_names::TrackNames;
use plx_data::metadata::{friendly_codec, Stream};
use plx_ui::form::{Form, FormSection, RowKind};
use plx_ui::menu::Page;
use plx_ui::table::{Badge, Row, Section};

use crate::track_menu::{format_offset, TrackPage, TrackRow};

/// The page type of both tracks pages' alphabet.
pub type TrackPageOf = Page<TrackRow, (), TrackPage>;

/// **What the subtitle is, as the context knows it**: the one value that says which row is checked and which
/// track is on screen. Never a track index paired with a flag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubState<'a> {
    /// No subtitle: the Off row is checked and nothing is drawn.
    Off,
    /// A track (the index into the item's subtitle list): its row is checked and it is drawn.
    Track(usize),
    /// Forced only is the MODE: its row holds the one checkmark (`readout` is the language it shows or looked
    /// for), and `drawn` is the forced track it put on screen, if one matched. That track is NOT also checked.
    ForcedOnly { readout: &'a str, drawn: Option<usize> },
}

impl SubState<'_> {
    /// The track whose row carries the checkmark: none while Forced only holds it.
    pub fn checked(&self) -> Option<usize> {
        match *self {
            SubState::Track(i) => Some(i),
            SubState::Off | SubState::ForcedOnly { .. } => None,
        }
    }

    /// The track the picture shows (what keeps Timing live): the checked one, or the forced track the mode drew.
    pub fn drawn(&self) -> Option<usize> {
        match *self {
            SubState::Track(i) => Some(i),
            SubState::ForcedOnly { drawn, .. } => drawn,
            SubState::Off => None,
        }
    }
}

/// What an audio row's sub-line says about the route the track plays on, read once by the player
/// ([`AudioMark::of`]); a viewer choosing before playback has no route, so its rows are [`Self::Plain`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AudioMark {
    #[default]
    Plain,
    /// The server converts this track.
    Converted,
    /// The app decodes this surround track itself and plays it in stereo.
    InAppStereo,
}

impl AudioMark {
    /// The server's mark outranks the app's: a converted track is not also "played in stereo".
    pub fn of(converted_by_server: bool, decoded_in_app: bool) -> Self {
        if converted_by_server {
            Self::Converted
        } else if decoded_in_app {
            Self::InAppStereo
        } else {
            Self::Plain
        }
    }
}

/// The Audio page's Plex Pass DSP pair (Boost dialog, Normalize loudness), as the player offers it.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Dsp {
    /// Not offered at all (no Plex Pass: the one silent absence).
    #[default]
    Hidden,
    /// Offered, reading `shown`; `note` is the consequence line under the pair (a burned subtitle, ...).
    Offered { shown: plx_plex::plex::AudioEnhancements, note: Option<String> },
    /// Drawn dim and reading Off, with the plain-language `reason` under them.
    Disabled { reason: String },
}

/// **What only the route knows about the Audio page**: the Plex Pass DSP pair and each track's mark. Passed to
/// [`audio_page`] while playing, never otherwise.
pub struct AudioRoute<'a> {
    pub dsp: Dsp,
    /// Per audio track, what its sub-line says about the route (a track past the end is [`AudioMark::Plain`]).
    pub marks: &'a [AudioMark],
}

/// **What only the route knows about the Subtitles page**: the Timing read-out, whether the server draws the
/// subtitle, and the remembered-subtitle note. Passed to [`subtitles_page`] while playing, never otherwise.
pub struct SubtitleRoute {
    /// The Timing row's read-out, in ms.
    pub offset_ms: i64,
    /// Why the server draws the subtitle into the picture, when it does (Timing and Style are then disabled).
    pub burn: Option<plx_media::route::BurnReason>,
    /// The show's remembered subtitle was chosen for this play and the route shows none of it: one inert note
    /// closes the first group.
    pub memory_note: bool,
}

/// **The context a page's rows are built for**: what every page reads, the checked audio and subtitle.
#[derive(Clone, Copy)]
pub enum TrackCx<'a> {
    /// Choosing before playback (Detail): `audio` and `sub` are what is chosen; no session, no route.
    Preplay { audio: usize, sub: Option<usize> },
    /// The in-player menu: the audio track playing (an index into the item's audio list; `None` when the route
    /// names none) and the subtitle ([`SubState`]).
    Playing { audio: Option<usize>, sub: SubState<'a> },
}

impl<'a> TrackCx<'a> {
    /// The audio track whose row is checked.
    pub fn audio(&self) -> Option<usize> {
        match self {
            TrackCx::Preplay { audio, .. } => Some(*audio),
            TrackCx::Playing { audio, .. } => *audio,
        }
    }

    pub fn sub(&self) -> SubState<'a> {
        match self {
            TrackCx::Preplay { sub: Some(i), .. } => SubState::Track(*i),
            TrackCx::Preplay { sub: None, .. } => SubState::Off,
            TrackCx::Playing { sub, .. } => *sub,
        }
    }

    /// The subtitle track whose row is checked, on the root and on every page behind it.
    pub fn checked(&self) -> Option<usize> {
        self.sub().checked()
    }

    fn is_playing(&self) -> bool {
        matches!(self, TrackCx::Playing { .. })
    }
}

// ---- the pages ---------------------------------------------------------------------------------

/// **The Audio page**: a checked [`TrackRow::Audio`] per track, then, while playing, the DSP pair (its own
/// headerless section with its note) the route offers. Opens on the checked track. `route` is `Some` exactly when
/// `cx` is playing.
pub fn audio_page(cx: &TrackCx<'_>, audio: &[Stream], names: &TrackNames, route: Option<&AudioRoute<'_>>) -> TrackPageOf {
    use plx_os::i18n::msg;
    debug_assert_eq!(cx.is_playing(), route.is_some(), "the route's facts come with a playing context, and only then");
    let checked = cx.audio();
    let mut sec = FormSection::new(msg::widgets_tracks_audio());
    for i in 0..audio.len() {
        let mark = route.and_then(|r| r.marks.get(i).copied()).unwrap_or_default();
        sec = sec.item(
            TrackRow::Audio(i),
            RowKind::Choice,
            (),
            audio_track_row(audio, i, checked == Some(i), names, |sub| audio_line(sub, mark)),
        );
    }
    let mut form = Form::new().section(sec);
    if let Some(route) = route {
        form = match &route.dsp {
            Dsp::Hidden => form,
            Dsp::Offered { shown, note } => {
                let mut dsp = FormSection::new("")
                    .item(
                        TrackRow::Boost,
                        RowKind::Toggle,
                        (),
                        Row::new(msg::widgets_tracks_boost_dialog()).toggle(shown.boost_dialog),
                    )
                    .item(
                        TrackRow::Loudness,
                        RowKind::Toggle,
                        (),
                        Row::new(msg::widgets_tracks_normalize_loudness()).toggle(shown.normalize_loudness),
                    );
                if let Some(note) = note {
                    dsp = dsp.note(note.clone());
                }
                form.section(dsp)
            }
            // Every gate but "no Plex Pass" is a visible reason (owner direction, 2026-09-29): the rows stay in
            // the list, dim and reading Off, with a one-line non-selectable footnote naming why.
            Dsp::Disabled { reason } => form.section(
                FormSection::new("")
                    .item(
                        TrackRow::Boost,
                        RowKind::Toggle,
                        (),
                        Row::new(msg::widgets_tracks_boost_dialog()).toggle(false).dim(true),
                    )
                    .item(
                        TrackRow::Loudness,
                        RowKind::Toggle,
                        (),
                        Row::new(msg::widgets_tracks_normalize_loudness()).toggle(false).dim(true),
                    )
                    .note(reason.clone()),
            ),
        };
    }
    Page::new(form, msg::widgets_tracks_audio()).initial(Some(TrackRow::Audio(checked.unwrap_or(0))))
}

/// **The Subtitles page** from the grouping (`sub_layout::sub_sections`): Off (and, while playing, Forced only)
/// and every single-track "yours" language under one *Subtitles* header, a several-track "yours" language in its
/// own section, ONE *Other languages* drill-in, and, while playing, Timing and Style (disabled with the reason
/// while the server draws the subtitle) and the remembered-subtitle note. Opens on the row that stands for the
/// checked subtitle: Forced only while it is the mode, the Other languages row when the checked track lives
/// behind it, else its own row or Off. `route` is `Some` exactly when `cx` is playing.
pub fn subtitles_page(cx: &TrackCx<'_>, model: &SubModel, route: Option<&SubtitleRoute>) -> TrackPageOf {
    use plx_os::i18n::msg;
    debug_assert_eq!(cx.is_playing(), route.is_some(), "the route's facts come with a playing context, and only then");
    let sub = cx.sub();
    let checked = sub.checked();
    let forced = match sub {
        SubState::ForcedOnly { readout, .. } => Some(readout),
        _ => None,
    };
    let active_other = model.other.iter().find(|o| o.tracks.iter().any(|t| checked == Some(t.i)));
    let form = model.sections.iter().enumerate().fold(Form::new(), |form, (at, sec)| {
        let head = match &sec.header {
            SubHeader::Subtitles => Section::new(msg::widgets_tracks_subtitles()),
            SubHeader::Language { name, tracks } => {
                Section::new(name.clone()).accessory(msg::widgets_tracks_count(*tracks as i64))
            }
            SubHeader::Bare => Section::new(""),
        };
        let out = sec.rows.iter().fold(FormSection::from_head(head), |out, row| {
            // the identity comes from the model's own row target, never from the position
            let id = TrackRow::from(row.target());
            match (row, route) {
                (SubRow::Off, _) => out.item(
                    id,
                    RowKind::Choice,
                    (),
                    Row::new(msg::widgets_tracks_off()).checked(sub == SubState::Off),
                ),
                (SubRow::Flat(t), _) => out.item(id, RowKind::Choice, (), flat_row(t, checked)),
                (SubRow::InLanguage(t), _) => out.item(id, RowKind::Choice, (), in_lang_row(t, checked)),
                (SubRow::OtherLanguages { languages }, _) => {
                    let (value, on) = match active_other {
                        Some(o) => (o.name.clone(), true),
                        None => (languages.to_string(), false),
                    };
                    out.item(
                        id,
                        RowKind::Nav(TrackPage::OtherLanguages),
                        (),
                        Row::new(msg::widgets_tracks_other_languages()).value(value).checked(on),
                    )
                }
                // what acts on a session is not declared for a viewer choosing before one
                (SubRow::ForcedOnly | SubRow::Timing | SubRow::Style, None) => out,
                (SubRow::ForcedOnly, Some(_)) => {
                    let row = Row::new(msg::widgets_tracks_forced_only()).checked(forced.is_some());
                    // the read-out is a language name: the stream's own text, so it may elide before the row's
                    // catalog words do
                    out.item(id, RowKind::Choice, (), match forced {
                        Some(language) => row.value(language).server_value(),
                        None => row,
                    })
                }
                (SubRow::Timing, Some(f)) => out
                    .item(
                        id,
                        RowKind::Button,
                        (),
                        Row::new(msg::widgets_tracks_timing()).value(format_offset(f.offset_ms)).dim(sub.drawn().is_none()),
                    )
                    .disabled(f.burn.is_some()),
                (SubRow::Style, Some(f)) => {
                    let out = out
                        .item(id, RowKind::Nav(TrackPage::Style), (), Row::new(msg::widgets_tracks_style()))
                        .disabled(f.burn.is_some());
                    match f.burn {
                        Some(plx_media::route::BurnReason::AudioConversion) => out.note(msg::widgets_tracks_sub_burn_audio()),
                        Some(plx_media::route::BurnReason::Quality) => out.note(msg::widgets_tracks_sub_burn_quality()),
                        None => out,
                    }
                }
            }
        });
        let remembered = route.is_some_and(|r| r.memory_note);
        form.section(if at == 0 && remembered { out.note(msg::widgets_tracks_sub_remembered_note()) } else { out })
    });
    let initial = match sub {
        SubState::ForcedOnly { .. } => TrackRow::SubForcedOnly,
        SubState::Track(i) if model.other.iter().any(|o| o.tracks.iter().any(|t| t.i == i)) => TrackRow::OpenOther,
        SubState::Track(i) => TrackRow::Sub(i),
        SubState::Off => TrackRow::SubOff,
    };
    Page::new(form, msg::widgets_tracks_subtitles()).initial(Some(initial))
}

/// A language's row on the Other languages page: the pick row of its only track, or its drill-in. The one rule
/// for which, so a host that follows a language changing shape names the row by it.
pub fn other_lang_row(o: &OtherLang) -> TrackRow {
    match o.tracks.as_slice() {
        [t] => TrackRow::Sub(t.i),
        _ => TrackRow::OpenLang(o.id),
    }
}

/// **The Other languages page**: one row per language, A-Z, from `other` (`SubModel::other`). A single-track
/// language is a direct pick row (checked when it is the checked track); a several-track one is a drill-in
/// reading out its track count, or, when the checked track is inside it, a check and that variant. Opens on the
/// language holding the checked track, else the first.
pub fn other_languages_page(cx: &TrackCx<'_>, other: &[OtherLang]) -> TrackPageOf {
    let checked = cx.checked();
    let sec = other.iter().fold(FormSection::new(""), |sec, lang| match lang.tracks.as_slice() {
        [t] => sec.item(TrackRow::Sub(t.i), RowKind::Choice, (), flat_row(t, checked)),
        tracks => {
            let current = tracks.iter().find(|t| checked == Some(t.i));
            let value = match current {
                Some(t) => in_lang_label(t),
                None => plx_os::i18n::msg::widgets_tracks_count(tracks.len() as i64),
            };
            let row = Row::new(lang.name.clone()).value(value).checked(current.is_some());
            sec.item(TrackRow::OpenLang(lang.id), RowKind::Nav(TrackPage::Language(lang.id.stream)), (), row)
        }
    });
    // a language changes shape under a viewer (a sidecar becomes offered or leaves): the page above it was opened
    // from the row it WAS, and a pop lands on the row that stands for it now
    let form = other.iter().fold(Form::new().section(sec), |form, lang| {
        let is = other_lang_row(lang);
        let was = std::iter::once(TrackRow::OpenLang(lang.id)).chain(lang.tracks.iter().map(|t| TrackRow::Sub(t.i)));
        was.filter(|was| *was != is).fold(form, |form, was| form.alias(was, is))
    });
    let holds_checked = |o: &&OtherLang| checked.is_some_and(|c| o.tracks.iter().any(|t| t.i == c));
    let initial = other.iter().find(holds_checked).or(other.first()).map(other_lang_row);
    Page::new(form, plx_os::i18n::msg::widgets_tracks_other_languages()).initial(initial)
}

/// **A language page**: the tracks of the language `stream` names ([`LangId::stream`]), ranked full < SDH < forced <
/// commentary, a pick row each, titled by the language. A language that is gone lists nothing (the host pops a
/// page whose language left). Opens on the checked variant if it is inside, else the first track.
pub fn language_page(cx: &TrackCx<'_>, other: &[OtherLang], stream: i64) -> TrackPageOf {
    let checked = cx.checked();
    let lang = other.iter().find(|o| o.id.stream == stream);
    let tracks = lang.map(|o| o.tracks.as_slice()).unwrap_or_default();
    let sec = tracks.iter().fold(FormSection::new(""), |sec, t| {
        sec.item(TrackRow::Sub(t.i), RowKind::Choice, (), in_lang_row(t, checked))
    });
    let initial = tracks.iter().find(|t| checked == Some(t.i)).or(tracks.first()).map(|t| TrackRow::Sub(t.i));
    Page::new(Form::new().section(sec), lang.map(|o| o.name.clone()).unwrap_or_default()).initial(initial)
}

// ---- the rows ----------------------------------------------------------------------------------

/// **One Audio row**: the language (with "Original" on the file's default), the per-track descriptor so sibling
/// tracks in one language are distinguishable (the stream title, else the codec + channel layout) and the AD
/// badge. `sub_line` is the only part that depends on a route ([`audio_line`]).
pub(crate) fn audio_track_row(
    audio: &[Stream],
    i: usize,
    checked: bool,
    names: &TrackNames,
    sub_line: impl FnOnce(String) -> String,
) -> Row {
    let s = &audio[i];
    let lang = if s.lang.is_empty() { plx_os::i18n::msg::widgets_tracks_unknown() } else { s.lang.as_str() };
    let label = if s.default { plx_os::i18n::msg::widgets_tracks_original(lang) } else { lang.to_string() };
    let mut row = Row::new(label).checked(checked);
    let name = track_label::track_name(&s.title, names.audio(plx_data::metadata::audio_ordinal(audio, i)), lang);
    let sub = sub_line(if name.is_empty() { audio_descriptor(s) } else { name });
    if !sub.is_empty() {
        row = row.detail(sub);
    }
    if s.ad {
        row = row.badge(Badge::Ad);
    }
    row
}

/// An audio row's sub-line for the route's `mark`: the descriptor, or the descriptor with what the route does to
/// the track. A surround track the APP decodes says it plays in stereo, since the descriptor ("DTS 5.1") would
/// otherwise promise a layout the downmix does not keep; the server's mark outranks it. Always ONE line.
pub(crate) fn audio_line(sub: String, mark: AudioMark) -> String {
    if sub.is_empty() {
        sub
    } else {
        match mark {
            AudioMark::Converted => plx_os::i18n::msg::widgets_tracks_converted_by_server(&sub),
            AudioMark::InAppStereo => plx_os::i18n::msg::widgets_tracks_decoded_in_app_stereo(&sub),
            AudioMark::Plain => sub,
        }
    }
}

/// "AC-3 5.1", "Dolby TrueHD 7.1", "DTS 5.1" — a compact codec + channel-layout descriptor.
fn audio_descriptor(s: &Stream) -> String {
    let codec = friendly_codec(&s.codec);
    let ch = if !s.layout.is_empty() {
        channel_short(&s.layout)
    } else if s.channels > 0 {
        match s.channels {
            1 => plx_os::i18n::msg::widgets_tracks_mono().to_string(),
            2 => plx_os::i18n::msg::widgets_tracks_stereo().to_string(),
            n => format!("{}.{}", n - 1, if n >= 6 { 1 } else { 0 }),
        }
    } else {
        String::new()
    };
    match (codec.is_empty(), ch.is_empty()) {
        (false, false) => format!("{codec} {ch}"),
        (false, true) => codec,
        (true, false) => ch,
        _ => String::new(),
    }
}

/// map a Plex audioChannelLayout ("5.1(side)", "7.1") to a short "5.1"/"7.1"/"Stereo"
fn channel_short(layout: &str) -> String {
    let base = layout.split('(').next().unwrap_or(layout).trim();
    match base {
        "mono" => plx_os::i18n::msg::widgets_tracks_mono().to_string(),
        "stereo" => plx_os::i18n::msg::widgets_tracks_stereo().to_string(),
        other => other.to_string(),
    }
}

/// The drawn form of a row's one badge.
fn row_badge(b: &RowBadge) -> Badge {
    match b {
        RowBadge::Forced => Badge::Forced,
        RowBadge::Sdh => Badge::Sdh,
        RowBadge::External => Badge::Text(plx_os::i18n::msg::widgets_tracks_external_badge().to_string()),
        RowBadge::Codec(c) => Badge::Text((*c).to_string()),
    }
}

/// A flat row: label = language, detail = source/region (+ "Track N" when this track needed one), one badge.
/// Used for a single-track "yours" language, and for every "Other languages" row.
pub(crate) fn flat_row(t: &SubTrack, checked: Option<usize>) -> Row {
    let mut row = Row::new(t.lang.clone()).checked(checked == Some(t.i));
    let mut parts: Vec<String> = Vec::new();
    if !t.detail.is_empty() {
        parts.push(t.detail.clone());
    }
    if let Some(n) = t.ordinal {
        parts.push(plx_os::i18n::msg::widgets_tracks_track_ordinal(n as i64));
    }
    if !parts.is_empty() {
        row = row.detail(parts.join(" \u{b7} "));
    }
    for b in &t.badges {
        row = row.badge(row_badge(b));
    }
    row
}

/// A row inside a several-track language's section (`player.html:1110-1115`): label = the source (+ "Track N" if
/// needed), or — for a NAMELESS track — the kind word itself ("Forced", "SDH", "Full", "Commentary"), in which
/// case a Forced/SDH badge that would only repeat the label is dropped.
pub(crate) fn in_lang_row(t: &SubTrack, checked: Option<usize>) -> Row {
    let mut row = Row::new(in_lang_label(t)).checked(checked == Some(t.i));
    // a Forced/SDH chip that would only repeat the kind word the label already is, is dropped; the image codec
    // never is
    let label_is_kind = t.detail.is_empty() && t.ordinal.is_none();
    for b in &t.badges {
        if !(label_is_kind && matches!(b, RowBadge::Forced | RowBadge::Sdh)) {
            row = row.badge(row_badge(b));
        }
    }
    row
}

/// A track's label inside its language: the source (+ "Track N" if needed), or the kind word for a nameless one.
/// Also the value a language's drill-in reads out for its checked variant.
fn in_lang_label(t: &SubTrack) -> String {
    let nth = t.ordinal.map(|n| plx_os::i18n::msg::widgets_tracks_track_ordinal(n as i64));
    if !t.detail.is_empty() {
        match &nth {
            Some(n) => format!("{} \u{b7} {}", t.detail, n),
            None => t.detail.clone(),
        }
    } else if let Some(n) = &nth {
        if t.kind == Kind::Full {
            n.clone()
        } else {
            format!("{} \u{b7} {}", t.kind.fallback_label(), n)
        }
    } else {
        t.kind.fallback_label().to_string()
    }
}
