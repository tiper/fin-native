//! **FLAC and Opus fed to the television's own decoder**, with no conversion on the app's side.
//!
//! The Starfish pipeline names these beside AC3, E-AC3, AAC and DTS: `libpf`'s
//! `CustomPipeline::parseOptionStringSpi` reads the audio names `OPUS` and `FLAC` (read from libpf on a
//! C1, webOS 6.5.3, 2026-10-21; the codec table `device_codec_capability_config.json` lists FLAC at six
//! channels). What each needs besides its name is read off the same library's
//! `CustomPipeline::getAudioCaps`, which builds the GStreamer caps the decoder gets:
//!
//! - **Opus** (`audio/x-opus`): `contents.opusInfo` with `channels`, `sampleRate` and `streamHeader`
//!   (`player::engine::with_opus_info` builds it).
//! - **FLAC** (`audio/x-flac`): no info object, and the caps carry no field at all. The decoder is
//!   left to find the stream header in the data, so the audio lane starts with `fLaC` and the
//!   `STREAMINFO` block, as in a `.flac` file ([`flac_stream_header`]).
//!
//! The demuxer records what it is feeding natively ([`publish_fed`]) so the player can tell a native
//! audio refusal from any other failure ([`fed`]): that is what the learned-refusal path and the
//! surround stall watchdog (`player::native_stall`) hang on.
use std::sync::Mutex;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Flac,
    Opus,
}

impl Kind {
    /// From FFmpeg's codec name (`avcodec_get_name`), which is also how PMS spells them.
    pub(crate) fn from_ffmpeg_name(name: &str) -> Option<Kind> {
        match name {
            "flac" => Some(Kind::Flac),
            "opus" => Some(Kind::Opus),
            _ => None,
        }
    }

    /// The name the learned-refusal list and the exclusion rows spell the codec with.
    pub(crate) fn ffmpeg_name(self) -> &'static str {
        match self {
            Kind::Flac => "flac",
            Kind::Opus => "opus",
        }
    }
}

/// `fLaC` followed by the `STREAMINFO` metadata block, marked as the last one. FFmpeg's FLAC
/// extradata is the 34-byte `STREAMINFO` body, or the whole header when the container kept it.
/// `None` when the extradata is neither: the stream is refused rather than fed a header the decoder
/// cannot parse.
pub(crate) fn flac_stream_header(extradata: &[u8]) -> Option<Vec<u8>> {
    if extradata.starts_with(b"fLaC") {
        return (extradata.len() >= 4 + 4 + 34).then(|| extradata.to_vec());
    }
    if extradata.len() < 34 {
        return None;
    }
    let mut out = Vec::with_capacity(4 + 4 + 34);
    out.extend_from_slice(b"fLaC");
    // Last-metadata-block flag set, block type 0 (STREAMINFO), 24-bit length 34.
    out.extend_from_slice(&[0x80, 0x00, 0x00, 34]);
    out.extend_from_slice(&extradata[..34]);
    Some(out)
}

static FED: Mutex<Option<(Kind, u32)>> = Mutex::new(None);

/// A new session is starting: forget the last one's answer. Called on the main thread before the
/// demuxer is spawned, so no demuxer is alive to answer into the cell.
pub(crate) fn reset() {
    *FED.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// The demuxer feeds this session's audio natively: the codec and the track's channel count.
pub(crate) fn publish_fed(kind: Kind, channels: u32) {
    *FED.lock().unwrap_or_else(|e| e.into_inner()) = Some((kind, channels));
}

/// What this session feeds the pipeline natively, once the demuxer has said so. `None` before that
/// and for any track the TV decodes by name alone (AC3, AAC, DTS). The player reads it to tell a
/// native audio refusal from any other.
pub(crate) fn fed() -> Option<(Kind, u32)> {
    *FED.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_kinds_are_the_names_ffmpeg_gives() {
        assert_eq!(Kind::from_ffmpeg_name("flac"), Some(Kind::Flac));
        assert_eq!(Kind::from_ffmpeg_name("opus"), Some(Kind::Opus));
        for other in ["vorbis", "ac3", "", "FLAC"] {
            assert_eq!(Kind::from_ffmpeg_name(other), None, "{other:?}");
        }
        assert_eq!(Kind::Flac.ffmpeg_name(), "flac");
        assert_eq!(Kind::Opus.ffmpeg_name(), "opus");
    }

    /// FLAC is a stream header in the data and nothing in the payload: the 34-byte STREAMINFO body
    /// FFmpeg keeps is wrapped as `fLaC` + a last-block header; a container that kept the whole
    /// header passes as it is; anything shorter is refused.
    #[test]
    fn flac_is_a_stream_header_in_the_data() {
        let body: Vec<u8> = (0..34).collect();
        let h = flac_stream_header(&body).unwrap();
        assert_eq!(&h[..8], b"fLaC\x80\x00\x00\x22");
        assert_eq!(&h[8..], &body[..]);
        let whole: Vec<u8> = [b"fLaC".as_slice(), &[0x80, 0, 0, 34], &body].concat();
        assert_eq!(flac_stream_header(&whole).unwrap(), whole);
        assert_eq!(flac_stream_header(&body[..33]), None);
        assert_eq!(flac_stream_header(b"fLaC\x80\x00"), None);
        assert_eq!(flac_stream_header(&[]), None);
    }

    #[test]
    fn what_a_session_feeds_is_remembered_until_the_next_one_starts() {
        let _guard = plx_base::testlock::serial();
        reset();
        assert_eq!(fed(), None);
        publish_fed(Kind::Opus, 6);
        assert_eq!(fed(), Some((Kind::Opus, 6)));
        reset();
        assert_eq!(fed(), None);
    }
}
