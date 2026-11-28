//! What route policy asks of the player. Route decides (it names no backend and carries no backend
//! `cfg`); this module executes. The audio and speed picks step the session's readout, which is
//! backend-neutral, so those run in every build, as does the Original failure mark (the readout's).
//! The rest still write the Feed executor's process state (the claim hold, the side reader), which
//! only a composed Feed reads: each is a no-op when the build composes no Feed driver, and each
//! waits for its route caller to hold the adapter (route effects as typed effects a runner
//! executes, L4). Which backends a session may offer is not decided here: the application's boot
//! installs the set its drivers form (`PlayerAdapter::composed_backends`).

use crate::route::{PlaybackSession, SideReaderTarget};

/// Drop the claim hold: the playback it belonged to is gone.
pub fn clear_claim_hold() {
    super::claim_hold::clear();
}

/// Forget the typed failure of the Original probe a replacement source is leaving.
pub fn clear_original_failure(ps: &mut PlaybackSession) {
    ps.step_readout(super::readout::ReadoutEvent::OriginalFailure(
        plx_playback_contract::adaptive::FailureEdit::Cleared,
    ));
}

/// Select the audio stream index the demuxer feeds at the first Load (`-1`: the default).
pub fn set_audio_track(ps: &mut PlaybackSession, idx: i32) {
    super::set_audio_track(ps, idx);
}

/// Back to the default audio stream, for a new item.
pub fn reset_audio_track(ps: &mut PlaybackSession) {
    super::reset_audio_track(ps);
}

/// Back to 1x, for a new item: the session's readout drops the offered rate and a pending reload.
pub fn reset_play_rate(ps: &mut PlaybackSession) {
    super::reset_play_rate(ps);
}

/// Stage a native audio switch (the Load codec and the demuxer's stream index) without queueing
/// its reload.
pub fn stage_native_audio(ps: &mut PlaybackSession, audio_idx: i32, codec: &str) {
    super::stage_native_audio(ps, audio_idx, codec);
}

/// Request a native audio-track switch: stage it and queue the direct-play reload.
pub fn request_audio_track(ps: &mut PlaybackSession, audio_idx: i32, codec: &str) {
    super::request_audio_track(ps, audio_idx, codec);
}

/// Is the side subtitle reader running (as of the feed's last sample)?
pub fn side_reader_active(ps: &PlaybackSession) -> bool {
    ps.readout().raw().side_reader_active
}

/// The track the dev trigger arms the side reader for, if any.
pub fn side_reader_dev_armed_ordinal(ps: &PlaybackSession) -> Option<i32> {
    if !ps.backends().feed { return None; }
    super::subside::dev_armed_ordinal()
}

/// Make the side reader match the route's target: start, switch, run clock-only, or stop for `None`.
/// The route only says what it wants (a readout event); the pump hands it to the feed on its next
/// frame (`FeedCommand::SyncSideReader`).
///
/// A reader begun mid-play starts where the live remux began in the film ([`remux_start_ns`]).
pub fn sync_side_reader(ps: &mut PlaybackSession, target: Option<SideReaderTarget>) {
    if !ps.backends().feed { return; }
    let want = target
        .and_then(|t| super::subside::spec_for(&t, remux_start_ns(ps.readout().raw())))
        .map(std::sync::Arc::new);
    ps.step_readout(super::readout::ReadoutEvent::SideReaderWanted(want));
}

/// Where the live remux began in the film, as the live sample carries it: its display base, or,
/// for a conversion that keeps the file's timestamps, the keyframe it landed on. That conversion is
/// re-anchored to display base 0, so its display base names no start, and a reader begun from it
/// scanned the Part from the film's start for a keyframe minutes later, matched none and moved
/// every cue by the whole offset (#49).
fn remux_start_ns(raw: &plx_playback_contract::feed::FeedRaw) -> i64 {
    raw.conversion_keyframe_ns.unwrap_or(raw.disp_base)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::readout::ReadoutEvent;
    use plx_playback_contract::adaptive::FailureEdit;
    use plx_playback_contract::feed::FeedRaw;

    /// A reader begun mid-play over a conversion that keeps the file's timestamps scans from the
    /// keyframe the conversion landed on (a reporter's resume at 66.5 s landed on 66.358 s, display
    /// base 0); a remux labelled by its display base keeps that.
    #[test]
    fn a_reader_begun_mid_play_starts_where_the_remux_began() {
        let anchored = FeedRaw { conversion_keyframe_ns: Some(66_358_000_000), disp_base: 0, ..FeedRaw::IDLE };
        assert_eq!(remux_start_ns(&anchored), 66_358_000_000);
        let labelled = FeedRaw { disp_base: 600_000_000_000, ..FeedRaw::IDLE };
        assert_eq!(remux_start_ns(&labelled), 600_000_000_000);
    }

    /// A replacement source leaving a failed Original probe forgets the reason, in the session's
    /// readout, in every build: it names no backend and holds no Feed state.
    #[test]
    fn a_replacement_source_clears_the_original_failure_in_the_readout() {
        let mut ps = PlaybackSession::IDLE;
        ps.step_readout(ReadoutEvent::OriginalFailure(FailureEdit::Noted {
            kind: crate::player::ABR_FAILURE_ORIGINAL_HTTP, status: 503,
        }));
        assert!(ps.readout().original_failure().is_some(), "PRECONDITION FAILED: the failure is noted");
        clear_original_failure(&mut ps);
        assert_eq!(ps.readout().original_failure(), None);
    }
}
