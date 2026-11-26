//! The pure data a URI-mode source and its display path carry between the route owner and the
//! URI driver. Route policy qualifies a source and the application stages it into a driver; neither
//! names the driver's crate, so the shapes live here and the driver re-exports them. How a driver
//! binds a picture to its display (a plane, a window, a canvas) is the driver's own detail: the
//! contract carries only what the driver says of it ([`DisplayEvidence`]).

use crate::{AttemptId, DriverSet, Failure, Requirements, SourceKey, TrackChoice, TrackKind,
    TrackSelection};

/// Whether a driver's display path can show a URI source at all. It says nothing of what may be
/// offered through the path (certification does) and nothing of the Start (the driver's own gate,
/// asked on the main thread through [`UriDriver::ready_to_start`], does).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayReadiness {
    Ready,
    /// The driver has no display path to show a source on, for the reason given
    /// ([`UriNotReady::DisplayUnavailable`] when it has no more to say).
    NotReady(UriNotReady),
}

/// What a URI driver reports of its own display path, beside the capabilities and certification its
/// offers carry. The route reads only this value, captured once when the composition installs the
/// driver: which window or plane the driver binds to, and how it knows the firmware can, are the
/// driver's own detail and never a shared one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplayEvidence {
    pub readiness: DisplayReadiness,
    /// Whether the driver can place a trailer preview's rectangle.
    pub preview_rectangle: bool,
    /// The driver's own name for the display path. It is opaque: printed in the boot log for a
    /// person to read, and branched on by no shared code.
    pub tag: &'static str,
}

impl DisplayEvidence {
    /// A display path that can show a source, with no preview rectangle.
    pub const fn ready(tag: &'static str) -> Self {
        Self { readiness: DisplayReadiness::Ready, preview_rectangle: false, tag }
    }

    /// No display path: nothing is offered through it.
    pub const fn unavailable(tag: &'static str) -> Self {
        Self { readiness: DisplayReadiness::NotReady(UriNotReady::DisplayUnavailable),
            preview_rectangle: false, tag }
    }

    pub const fn is_ready(self) -> bool { matches!(self.readiness, DisplayReadiness::Ready) }
}

/// Explicit translation from a neutral selected track to this source's URI-native index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeTrackIndex { pub selection: TrackSelection, pub index: i64 }

/// The native index of `selected`, only when exactly one mapping names it and the index is valid.
pub fn native_index(tracks: &[NativeTrackIndex], selected: TrackSelection) -> Option<i64> {
    let mut matches = tracks.iter().filter(|track| track.selection == selected);
    let first = matches.next()?;
    (first.index >= 0 && matches.next().is_none()).then_some(first.index)
}

/// The authorized URL stays opaque; callers provide the exact native track map with it.
pub struct UriSource { url: String, tracks: Vec<NativeTrackIndex> }
impl UriSource {
    pub fn new(url: String, tracks: Vec<NativeTrackIndex>) -> Self { Self { url, tracks } }

    pub fn has_url(&self) -> bool { !self.url.is_empty() }

    /// The locator and its track map, handed to the driver that executes the source.
    pub fn into_parts(self) -> (String, Vec<NativeTrackIndex>) { (self.url, self.tracks) }

    pub fn valid_track_map(&self) -> bool {
        !self.tracks.iter().enumerate().any(|(i, track)| track.index < 0
            || self.tracks[..i].iter().any(|prior| prior.selection == track.selection
                || (prior.selection.kind == track.selection.kind && prior.index == track.index)))
    }

    /// Whether this owned source can execute its required native track choices. The same
    /// check guards qualification and the physical Start; no URL or index map escapes.
    ///
    /// `Off` names no track, so it needs nothing of the source's map: whether a play can be carried
    /// with a track off is the DRIVER's capability ([`Capabilities::supports_track`]), which every
    /// URI driver checks before this (a pipeline that shows a default subtitle by itself states
    /// `subtitle_off: false` and refuses it there).
    ///
    /// [`Capabilities::supports_track`]: crate::Capabilities::supports_track
    pub fn can_supply(&self, requirements: Requirements) -> bool {
        let mapped = |choice, kind| match choice {
            TrackChoice::Unspecified | TrackChoice::Off => true,
            TrackChoice::Selected(id) => native_index(&self.tracks, TrackSelection { kind, id }).is_some(),
        };
        self.has_url() && self.valid_track_map()
            && mapped(requirements.audio_track, TrackKind::Audio)
            && mapped(requirements.subtitle_track, TrackKind::Subtitle)
    }
}

/// What the player adapter needs of a URI driver beyond [`DriverSet`]: the escrow of an
/// authorized source, and two ownership facts the route reads. The adapter holds one as
/// `Box<dyn UriDriver<Context = MainThread>>`, so the product driver (`plx_play_uri`) and a
/// test fake are interchangeable and the player crate names neither.
pub trait UriDriver: DriverSet {
    /// What this driver reports of its display path, asked once by the composition that installs it
    /// and handed to the route as immutable evidence. It is the display half of the driver's
    /// evidence, beside the capabilities its offers carry and the certification the product grants;
    /// whether a Start can be taken now is [`ready_to_start`](Self::ready_to_start)'s question.
    fn display(&self) -> DisplayEvidence;
    /// Give exactly one authorized, owned URL to the next matching Start. Superseding a staged
    /// source destroys its old locator; a failed, cancelled or expired Start cannot reuse it.
    fn stage_source(&mut self, attempt: AttemptId, key: SourceKey, source: UriSource)
        -> Result<(), Failure>;
    /// Cancel an unconsumed locator without changing a live native attempt.
    fn cancel_staged(&mut self, attempt: AttemptId);
    /// The plane remains owned after the service reports Playing until native release. This is
    /// an ownership fact, not proof that a picture reached the panel.
    fn plane_owned(&self) -> bool;
    /// Whether the active source has an exact native index for this selected source track.
    fn maps_track(&self, selection: TrackSelection) -> bool;
    /// Whether this driver can take a Start now, asked on the main thread when a qualified source is
    /// OFFERED, before the route commits to the URI engine: a driver that has something to set up
    /// first (the ACB display's own handle, made once) does it here, so a setup that fails declines
    /// the offer and the play stays with Feed instead of failing at its Start. The Start re-checks
    /// whatever it needs. A driver with nothing to set up is always ready.
    fn ready_to_start(&mut self, _ctx: &Self::Context) -> Result<(), UriNotReady> { Ok(()) }
}

/// Why a [`UriDriver`] cannot take a Start ([`UriDriver::ready_to_start`]), or has no display path
/// ([`DisplayReadiness::NotReady`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UriNotReady {
    /// The driver has no display path to show a source on.
    DisplayUnavailable,
    /// The driver's own reason, in its own words: the text is what the offer's decline line says.
    Driver(&'static str),
}

impl UriNotReady {
    /// The reason as `uri: original declined (<reason>)` says it.
    pub fn log_name(self) -> &'static str {
        match self {
            Self::DisplayUnavailable => "the driver has no display path",
            Self::Driver(reason) => reason,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Capabilities, Fidelity, SourceKind};

    fn wants(audio_track: TrackChoice, subtitle_track: TrackChoice) -> Requirements {
        Requirements { source: SourceKind::ProgressiveUri, video: Fidelity::Exact, audio: Fidelity::Exact,
            audio_track, subtitle_track }
    }

    /// Off names no track, so a URL source supplies it with an empty map; whether it is carried is the driver's
    /// capability, which refuses it where the pipeline cannot hold a track off.
    #[test]
    fn off_needs_no_native_track_and_is_the_drivers_to_refuse() {
        let source = UriSource::new("http://pms/file".into(), vec![]);
        let off = wants(TrackChoice::Unspecified, TrackChoice::Off);
        assert!(source.can_supply(off));
        assert!(source.can_supply(wants(TrackChoice::Off, TrackChoice::Unspecified)));
        assert!(!source.can_supply(wants(TrackChoice::Unspecified, TrackChoice::Selected(7))),
            "a selected track still needs its native index");
        assert!(!UriSource::new(String::new(), vec![]).can_supply(off), "no URL, no source");
        let no_off = Capabilities { progressive_uri: true, hls_uri: false, packet_feed: false, pause: true, seek: true,
            audio_track: true, subtitle_track: true, audio_off: false, subtitle_off: false };
        assert!(!no_off.supports(off));
        assert!(Capabilities { subtitle_off: true, ..no_off }.supports(off));
    }
}
