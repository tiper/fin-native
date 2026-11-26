//! Live TV reads and the tune.

use super::*;
use crate::jellyfin::livetv::{self, Tuned};
use plx_plex::plex::livetv::{LivePlay, LiveSession};

impl JfClient {
    /// The user's channels, each with the programme on now, as read in the last few seconds (the
    /// source, lineup and airings of one discovery are three questions of one answer), else read
    /// now. `None` when the server could not be asked.
    pub fn live_channels(&self) -> Option<Arc<Vec<BaseItemDto>>> {
        self.recent_channels
            .get(RECENT_CHANNELS, || {
                self.get::<QueryResult<BaseItemDto>>(&paths::live_channels(self.user_id()))
                    .map(|r| Arc::new(r.items))
                    .ok_or(())
            })
            .ok()
    }

    /// Tune `channel_id` for the play session `play_session_id`: the server opens its live stream
    /// and answers the playlist for `profile`, which [`Self::live_play`] hands back by the stream's
    /// id until [`Self::close_live_stream`]. The session is remembered as transcoding, so ending
    /// the play ends the server's encoder too.
    pub fn tune_channel(&self, channel_id: &str, play_session_id: &str, profile: &serde_json::Value) -> Option<LiveSession> {
        let body = serde_json::json!({ "DeviceProfile": profile }).to_string();
        let a = self.send(Verb::Post, &paths::live_playback_info(self.user_id(), channel_id), Some(&body))?;
        if !a.ok() {
            return None;
        }
        let Tuned { session, play } = livetv::tuned(channel_id, &serde_json::from_slice(&a.body).ok()?)?;
        self.transcoding.lock().unwrap_or_else(|p| p.into_inner()).insert(play_session_id.to_string());
        self.live.lock().unwrap_or_else(|p| p.into_inner()).insert(session.key.clone(), play);
        Some(session)
    }

    /// What to play for a tuned stream.
    pub fn live_play(&self, live_stream_id: &str) -> Option<LivePlay> {
        self.live.lock().unwrap_or_else(|p| p.into_inner()).get(live_stream_id).cloned()
    }

    /// Release a live stream's tuner and end the encoder of the play session that read it.
    pub fn close_live_stream(&self, live_stream_id: &str, play_session_id: &str) -> bool {
        self.live.lock().unwrap_or_else(|p| p.into_inner()).remove(live_stream_id);
        let closed = !live_stream_id.is_empty()
            && self.send(Verb::Post, &paths::close_live_stream(live_stream_id), None).is_some_and(|a| a.ok());
        self.stop_transcode(play_session_id) || closed
    }
}
