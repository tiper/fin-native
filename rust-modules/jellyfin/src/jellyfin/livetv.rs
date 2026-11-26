//! Live TV: Jellyfin's channels and guide in the model the Live TV screens read.
//!
//! The app's Live TV is organised as the sources a server offers (on Plex, a DVR each), the
//! channels of each and what airs now. A Jellyfin server has one channel list across its tuners
//! and M3U sources, so it is one source, [`SOURCE`]. A channel is an item of its own: tuning
//! asks the server to play it (`PlaybackInfo` with `autoOpenLiveStream`), which answers the
//! stream's id and an HLS playlist the player reads; closing the stream releases the tuner.

use super::dto::BaseItemDto;
use super::ids::normalize;
use super::paths;
use super::playback_info::Response;
use super::time::iso_to_unix;
use super::translate::primary_image;
use plx_plex::plex::livetv::{LivePlay, LiveSession};
use plx_plex::plex::{Dvr, EpgChannel, Media, Metadata};
use serde_json::{json, Value};

/// The one source a Jellyfin server offers, as the id both the lineup and the grid are read under.
pub const SOURCE: &str = "jf";

pub fn source() -> Dvr {
    Dvr { key: SOURCE.into(), epg_identifier: SOURCE.into(), ..Default::default() }
}

/// A channel of the lineup; one the server sent without an id is not tunable and is dropped.
pub fn channel_of(c: &BaseItemDto) -> Option<EpgChannel> {
    let id = normalize(&c.id);
    if id.is_empty() {
        return None;
    }
    let vcn = c.channel_number.clone().unwrap_or_default();
    let title = if vcn.is_empty() { c.name.clone() } else { format!("{vcn} {}", c.name) };
    Some(EpgChannel { grid_key: id, vcn, title, thumb: primary_image(c) })
}

/// What a channel airs now, as the guide's airing: its slot and the channel it is on.
pub fn airing_of(c: &BaseItemDto) -> Option<Metadata> {
    let p = c.current_program.as_deref()?;
    let at = |stamp: &Option<String>| stamp.as_deref().map_or(0, iso_to_unix);
    let series = p.series_name.clone().filter(|s| !s.is_empty());
    Some(Metadata {
        title: p.name.clone(),
        kind: if series.is_some() { "episode".into() } else { "movie".into() },
        grandparent_title: series.unwrap_or_default(),
        summary: p.overview.clone().unwrap_or_default(),
        thumb: primary_image(p),
        media: vec![Media {
            channel_identifier: normalize(&c.id),
            channel_vcn: c.channel_number.clone().unwrap_or_default(),
            begins_at: at(&p.start_date),
            ends_at: at(&p.end_date),
            ..Default::default()
        }],
        ..Default::default()
    })
}

/// What a live play arrives as: H.264 and AAC stereo. The profile allows nothing else, so a
/// source in other codecs is converted to them, and the route's declaration is these names rather
/// than whatever the server's URL happens to list.
pub const VIDEO_CODEC: &str = "h264";
pub const AUDIO_CODEC: &str = "aac";
const AUDIO_CHANNELS: i64 = 2;

/// The device profile for a live play: HLS in [`VIDEO_CODEC`] and [`AUDIO_CODEC`], nothing the
/// server could send as it is.
pub fn live_profile() -> Value {
    json!({
        "Name": "FinNative Live TV",
        "DirectPlayProfiles": [],
        "TranscodingProfiles": [
            {"Type": "Video", "Container": "ts", "Protocol": "hls", "Context": "Streaming",
             "VideoCodec": VIDEO_CODEC, "AudioCodec": AUDIO_CODEC, "MinSegments": 1, "BreakOnNonKeyFrames": true,
             "EnableSubtitlesInManifest": false}
        ],
        "CodecProfiles": [],
        "SubtitleProfiles": [],
        "ContainerProfiles": [],
        "ResponseProfiles": []
    })
}

/// A tuned channel: the stream to close and the play the route reads.
#[derive(Debug, PartialEq, Eq)]
pub struct Tuned {
    pub session: LiveSession,
    pub play: LivePlay,
}

/// The tune's answer: the first source with a playlist. The session key is the live stream id (what
/// closes it) and its rating key the channel. The playlist is kept as a path with neither the
/// server's token nor its play session: the route adds this client's own.
pub fn tuned(channel_id: &str, r: &Response) -> Option<Tuned> {
    let source = r.media_sources.iter().find(|s| s.transcoding_url.as_deref().is_some_and(|u| !u.is_empty()))?;
    let url = source.transcoding_url.as_deref()?;
    let part = paths::without_param(&paths::without_param(url, "api_key"), "PlaySessionId");
    Some(Tuned {
        session: LiveSession { key: source.live_stream_id.clone().unwrap_or_default(), rating_key: channel_id.to_owned() },
        play: LivePlay { part, vcodec: VIDEO_CODEC.into(), acodec: AUDIO_CODEC.into(), width: 0, height: 0, channels: AUDIO_CHANNELS },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channel(json: &str) -> BaseItemDto {
        serde_json::from_str(json).expect("channel fixture")
    }

    #[test]
    fn a_channel_is_a_lineup_row_numbered_by_the_server() {
        let c = channel(r#"{"Id":"AB12","Name":"Family","Type":"TvChannel","ChannelNumber":"101","ImageTags":{"Primary":"t1"}}"#);
        let row = channel_of(&c).unwrap();
        assert_eq!((row.grid_key.as_str(), row.vcn.as_str(), row.title.as_str()), ("ab12", "101", "101 Family"));
        assert!(row.thumb.contains("/Items/ab12/Images/Primary"));
        assert!(channel_of(&channel(r#"{"Name":"x"}"#)).is_none());
    }

    #[test]
    fn the_programme_on_now_is_an_airing_on_its_channel() {
        let c = channel(r#"{"Id":"ab12","Name":"Family","ChannelNumber":"101","CurrentProgram":{"Id":"p1","Name":"Pilot","SeriesName":"Show","Overview":"Plot","StartDate":"2026-11-26T10:00:00.0000000Z","EndDate":"2026-11-26T11:00:00.0000000Z"}}"#);
        let m = airing_of(&c).unwrap();
        assert_eq!((m.title.as_str(), m.grandparent_title.as_str(), m.kind.as_str()), ("Pilot", "Show", "episode"));
        let media = &m.media[0];
        assert_eq!(media.channel_identifier, "ab12");
        assert_eq!(media.ends_at - media.begins_at, 3600);
        assert!(airing_of(&channel(r#"{"Id":"ab12"}"#)).is_none());
    }

    #[test]
    fn a_tune_keeps_the_stream_to_close_and_the_playlist_without_the_servers_credentials() {
        let r: Response = serde_json::from_str(r#"{"MediaSources":[{"Id":"s","LiveStreamId":"ls9","TranscodingUrl":"/videos/ab12/master.m3u8?DeviceId=d&VideoCodec=h264&AudioCodec=ac3&PlaySessionId=srv&api_key=secret&LiveStreamId=ls9"}]}"#).unwrap();
        let t = tuned("ab12", &r).unwrap();
        assert_eq!((t.session.key.as_str(), t.session.rating_key.as_str()), ("ls9", "ab12"));
        assert_eq!((t.play.vcodec.as_str(), t.play.acodec.as_str(), t.play.channels), ("h264", "aac", 2));
        assert!(!t.play.part.contains("secret") && !t.play.part.contains("srv"));
        assert!(t.play.part.contains("LiveStreamId=ls9"));
        assert!(tuned("ab12", &Response::default()).is_none());
    }

    #[test]
    fn the_profile_asks_for_hls_in_h264_and_aac_only() {
        let p = live_profile();
        assert_eq!(p["TranscodingProfiles"][0]["Protocol"], "hls");
        assert_eq!(p["TranscodingProfiles"][0]["AudioCodec"], "aac");
        assert_eq!(p["DirectPlayProfiles"].as_array().unwrap().len(), 0);
    }
}
