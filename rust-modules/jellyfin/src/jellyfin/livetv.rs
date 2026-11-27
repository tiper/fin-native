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
use plx_plex::plex::urlenc_str as enc;
use super::playback_info::Response;
use super::time::{iso_to_unix, unix_to_iso};
use super::translate::primary_image;
use plx_plex::plex::livetv::{LivePlay, LiveSession};
use plx_plex::plex::{Dvr, EpgChannel, Media, Metadata};
use serde_json::{json, Value};
use std::collections::HashMap;

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
    Some(airing(c.current_program.as_deref()?, &c.id, c.channel_number.as_deref()))
}

/// The guide's programmes ([`programs_path`]) as airings on their channels, numbered as the
/// lineup `channels` numbers them; a programme on no channel of the lineup is dropped.
pub fn programs_of(programs: &[BaseItemDto], channels: &[BaseItemDto]) -> Vec<Metadata> {
    let lineup: HashMap<String, &BaseItemDto> = channels.iter().map(|c| (normalize(&c.id), c)).collect();
    programs
        .iter()
        .filter_map(|p| {
            let c = lineup.get(&normalize(p.channel_id.as_deref()?))?;
            Some(airing(p, &c.id, c.channel_number.as_deref()))
        })
        .collect()
}

fn airing(p: &BaseItemDto, channel_id: &str, vcn: Option<&str>) -> Metadata {
    let at = |stamp: &Option<String>| stamp.as_deref().map_or(0, iso_to_unix);
    let series = p.series_name.clone().filter(|s| !s.is_empty());
    Metadata {
        title: p.name.clone(),
        kind: if series.is_some() { "episode".into() } else { "movie".into() },
        grandparent_title: series.unwrap_or_default(),
        summary: p.overview.clone().unwrap_or_default(),
        thumb: primary_image(p),
        media: vec![Media {
            channel_identifier: normalize(channel_id),
            channel_vcn: vcn.unwrap_or_default().to_owned(),
            begins_at: at(&p.start_date),
            ends_at: at(&p.end_date),
            ..Default::default()
        }],
        ..Default::default()
    }
}

/// What a live play arrives as: H.264 and AAC stereo. The profile allows nothing else, so a
/// source in other codecs is converted to them, and the route's declaration is these names rather
/// than whatever the server's URL happens to list.
const VIDEO_CODEC: &str = "h264";
const AUDIO_CODEC: &str = "aac";

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

/// `GET`: the user's channels, each with the programme on now.
pub fn channels_path(user_id: &str) -> String {
    format!(
        "/LiveTv/Channels?userId={}&addCurrentProgram=true&enableUserData=false&enableImages=true&enableImageTypes=Primary&imageTypeLimit=1",
        enc(user_id)
    )
}

/// `GET`: the guide's programmes that have not ended by `now` and start by `until` (unix seconds),
/// on every channel, in start order: per channel the one on now and the ones that follow.
pub fn programs_path(user_id: &str, now: i64, until: i64) -> String {
    format!(
        "/LiveTv/Programs?userId={}&minEndDate={}&maxStartDate={}&sortBy=StartDate&fields=Overview&enableUserData=false&enableTotalRecordCount=false&enableImages=true&enableImageTypes=Primary&imageTypeLimit=1",
        enc(user_id),
        enc(&unix_to_iso(now)),
        enc(&unix_to_iso(until))
    )
}

/// `POST`: tune a channel. The server opens its live stream with the play and answers the source
/// with the stream's id and, for the profile's HLS target, the playlist.
pub fn tune_path(user_id: &str, channel_id: &str) -> String {
    format!(
        "/Items/{}/PlaybackInfo?userId={}&isPlayback=true&autoOpenLiveStream=true",
        enc(channel_id),
        enc(user_id)
    )
}

/// `POST`: release a live stream's tuner.
pub fn close_path(live_stream_id: &str) -> String {
    format!("/LiveStreams/Close?liveStreamId={}", enc(live_stream_id))
}

/// The tune's answer: the session (the live stream's id, which closes it, and the channel) and the
/// first source's playlist, kept as a path with neither the server's credential nor its play
/// session: the route adds this client's own. `None` when no source has a playlist or no stream
/// was opened, which nothing could close.
pub fn tuned(channel_id: &str, r: &Response) -> Option<(LiveSession, String)> {
    let source = r.media_sources.iter().find(|s| s.transcoding_url.as_deref().is_some_and(|u| !u.is_empty()))?;
    let key = source.live_stream_id.clone().filter(|id| !id.is_empty())?;
    // The server signs the URL as `ApiKey` (older ones `api_key`): either would be a second
    // credential beside this client's own.
    let part = ["api_key", "ApiKey", "PlaySessionId"]
        .iter()
        .fold(source.transcoding_url.clone()?, |u, key| paths::without_param(&u, key));
    Some((LiveSession { key, rating_key: channel_id.to_owned() }, part))
}

/// The play the route reads for a tuned playlist: always what the profile asked for.
pub fn play_of(part: String) -> LivePlay {
    LivePlay { part, vcodec: VIDEO_CODEC.into(), acodec: AUDIO_CODEC.into(), width: 0, height: 0, channels: 2 }
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
    fn a_guide_programme_airs_on_its_lineup_channel() {
        let lineup = [channel(r#"{"Id":"ab12","Name":"Family","ChannelNumber":"101"}"#)];
        let programs = [
            channel(r#"{"Id":"p2","Name":"Next","ChannelId":"AB12","StartDate":"2026-11-26T11:00:00Z","EndDate":"2026-11-26T11:30:00Z"}"#),
            channel(r#"{"Id":"p3","Name":"Other","ChannelId":"cd34"}"#),
            channel(r#"{"Id":"p4","Name":"Loose"}"#),
        ];
        let airings = programs_of(&programs, &lineup);
        assert_eq!(airings.len(), 1, "programmes on no lineup channel are dropped");
        let m = &airings[0];
        assert_eq!((m.title.as_str(), m.kind.as_str()), ("Next", "movie"));
        let media = &m.media[0];
        assert_eq!((media.channel_identifier.as_str(), media.channel_vcn.as_str()), ("ab12", "101"));
        assert_eq!(media.ends_at - media.begins_at, 1800);
    }

    #[test]
    fn the_guide_asks_for_what_has_not_ended_by_now_and_starts_by_until() {
        let path = programs_path("u1", 1_764_151_200, 1_764_156_600);
        assert!(path.starts_with("/LiveTv/Programs?userId=u1&"), "{path}");
        assert!(path.contains("minEndDate=2025-11-26T10%3A00%3A00Z"), "{path}");
        assert!(path.contains("maxStartDate=2025-11-26T11%3A30%3A00Z"), "{path}");
        assert!(path.contains("sortBy=StartDate"), "{path}");
    }

    #[test]
    fn a_tune_keeps_the_stream_to_close_and_the_playlist_without_the_servers_credentials() {
        let r: Response = serde_json::from_str(r#"{"MediaSources":[{"Id":"s","LiveStreamId":"ls9","TranscodingUrl":"/videos/ab12/master.m3u8?DeviceId=d&VideoCodec=h264&AudioCodec=ac3&PlaySessionId=srv&ApiKey=secret&api_key=old&LiveStreamId=ls9"}]}"#).unwrap();
        let (session, part) = tuned("ab12", &r).unwrap();
        assert_eq!((session.key.as_str(), session.rating_key.as_str()), ("ls9", "ab12"));
        let play = play_of(part);
        assert_eq!((play.vcodec.as_str(), play.acodec.as_str(), play.channels), ("h264", "aac", 2));
        assert!(!play.part.contains("secret") && !play.part.contains("old") && !play.part.contains("srv"));
        assert!(!play.part.to_ascii_lowercase().contains("apikey") && !play.part.contains("api_key"));
        assert!(play.part.contains("LiveStreamId=ls9"));
        assert!(tuned("ab12", &Response::default()).is_none());
        let unopened: Response = serde_json::from_str(r#"{"MediaSources":[{"Id":"s","TranscodingUrl":"/videos/ab12/master.m3u8"}]}"#).unwrap();
        assert!(tuned("ab12", &unopened).is_none());
    }

    #[test]
    fn the_profile_asks_for_hls_in_h264_and_aac_only() {
        let p = live_profile();
        assert_eq!(p["TranscodingProfiles"][0]["Protocol"], "hls");
        assert_eq!(p["TranscodingProfiles"][0]["AudioCodec"], "aac");
        assert_eq!(p["DirectPlayProfiles"].as_array().unwrap().len(), 0);
    }
}
