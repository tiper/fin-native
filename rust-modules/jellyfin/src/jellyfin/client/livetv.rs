//! Live TV reads and the tune.

use super::*;
use crate::jellyfin::livetv;
use plx_plex::plex::livetv::{LivePlay, LiveSession};
use std::collections::HashMap;

/// How long one read of the channel list answers the questions of a single discovery.
const RECENT_CHANNELS: Duration = Duration::from_secs(5);

/// What a client holds for Live TV.
#[derive(Default)]
pub(super) struct LiveTv {
    channels: Recent<Arc<Vec<BaseItemDto>>, ()>,
    /// Channels tuned and not yet closed: the playlist path each stream (by its id) is read from.
    parts: Mutex<HashMap<String, String>>,
}

impl JfClient {
    /// The user's channels, each with the programme on now, as read in the last few seconds (the
    /// source, lineup and airings of one discovery are three questions of one answer), else read
    /// now. `None` when the server could not be asked.
    pub fn live_channels(&self) -> Option<Arc<Vec<BaseItemDto>>> {
        self.live
            .channels
            .get(RECENT_CHANNELS, || {
                self.get::<QueryResult<BaseItemDto>>(&livetv::channels_path(self.user_id()))
                    .map(|r| Arc::new(r.items))
                    .ok_or(())
            })
            .ok()
    }

    /// The guide's programmes from the window `now` falls in to `until` (unix seconds) on every channel
    /// ([`livetv::programs_path`]). `None` when the server could not be asked.
    pub fn live_programs(&self, now: i64, until: i64) -> Option<Vec<BaseItemDto>> {
        self.get::<QueryResult<BaseItemDto>>(&livetv::programs_path(self.user_id(), now, until)).map(|r| r.items)
    }

    /// Tune `channel_id` for the play session `play_session_id`: the server opens its live stream
    /// and answers the playlist for `profile`, which [`Self::live_play`] hands back by the stream's
    /// id until [`Self::close_live_stream`]. The session is remembered as transcoding, so ending
    /// the play ends the server's encoder too.
    pub fn tune_channel(&self, channel_id: &str, play_session_id: &str, profile: &serde_json::Value) -> Option<LiveSession> {
        let body = serde_json::json!({ "DeviceProfile": profile }).to_string();
        let a = self.send(Verb::Post, &livetv::tune_path(self.user_id(), channel_id), Some(&body))?;
        if !a.ok() {
            return None;
        }
        let (session, part) = livetv::tuned(channel_id, &serde_json::from_slice(&a.body).ok()?)?;
        self.transcoding.lock().unwrap_or_else(|p| p.into_inner()).insert(play_session_id.to_string());
        self.live.parts.lock().unwrap_or_else(|p| p.into_inner()).insert(session.key.clone(), part);
        Some(session)
    }

    /// What to play for a tuned stream.
    pub fn live_play(&self, live_stream_id: &str) -> Option<LivePlay> {
        let part = self.live.parts.lock().unwrap_or_else(|p| p.into_inner()).get(live_stream_id).cloned()?;
        Some(livetv::play_of(part))
    }

    /// Release a live stream's tuner and end the encoder of the play session that read it.
    pub fn close_live_stream(&self, live_stream_id: &str, play_session_id: &str) -> bool {
        self.live.parts.lock().unwrap_or_else(|p| p.into_inner()).remove(live_stream_id);
        let closed = self.send(Verb::Post, &livetv::close_path(live_stream_id), None).is_some_and(|a| a.ok());
        self.stop_transcode(play_session_id) || closed
    }
}
