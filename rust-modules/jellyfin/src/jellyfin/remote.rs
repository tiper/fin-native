//! Remote control: another Jellyfin client (the phone app, the web client) drives this one.
//!
//! The server lists this client as a cast target once it says what it can do
//! ([`CAPABILITIES`], posted each time the socket opens), and sends the commands over the socket
//! ([`super::live`]). Each becomes what the app already acts on:
//!
//! * `Play` (`PlayNow`) starts the first item named where Resume would, as a launch link does
//!   (`plx_platform::tv::launch`): the link waits for a signed-in Home and ends any playback first.
//!   The other play commands (next, last, shuffle, instant mix) need a queue and are not taken.
//! * `Playstate` (pause, unpause, play/pause, stop, rewind, fast-forward) and the navigation
//!   `GeneralCommand`s (the arrows, select, back, text) become presses of the remote's keys, in the
//!   remote FIFO's alphabet, which the app takes from [`take_keys`] each frame. A seek to a
//!   position and a track change have no key and are not taken.

use super::ids::normalize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// What this client tells the server it does (`POST /Sessions/Capabilities/Full`).
/// `SupportsMediaControl` is what lists it as a cast target and lets the server send `Play` and
/// `Playstate`; the `GeneralCommand`s named are the ones [`read`] takes.
pub const CAPABILITIES: &str = r#"{"PlayableMediaTypes":["Video"],"SupportedCommands":["MoveUp","MoveDown","MoveLeft","MoveRight","Select","Back","SendString"],"SupportsMediaControl":true,"SupportsPersistentIdentifier":true}"#;

/// The most keys waiting for a frame; a stalled frame thread drops the oldest.
const KEYS_MAX: usize = 32;

static KEYS: Mutex<Vec<String>> = Mutex::new(Vec::new());
static WAITING: AtomicBool = AtomicBool::new(false);

/// What one socket message asks of this client.
#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    /// Start this item (a rating key).
    Play(String),
    /// Press this key (a remote FIFO token).
    Key(String),
}

/// The command in a socket message of type `kind`, if it is one this client takes.
pub fn read(kind: &str, data: &serde_json::Value) -> Option<Command> {
    let key = |k: &str| Some(Command::Key(k.to_owned()));
    match kind {
        "Play" if data["PlayCommand"].as_str().unwrap_or("PlayNow") == "PlayNow" => {
            let id = normalize(data["ItemIds"].get(0)?.as_str()?);
            (!id.is_empty()).then_some(Command::Play(id))
        }
        "Playstate" => match data["Command"].as_str()? {
            "Pause" => key("pause"),
            "Unpause" => key("play"),
            "PlayPause" => key("playpause"),
            "Stop" => key("stop"),
            "Rewind" => key("rew"),
            "FastForward" => key("ff"),
            _ => None,
        },
        "GeneralCommand" => match data["Name"].as_str()? {
            "MoveUp" => key("up"),
            "MoveDown" => key("down"),
            "MoveLeft" => key("left"),
            "MoveRight" => key("right"),
            "Select" => key("ok"),
            "Back" => key("back"),
            // The FIFO's tokens are space-delimited: `+` stands for a space, so a typed `+` is
            // read back as one.
            "SendString" => match data["Arguments"]["String"].as_str()? {
                "" => None,
                text => Some(Command::Key(format!("txt:{}", text.replace(char::is_whitespace, "+")))),
            },
            _ => None,
        },
        _ => None,
    }
}

/// The launch payload that plays `id`, shaped as the system's own (`app::deeplink::parse` reads
/// the caller's params under `parameters`).
fn play_link(id: &str) -> String {
    serde_json::json!({ "parameters": { "plex": { "ratingKey": id, "action": "play" } } }).to_string()
}

/// Act on a command: a play goes to the launch link, a key waits for the next frame.
pub fn act(command: Command) {
    plx_base::eventlog::log(&format!("jellyfin: remote {command:?}"));
    match command {
        Command::Play(id) => plx_platform::tv::launch::offer(&play_link(&id)),
        Command::Key(token) => {
            let mut keys = KEYS.lock().unwrap_or_else(|p| p.into_inner());
            if keys.len() == KEYS_MAX {
                keys.remove(0);
            }
            keys.push(token);
            WAITING.store(true, Ordering::Release);
        }
    }
}

/// The keys pressed from another device since the last call, oldest first. FRAME THREAD; one
/// atomic swap when there are none. The queue is `plx_platform::tv::launch`'s, for keys.
pub fn take_keys() -> Vec<String> {
    if !WAITING.swap(false, Ordering::Acquire) {
        return Vec::new();
    }
    std::mem::take(&mut *KEYS.lock().unwrap_or_else(|p| p.into_inner()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(kind: &str, data: &str) -> Option<Command> {
        read(kind, &serde_json::from_str(data).unwrap())
    }

    fn key(k: &str) -> Option<Command> {
        Some(Command::Key(k.into()))
    }

    #[test]
    fn play_now_starts_the_first_item_and_the_queue_commands_are_not_taken() {
        assert_eq!(msg("Play", r#"{"ItemIds":["AB-12","cd"],"PlayCommand":"PlayNow"}"#), Some(Command::Play("ab12".into())));
        assert_eq!(msg("Play", r#"{"ItemIds":["ab"]}"#), Some(Command::Play("ab".into())));
        assert_eq!(msg("Play", r#"{"ItemIds":["ab"],"PlayCommand":"PlayNext"}"#), None);
        assert_eq!(msg("Play", r#"{"ItemIds":[]}"#), None);
    }

    #[test]
    fn transport_and_navigation_commands_are_the_remotes_keys() {
        assert_eq!(msg("Playstate", r#"{"Command":"Pause"}"#), key("pause"));
        assert_eq!(msg("Playstate", r#"{"Command":"Unpause"}"#), key("play"));
        assert_eq!(msg("Playstate", r#"{"Command":"PlayPause"}"#), key("playpause"));
        assert_eq!(msg("Playstate", r#"{"Command":"Stop"}"#), key("stop"));
        assert_eq!(msg("Playstate", r#"{"Command":"FastForward"}"#), key("ff"));
        assert_eq!(msg("Playstate", r#"{"Command":"Seek","SeekPositionTicks":1}"#), None);
        assert_eq!(msg("GeneralCommand", r#"{"Name":"MoveLeft"}"#), key("left"));
        assert_eq!(msg("GeneralCommand", r#"{"Name":"Select"}"#), key("ok"));
        assert_eq!(msg("GeneralCommand", r#"{"Name":"SendString","Arguments":{"String":"star wars"}}"#), key("txt:star+wars"));
        assert_eq!(msg("GeneralCommand", r#"{"Name":"SetVolume"}"#), None);
        assert_eq!(msg("UserDataChanged", "{}"), None);
    }

    #[test]
    fn a_play_is_a_launch_link_under_parameters() {
        assert_eq!(play_link("ab12"), r#"{"parameters":{"plex":{"action":"play","ratingKey":"ab12"}}}"#);
    }

    #[test]
    fn keys_wait_for_the_frame_in_order_and_once() {
        let _ = take_keys();
        act(Command::Key("pause".into()));
        act(Command::Key("play".into()));
        assert_eq!(take_keys(), ["pause", "play"]);
        assert!(take_keys().is_empty());
    }
}
