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
//! - **Only a source with ONE audio stream is certified** (`!has_multiple_audio_tracks`, the webOS precedent
//!   `ManyAudioTracksExported` in `app/adapters/backends.rs`). The host ABI has no track verb and nobody has measured
//!   which stream the retail player starts on when a source has several, so a multi-audio source is declined
//!   ([`UriDecline::NotCertified`]) instead of started on a stream that may not be the one the viewer is shown as
//!   selected. For the single stream the driver's track settle is by construction (nothing to choose between), not
//!   native evidence ([`super::machine::CAPS`]). A play that selects an audio stream that is not the Part's first is
//!   declined too.
//! - **A converted PMS output is never certified** ([`PmsVerdict::Unknown`]): certifying a remux or encode needs
//!   evidence the player lacks (HLS, a source time mapping), so a source Standard would not direct-play is the
//!   planner's typed `NoQualifiedBackend` refusal with the PMS decline named, not a conversion.
use plx_media::route::uri_preflight::{KnownSource, OriginalQualifier, OriginalRequest, PmsRequest, PmsVerdict, UriDecline,
    Verdict, SOURCE_VERDICT_TTL};
use plx_base::task::OffFrame;

pub struct TizenQualifier;

impl OriginalQualifier for TizenQualifier {
    fn qualify(&self, _: &OffFrame, request: OriginalRequest) -> Verdict {
        if let Some(gap) = request.standard.first_gap() {
            return Verdict::Unknown(gap);
        }
        // One audio stream or none: with several, which one the native player starts on is unmeasured.
        if request.has_multiple_audio_tracks {
            return Verdict::Unknown(UriDecline::NotCertified);
        }
        // `audio_ordinal` is the selected stream's position among the Part's audio streams; the machine settles the
        // request only for position 0 (`player::machine::CAPS`).
        if request.selection.audio_sid > 0 && request.audio_ordinal != Some(0) {
            return Verdict::Unknown(UriDecline::NotCertified);
        }
        let url = request.source_url.clone();
        Verdict::Known(KnownSource::url_carrier(&request, url, SOURCE_VERDICT_TTL))
    }

    fn qualify_pms(&self, _: &OffFrame, _: PmsRequest) -> PmsVerdict {
        PmsVerdict::Unknown
    }

    /// Only the source's default (single) audio stream is certified and no converted output is, so a viewer's pick of
    /// another track mid-play has nothing to land on: the session refuses it instead of replacing the playing title.
    fn carries_source_track_change(&self) -> bool {
        false
    }
}
