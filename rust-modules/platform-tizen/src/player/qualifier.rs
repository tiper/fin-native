//! What the Tizen platform certifies for the planner: [`TizenQualifier`], the production twin of the test-only
//! `SelfCertified` of the route's W6c composition tests.
//!
//! The driver is the platform's only path and says what it plays, so it certifies itself: no viewer opt-in is part
//! of the answer (`ResolvePort::qualify` hands the same evidence to everyone, [`super::compose`] builds it from
//! the driver's own capabilities and display). Four rules, each from the contract's own words:
//!
//! - **An original is certified only where Standard would direct-play it** (`OriginalRequest::standard`): the plan's
//!   own verdicts, never re-derived here. That is what keeps DTS and TrueHD out (the planner's audio table has no
//!   row for them on this platform) and every container Standard hands to the server to remux. The codec table
//!   behind those verdicts is `Caps::assumed()` today, not a Tizen measurement (`docs/tizen.md`).
//! - **The selected audio stream is certified wherever it sits** (discussion #1460: the first retail set refused every
//!   multi-audio title while this rule allowed only one stream). The offer's track map names it at its position among
//!   the Part's audio streams (`audio_ordinal`), and states its identity check: [`NativeTrackCheck::Sole`] (settled by construction, no host call) for a Part
//!   with one audio stream, otherwise the stream's language ([`NativeTrackCheck::Language`]), which the driver matches
//!   against the native tracks' own languages before it asks the host player for an index, and settles on the player's
//!   answer ([`super::machine::CAPS`]). A play that selects a stream whose position is unknown is declined
//!   ([`UriDecline::NotCertified`]): there is no index to ask for. UNMEASURED on a retail set: the native order
//!   (`docs/tizen.md`, "The audio track is selected").
//! - **A converted PMS output is never certified** ([`PmsVerdict::Unknown`]): certifying a remux or encode needs
//!   evidence the player lacks (HLS, a source time mapping), so a source Standard would not direct-play is the
//!   planner's typed `NoQualifiedBackend` refusal with the PMS decline named, not a conversion.
use plx_media::route::uri_preflight::{KnownSource, OriginalQualifier, OriginalRequest, PmsRequest, PmsVerdict, UriDecline,
    Verdict, SOURCE_VERDICT_TTL};
use plx_base::task::OffFrame;
use plx_playback_contract::uri::NativeTrackCheck;
use plx_playback_contract::{TrackKind, TrackSelection};

pub struct TizenQualifier;

impl OriginalQualifier for TizenQualifier {
    fn qualify(&self, _: &OffFrame, request: OriginalRequest) -> Verdict {
        if let Some(gap) = request.standard.first_gap() {
            return Verdict::Unknown(gap);
        }
        // `audio_ordinal` is the selected stream's position among the Part's audio streams: the native index the driver
        // asks the host player for (`player::machine::CAPS`). Without it there is nothing to ask.
        if request.selection.audio_sid > 0 && request.audio_ordinal.map_or(true, |ordinal| ordinal < 0) {
            return Verdict::Unknown(UriDecline::NotCertified);
        }
        let url = request.source_url.clone();
        let mut known = KnownSource::url_carrier(&request, url, SOURCE_VERDICT_TTL);
        // The native player's track order is unproven (LG's service listed a four-track file in another order than
        // PMS), so the driver may trust the ordinal only on the evidence this states for it.
        if let Ok(id) = u64::try_from(request.selection.audio_sid) {
            if id > 0 {
                let check = if request.has_multiple_audio_tracks {
                    NativeTrackCheck::Language(request.audio_language.clone())
                } else {
                    NativeTrackCheck::Sole
                };
                known.source = known.source.with_check(TrackSelection { kind: TrackKind::Audio, id }, check);
            }
        }
        Verdict::Known(known)
    }

    fn qualify_pms(&self, _: &OffFrame, _: PmsRequest) -> PmsVerdict {
        PmsVerdict::Unknown
    }

    /// Still false although the selected audio stream is now certified wherever it sits: this one flag also gates the
    /// Quality ladder (`PlaybackSession::quality_pick_carriable`), whose rungs ask the server to convert, and no converted
    /// output is certified here, so a true answer would let a Quality pick retire the playing title and end in
    /// `NoQualifiedBackend`. A mid-play audio pick is therefore refused by the session, as before; the track is chosen
    /// before the play starts.
    fn carries_source_track_change(&self) -> bool {
        false
    }
}
