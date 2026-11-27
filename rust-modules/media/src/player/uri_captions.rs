//! **A URI driver's captions, into the app's subtitle renderer.** A driver whose native player draws no subtitle
//! hands the selected track's cues over instead ([`UriDriver::delivers_captions`]); the adapter drains them after each
//! poll and this puts them where the app's renderer reads an embedded text track: the shared caption store
//! ([`super::ass_source`]), at the render ordinal the session says the app draws ([`embedded_subtitle_drawn_by_app`]).
//! From there the subtitle is drawn exactly as a Feed play's embedded text track is, with the viewer's Style and Timing.
//!
//! The cues arrive already stripped of the source's tags (the native player removes SubRip's `<b>`/`<i>` and ASS override
//! blocks), so the track is written in the plain dialect: the shared converter's common floor, under which a newline
//! breaks the line, exactly as for an embedded text track on a Feed play (a literal `{...}` left in a cue is read as an
//! override block there too).
//!
//! [`UriDriver::delivers_captions`]: plx_playback_contract::uri::UriDriver::delivers_captions
//! [`embedded_subtitle_drawn_by_app`]: crate::route::embedded_subtitle_drawn_by_app
use super::ass_source::{self, TrackHeader};
use super::text_ass::TextDialect;
use plx_playback_contract::subtitle::CaptionRetention;
use plx_playback_contract::uri::UriCaption;
use plx_playback_contract::AttemptId;

/// Which track the caption store was begun for: one per attempt, stream and ordinal, so a new play, a pick or a moved
/// ordinal starts a fresh source and the old track's cues are never drawn as the new one's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Begun {
    attempt: AttemptId,
    sid: i64,
    ordinal: i32,
    generation: u64,
}

/// The adapter's half of the caption path: the store generation its cues are pushed under.
#[derive(Default)]
pub(crate) struct UriCaptionSink {
    begun: Option<Begun>,
}

impl UriCaptionSink {
    /// Forget the track: the next caption begins a fresh source.
    pub(crate) fn clear(&mut self) {
        self.begun = None;
    }

    /// Push the captions of `attempt` (the live URI attempt) for the subtitle the app draws, `drawn` as
    /// `(stream id, render ordinal)`; a caption of another attempt or another stream is dropped. `playhead_ns` is the
    /// retention anchor the store retires old cues against. Returns how many were pushed.
    pub(crate) fn deliver(&mut self, attempt: AttemptId, drawn: Option<(i64, i32)>, playhead_ns: Option<i64>,
        captions: &[UriCaption]) -> usize {
        let Some((sid, ordinal)) = drawn else { return 0 };
        let mut pushed = 0;
        for caption in captions.iter().filter(|caption| caption.attempt == attempt
            && i64::try_from(caption.track.id) == Ok(sid)) {
            let generation = match self.begun {
                Some(begun) if begun == (Begun { attempt, sid, ordinal, generation: begun.generation }) => begun.generation,
                _ => {
                    let key = format!("uri-captions:{}:{}:{sid}", attempt.owner(), attempt.generation());
                    let generation = ass_source::begin(&key, vec![(ordinal, TrackHeader::Text(TextDialect::Plain))],
                        Vec::new());
                    self.begun = Some(Begun { attempt, sid, ordinal, generation });
                    generation
                }
            };
            let start_ms = i64::try_from(caption.start.as_millis()).unwrap_or(i64::MAX);
            let end_ms = start_ms.saturating_add(i64::try_from(caption.duration_ms).unwrap_or(i64::MAX));
            let identity = ass_source::packet_identity(caption.text.as_bytes());
            ass_source::push_text(generation, ordinal, start_ms.saturating_mul(1_000_000), end_ms.saturating_mul(1_000_000),
                &caption.text, identity, CaptionRetention { playhead_ns, seek_target_ns: None });
            pushed += 1;
        }
        pushed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::ass::Content;
    use plx_playback_contract::{ContentTime, TrackKind, TrackSelection};

    fn caption(attempt: AttemptId, sid: u64, start_ms: u64, text: &str) -> UriCaption {
        UriCaption { attempt, track: TrackSelection { kind: TrackKind::Subtitle, id: sid },
            start: ContentTime::from_millis(start_ms), duration_ms: 1_500, text: text.to_owned() }
    }

    fn shown(ordinal: i32) -> Vec<(i64, i64, String)> {
        let Some(source) = ass_source::selected(ordinal) else { return Vec::new() };
        let Content::Embedded { events, .. } = &source.content else { panic!("an embedded text track") };
        events.iter().map(|e| (e.start_ms, e.duration_ms, String::from_utf8_lossy(&e.payload).into_owned())).collect()
    }

    /// **A URI driver's captions reach the renderer's source at the ordinal the app draws** (the Tizen URI driver hands
    /// its host player's cues over: `UriDriver::delivers_captions`). Only the live attempt's captions of the drawn stream
    /// are kept; a pick (another stream or ordinal) begins a fresh source, so the old track's cues are never drawn as the
    /// new one's; with nothing drawn, nothing is pushed.
    #[test]
    fn captions_of_the_drawn_stream_reach_the_renderer_at_its_ordinal() {
        let _guard = plx_base::testlock::serial();
        ass_source::reset();
        let live = AttemptId::new(1, 7);
        let mut sink = UriCaptionSink::default();
        crate::player::request_subtitle(1);
        let batch = [caption(live, 12, 2_003, "TWO 01 top\nTWO 01 bottom"), caption(live, 11, 2_003, "another stream"),
            caption(AttemptId::new(1, 6), 12, 2_003, "an older attempt")];
        assert_eq!(sink.deliver(live, Some((12, 1)), Some(2_000_000_000), &batch), 1);
        let events = shown(1);
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!((events[0].0, events[0].1), (2_003, 1_500));
        assert!(events[0].2.ends_with(r"TWO 01 top\NTWO 01 bottom"), "the newline is a line break: {:?}", events[0].2);
        // The same cue again (the player re-raised it) is one event.
        assert_eq!(sink.deliver(live, Some((12, 1)), Some(2_100_000_000), &batch[..1]), 1);
        assert_eq!(shown(1).len(), 1);

        // A pick of the stream at ordinal 0: a fresh source, the old cues gone.
        crate::player::request_subtitle(0);
        assert_eq!(sink.deliver(live, Some((11, 0)), Some(4_000_000_000), &[caption(live, 11, 4_001, "FRENCH 02")]), 1);
        let events = shown(0);
        assert_eq!(events.len(), 1, "{events:?}");
        assert!(events[0].2.ends_with("FRENCH 02"), "{:?}", events[0].2);
        assert!(shown(1).is_empty(), "the old track's source was replaced");

        // Nothing drawn (Off): nothing pushed.
        assert_eq!(sink.deliver(live, None, Some(6_000_000_000), &[caption(live, 11, 6_001, "after off")]), 0);
        ass_source::reset();
        crate::player::request_subtitle(-1);
    }
}
