//! **The server's selection: what PMS holds as one Part's selected audio and subtitle streams.**
//!
//! A track pick during playback that needs a server conversion is written to the Part's selection
//! BEFORE `/decision` answers (the transcoder encodes what is SELECTED, so the write cannot wait,
//! `plex::library::select_streams`). A server that then refuses the conversion leaves the refused
//! pick as its selection, and the next start of that title plans from it. Undoing the write needs to
//! know what the server held before it. No other value says: `source_selection` is the pre-sibling
//! SOURCE intent and is overwritten at the pick's commit, `cur_*` and the applied projection are what
//! the app shows (a fall-through auto-pick is shown and never PUT, `send_parked_write_back`, W2).
//!
//! **One value, one writer.** [`PlaybackSession::server_selection`](super::PlaybackSession) is
//! written by exactly these steps, all of them naming a PUT the server answered (or, for the serial
//! queue, accepted into its FIFO):
//!   * the landing's own start, from the fetched item's `selected` streams as the resolve read them
//!     BEFORE any local pick rewrote the flags (`Plan::selection_fetched`, [`ServerSelection::fetched`];
//!     an item taken from the detail page's cache carries the flags of the fetch that produced it)
//!     then the plan's PUTs ([`ServerSelection::after`]);
//!   * a claim flight's landing ([`FlightSelection::settle`]), for the PUTs its worker sent;
//!   * a queued selection ([`ServerSelection::after`] at the queue site; the serial worker has no way
//!     back to its owner, so a PUT it fails stays recorded: the value then claims a pair the server
//!     does not hold, and a restore to it is one more write of the pair on screen).
//!
//! A restore ([`ServerSelection::restore_of`]) names only the fields the refused PUT changed and only
//! those this value knows: where the server's earlier value is unknown, nothing is sent for it. A
//! `None` audio is unknown (the wire drops `audioStreamID` for it); a `None` subtitle is unknown and
//! `Some(0)` is a known Off.

use super::decision::{queue_selection, PlaybackSession};
use plx_plex::plex::ServerId;

/// One Part's selected streams on the server. `audio`/`subtitle` are stream ids; `None` is unknown
/// (never sent), `Some(0)` for the subtitle is Off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ServerSelection {
    pub(super) part_id: i64,
    pub(super) audio: Option<i64>,
    pub(super) subtitle: Option<i64>,
}

impl ServerSelection {
    /// What a selection PUT of `audio` (0 = not sent) and `subtitle` to `part_id` names.
    pub(super) fn put(part_id: i64, audio: i64, subtitle: Option<i64>) -> Self {
        Self { part_id, audio: (audio > 0).then_some(audio), subtitle }
    }

    /// The server's selection of `part_id` as the item was fetched: its `selected` streams. A part
    /// with subtitle streams and none selected is Off (`Some(0)`), one without any has nothing to
    /// know. `None` for a part the plan names no id for.
    pub(super) fn fetched(part_id: i64, item: &plx_data::metadata::PlayingItem) -> Option<Self> {
        if part_id <= 0 {
            return None;
        }
        Some(Self {
            part_id,
            audio: item.audio.iter().find(|s| s.selected && s.id > 0).map(|s| s.id),
            subtitle: if item.subs.is_empty() {
                None
            } else {
                Some(item.subs.iter().find(|s| s.selected && s.id > 0).map_or(0, |s| s.id))
            },
        })
    }

    /// The selection after `put` was written over `before`: the put's fields over the earlier ones
    /// when it is the same Part, and only the put's own when it is another (the rest is unknown).
    pub(super) fn after(before: Option<Self>, put: Self) -> Self {
        match before.filter(|b| b.part_id == put.part_id) {
            Some(b) => Self { part_id: put.part_id, audio: put.audio.or(b.audio), subtitle: put.subtitle.or(b.subtitle) },
            None => put,
        }
    }

    /// The write that puts the server back to `self` for the fields `sent` changed, or `None` when
    /// there is nothing to say: another Part, a field the PUT did not change, or one `self` does not
    /// know. The result has the shape of a PUT (`audio` 0 and `subtitle` `None` are omitted).
    pub(super) fn restore_of(self, sent: &Self) -> Option<(i64, Option<i64>)> {
        if self.part_id != sent.part_id {
            return None;
        }
        let audio = sent.audio.filter(|a| Some(*a) != self.audio).and(self.audio);
        let subtitle = sent.subtitle.filter(|s| Some(*s) != self.subtitle).and(self.subtitle);
        (audio.is_some() || subtitle.is_some()).then_some((audio.unwrap_or(0), subtitle))
    }
}

/// The PUTs one claim flight's worker sent, merged, and what the server held before them: the
/// worker's whole report about the selection, posted with its landing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct FlightSelection {
    pub(super) sid: ServerId,
    /// [`PlaybackSession::server_selection`] as the claim captured it, on the main thread.
    pub(super) before: Option<ServerSelection>,
    /// The merged PUTs the server answered, `None` when the flight sent none.
    pub(super) sent: Option<ServerSelection>,
}

/// Whether a field was written by someone else since the claim captured it: the value is known, differs
/// from what the claim captured (`before`), and is not the claim's own (`sent`): a value equal to the
/// claim's own is not a newer word, it is the same pick.
fn written_since(before: Option<i64>, sent: Option<i64>, current: Option<i64>) -> bool {
    current.is_some() && current != before && current != sent
}

impl FlightSelection {
    /// Take a claim's landing into the one value, field by field. An ACCEPTED claim moves the server's
    /// selection to what its PUTs wrote; a REFUSED one writes the earlier selection back (through the
    /// serial queue, so a PUT queued since is ordered with it).
    ///
    /// A field the main thread has written since the claim captured the selection (an in-place pick
    /// queued its own PUT while the flight flew) is the newer word and is left as it is: the claim's
    /// value for it is neither restored nor recorded. The other field is settled as usual, so a refused
    /// audio rebuild is still undone under an in-place subtitle pick, whose PUT names the subtitle only.
    pub(super) fn settle(&self, ps: &mut PlaybackSession, accepted: bool) {
        let Some(sent) = self.sent else { return };
        if ps.server_selection.is_some_and(|c| c.part_id != sent.part_id) {
            return;
        }
        let current = ps.server_selection;
        let (cur_audio, cur_sub) = current.map_or((None, None), |c| (c.audio, c.subtitle));
        let (before_audio, before_sub) = self.before.map_or((None, None), |b| (b.audio, b.subtitle));
        let audio_newer = written_since(before_audio, sent.audio, cur_audio);
        let sub_newer = written_since(before_sub, sent.subtitle, cur_sub);
        if accepted {
            ps.server_selection = Some(ServerSelection {
                part_id: sent.part_id,
                audio: if audio_newer { cur_audio } else { sent.audio.or(cur_audio) },
                subtitle: if sub_newer { cur_sub } else { sent.subtitle.or(cur_sub) },
            });
            return;
        }
        let owed = ServerSelection {
            part_id: sent.part_id,
            audio: sent.audio.filter(|_| !audio_newer),
            subtitle: sent.subtitle.filter(|_| !sub_newer),
        };
        if let Some((audio, subtitle)) = self.before.and_then(|before| before.restore_of(&owed)) {
            crate::player::log("select streams: restoring the selection a refused rebuild had written");
            if queue_selection(self.sid, sent.part_id, audio, subtitle, false) {
                ps.server_selection = Some(ServerSelection::after(current, ServerSelection::put(sent.part_id, audio, subtitle)));
            }
        }
    }
}

/// The PUTs a flight's worker has sent so far, merged ([`ServerSelection::after`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct PutTrail(Option<ServerSelection>);

impl PutTrail {
    /// Record `put` (what [`put_selection`] returns: `Some` for a PUT the server answered).
    pub(super) fn note(&mut self, put: Option<ServerSelection>) {
        if let Some(put) = put {
            self.0 = Some(ServerSelection::after(self.0, put));
        }
    }

    pub(super) fn sent(self) -> Option<ServerSelection> {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sel(audio: Option<i64>, subtitle: Option<i64>) -> ServerSelection {
        ServerSelection { part_id: 7, audio, subtitle }
    }

    #[test]
    fn a_restore_names_only_the_fields_the_put_changed_and_this_value_knows() {
        let before = sel(Some(10), Some(55));
        // A subtitle burn pick: the PUT repeats the audio and changes the subtitle.
        let burn = sel(Some(10), Some(88));
        assert_eq!(before.restore_of(&burn), Some((0, Some(55))), "only the subtitle changed, so only it goes back");
        // Off is a known value.
        assert_eq!(sel(Some(10), Some(0)).restore_of(&burn), Some((0, Some(0))));
        // An unknown subtitle is never guessed.
        assert_eq!(sel(Some(10), None).restore_of(&burn), None);
        // Audio unknown: the subtitle alone goes back, and an audio pick has nothing to say.
        assert_eq!(sel(None, Some(55)).restore_of(&sel(Some(11), Some(88))), Some((0, Some(55))));
        assert_eq!(sel(None, Some(55)).restore_of(&sel(Some(11), Some(55))), None);
        // An audio pick: the audio goes back, the unchanged subtitle is left alone.
        assert_eq!(before.restore_of(&sel(Some(11), Some(55))), Some((10, None)));
        // Another Part is not this value's to undo.
        assert_eq!(before.restore_of(&ServerSelection { part_id: 8, ..burn }), None);
    }

    #[test]
    fn the_fetched_selection_is_the_streams_the_server_marks_selected() {
        let stream = |id, selected| plx_data::metadata::Stream { id, selected, ..Default::default() };
        let item = plx_data::metadata::PlayingItem {
            audio: vec![stream(10, false), stream(11, true)],
            ..plx_data::metadata::PlayingItem::with_subs(vec![stream(55, false), stream(56, false)])
        };
        assert_eq!(ServerSelection::fetched(7, &item), Some(sel(Some(11), Some(0))), "no subtitle selected is a known Off");
        let none = plx_data::metadata::PlayingItem::with_subs(Vec::new());
        assert_eq!(ServerSelection::fetched(7, &none), Some(sel(None, None)), "a Part with no streams knows nothing");
        assert_eq!(ServerSelection::fetched(0, &item), None);
    }

    #[test]
    fn a_field_is_a_newer_word_only_when_it_is_neither_the_captured_nor_the_claims_value() {
        // before 10, the claim sent 11: 10 (untouched), 11 (an in-place PUT repeating the pending pick)
        // are not newer; 12 is; an unknown is not.
        assert!(!written_since(Some(10), Some(11), Some(10)));
        assert!(!written_since(Some(10), Some(11), Some(11)));
        assert!(written_since(Some(10), Some(11), Some(12)));
        assert!(!written_since(Some(10), Some(11), None));
        // a field the claim did not send (None) is any known change
        assert!(written_since(Some(55), None, Some(0)));
    }

    #[test]
    fn a_put_over_the_same_part_keeps_what_it_did_not_name() {
        let before = Some(sel(Some(10), Some(55)));
        assert_eq!(ServerSelection::after(before, ServerSelection::put(7, 0, Some(88))), sel(Some(10), Some(88)));
        assert_eq!(ServerSelection::after(before, ServerSelection::put(8, 12, Some(0))), ServerSelection { part_id: 8, audio: Some(12), subtitle: Some(0) });
    }
}
