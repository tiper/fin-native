//! Live updates: Jellyfin's WebSocket, as the app's server-update feed.
//!
//! The data layer keeps one feed per server open (`plx_data::stores::notify`) and turns what it
//! reports into quiet re-reads of the items the screens hold (`stores::notify_map`). On Plex the
//! feed is the server's event stream (`plx_plex::plex::notify`); here it is `/socket`, held open by
//! the shared WebSocket transport (`plx_net::ws`), and each message the app can use becomes one of
//! the feed's sessions:
//!
//! * `UserDataChanged` for this user (a watched mark, a resume point, a play count, from any
//!   device, this one included) reports each changed item `stopped`, which re-reads it where it is
//!   held and refreshes Continue Watching. A favourite mark that differs from the one this app
//!   holds was made on another device: Home is re-read at once, so its Favorites shelf follows
//!   (`favorites::heard`); the feed's own Home refresh waits while a visible row would move.
//! * `LibraryChanged` reports the items added, removed and updated (the first few of each) as
//!   the library changes they are, which re-read the server's Home, shelves and library grids. An
//!   update counts: an added item is listed only once its scan finishes, which Jellyfin reports so.
//! * `UserUpdated` for this user (a name or avatar edited on any device) has the backend read the
//!   user again and swap in the new name and avatar (`backend::refresh_signed_in_user`), and
//!   Home is re-read with the user's Home settings as they now are (`favorites::owe_home`); it
//!   adds nothing to the feed.
//! * `Play`, `Playstate` and `GeneralCommand` are another client driving this one
//!   ([`super::remote`]): they add nothing to the feed.
//! * `ForceKeepAlive` sets how often this client says `KeepAlive`, which the server needs to keep
//!   the connection; every message, the server's `KeepAlive` answers included, is a ping.
//!
//! The feed is a hint, never a playback event of ours (the data layer's rule), so nothing here
//! touches a timeline or a stop. Each report gets its own session key: the map keeps the last state
//! per session, and two changes to one item would otherwise read as no news.

use super::ids::normalize;
use super::remote::{self, Command};
use plx_net::net::connect_only::OpenError;
use plx_net::ws::{self, Dial, End, Event, Fault, HandshakeFault, Outbox, Refusal};
use plx_plex::plex::notify::{LibraryEntry, LibraryKind, LibraryState, Notification, NotifyEnd, PlaySession, PlayState};
use std::sync::atomic::AtomicBool;
use std::sync::{Condvar, Mutex};
use std::time::Duration;

/// How often to say `KeepAlive` until the server asks for a rate (`ForceKeepAlive`).
const KEEPALIVE: Duration = Duration::from_secs(30);

/// A connection that has heard nothing for this long past its keep-alive is gone.
const IDLE_MARGIN: Duration = Duration::from_secs(20);

/// How many items of each kind (added, removed, updated) one `LibraryChanged` reports; a scan can
/// name thousands, and any one entry already re-lists the server's pages.
const LIBRARY_IDS: usize = 64;

/// A connection that has heard nothing for this long is gone. The transport takes one figure for
/// the whole connection, so this is the server's keep-alive (a minute, in every Jellyfin) plus
/// [`IDLE_MARGIN`], not a figure that follows `ForceKeepAlive`.
const IDLE: Duration = Duration::from_secs(60 + IDLE_MARGIN.as_secs());

/// The largest message read: a `LibraryChanged` after a scan names every item it touched.
const MESSAGE_MAX: usize = 1024 * 1024;

/// Where the socket is and whose it is.
pub struct Target {
    /// `ws://` or `wss://`, the server's `/socket`, signed in by its `ApiKey` parameter (the
    /// transport sends no header of the caller's, and the query is how Jellyfin's own web client
    /// signs its socket in).
    pub url: String,
    /// Whose changes are this user's.
    pub user_id: String,
}

/// What one message from the server means to the feed.
#[derive(Default)]
pub struct Messages {
    user_id: String,
    seq: u64,
}

/// One message, read: what to tell the feed, and a keep-alive rate the server asked for.
#[derive(Debug, PartialEq, Eq)]
pub struct Read {
    pub notification: Notification,
    pub keepalive: Option<Duration>,
    /// The server says this user was edited (`UserUpdated`): a name or avatar may have changed.
    pub user_updated: bool,
    /// Another client's command to this one (`Play`, `Playstate`, `GeneralCommand`).
    pub command: Option<Command>,
}

impl Read {
    fn of(notification: Notification) -> Self {
        Read { notification, keepalive: None, user_updated: false, command: None }
    }
}

impl Messages {
    pub fn new(user_id: &str) -> Self {
        Messages { user_id: normalize(user_id), seq: 0 }
    }

    pub fn read(&mut self, text: &str) -> Read {
        let ping = Read::of(Notification::Ping);
        let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else { return ping };
        let data = &v["Data"];
        let kind = v["MessageType"].as_str().unwrap_or("");
        let mine = |key: &str| normalize(data[key].as_str().unwrap_or("")) == self.user_id;
        let changed: Vec<String> = match kind {
            "ForceKeepAlive" => {
                let secs = data.as_u64().unwrap_or(60).max(10);
                return Read { keepalive: Some(Duration::from_secs(secs / 2)), ..ping };
            }
            "UserDataChanged" if mine("UserId") => data["UserDataList"]
                .as_array()
                .map(|list| {
                    list.iter()
                        .filter_map(|d| {
                            let id = normalize(d["ItemId"].as_str()?);
                            // A favourite made on another device: Home's Favorites shelf is re-read.
                            if let Some(favourite) = d["IsFavorite"].as_bool() {
                                super::favorites::heard(&id, favourite);
                            }
                            Some(id)
                        })
                        .collect()
                })
                .unwrap_or_default(),
            "LibraryChanged" => return library(data),
            "Play" | "Playstate" | "GeneralCommand" => return Read { command: remote::read(kind, data), ..ping },
            "UserUpdated" if mine("Id") => {
                return Read { user_updated: true, ..ping };
            }
            _ => return ping,
        };
        let sessions: Vec<PlaySession> = changed
            .into_iter()
            .filter(|id| !id.is_empty())
            .map(|id| {
                self.seq += 1;
                PlaySession {
                    session_key: Some(format!("jf-{}", self.seq)),
                    client_identifier: None,
                    rating_key: Some(id),
                    key: None,
                    view_offset_ms: None,
                    state: PlayState::Stopped,
                }
            })
            .collect();
        if sessions.is_empty() {
            return ping;
        }
        Read::of(Notification::Play(sessions))
    }
}

/// Items added, removed and updated (the first [`LIBRARY_IDS`] of each), as the library changes
/// they are. Jellyfin names no kind. An update moves the lists too: an item Jellyfin reports added
/// may not be listed until its scan finishes, which it reports as an update.
fn library(data: &serde_json::Value) -> Read {
    let count = |k: &str| data[k].as_array().map_or(0, Vec::len);
    plx_base::eventlog::log(&format!(
        "jellyfin: library changed added={} removed={} updated={}",
        count("ItemsAdded"),
        count("ItemsRemoved"),
        count("ItemsUpdated")
    ));
    let entries: Vec<LibraryEntry> = [("ItemsAdded", LibraryState::Created), ("ItemsRemoved", LibraryState::Deleted), ("ItemsUpdated", LibraryState::Updated)]
        .into_iter()
        .flat_map(|(k, state)| {
            data[k]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str())
                .map(normalize)
                .filter(|id| !id.is_empty())
                .take(LIBRARY_IDS)
                .map(move |item_id| LibraryEntry { item_id, kind: LibraryKind::Unknown, state, section: None })
        })
        .collect();
    Read::of(if entries.is_empty() { Notification::Ping } else { Notification::Library(entries) })
}

/// When this client says `KeepAlive`: every `every`, and at once when the server names a rate.
/// The transport runs until the connection ends and calls back only for what arrives, so the
/// saying has a thread of its own for as long as the socket is open.
struct Beat {
    /// The rate, whether a `KeepAlive` is owed now, and whether the socket has ended.
    state: Mutex<(Duration, bool, bool)>,
    changed: Condvar,
}

impl Beat {
    fn new(every: Duration) -> Self {
        Beat { state: Mutex::new((every, false, false)), changed: Condvar::new() }
    }

    fn set(&self, every: Duration) {
        *self.state.lock().unwrap_or_else(|p| p.into_inner()) = (every, true, false);
        self.changed.notify_all();
    }

    fn stop(&self) {
        self.state.lock().unwrap_or_else(|p| p.into_inner()).2 = true;
        self.changed.notify_all();
    }

    /// Call `say` at each beat until [`Self::stop`], or until it answers that nobody listens.
    fn run(&self, mut say: impl FnMut() -> bool) {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        let mut due = false;
        loop {
            // Looked at before every wait: a rate or a stop set before this thread got here woke
            // nobody.
            if state.2 || ((due || std::mem::take(&mut state.1)) && !say()) {
                return;
            }
            let every = state.0;
            let (next, waited) = self.changed.wait_timeout(state, every).unwrap_or_else(|p| p.into_inner());
            state = next;
            due = waited.timed_out();
        }
    }
}

/// Open the socket at `target` and run it until it ends or `cancel` is set. `opened` is called
/// once the server has taken the connection.
pub fn open(off: &plx_base::task::OffFrame, target: &Target, cancel: &AtomicBool, on: &mut dyn FnMut(Notification),
    opened: &dyn Fn()) -> NotifyEnd {
    let Some((outbox, inbox)) = ws::channel() else { return NotifyEnd::Unsupported };
    let beat = Beat::new(KEEPALIVE);
    let mut messages = Messages::new(&target.user_id);
    let dial = Dial { url: &target.url, trust: plx_net::net::Tls::Ca, limit: MESSAGE_MAX, idle: IDLE };
    let end = std::thread::scope(|s| {
        let beating = std::thread::Builder::new()
            .name("jellyfin-keepalive".into())
            .stack_size(64 * 1024)
            .spawn_scoped(s, || beat.run(|| keep_alive(&outbox)));
        if beating.is_err() {
            return End::Lost;
        }
        let end = ws::run(off, dial, cancel, inbox, &mut |event| {
            let text = match event {
                Event::Opened => return opened(),
                Event::Text(text) => text,
                _ => return,
            };
            let read = messages.read(&text);
            if let Some(every) = read.keepalive {
                beat.set(every);
            }
            on(read.notification);
            if let Some(command) = read.command {
                remote::act(command);
            }
            if read.user_updated {
                // The name and avatar, and the Home settings the next read of Home takes.
                super::backend::refresh_signed_in_user();
                super::favorites::owe_home();
            }
        });
        beat.stop();
        end
    });
    notify_end(end)
}

fn keep_alive(outbox: &Outbox) -> bool {
    outbox.text(r#"{"MessageType":"KeepAlive"}"#.to_owned())
}

/// The transport's ending, as the feed names endings.
fn notify_end(end: End) -> NotifyEnd {
    match end {
        End::Cancelled | End::Refused(Refusal::Open(OpenError::Cancelled)) => NotifyEnd::Cancelled,
        End::Idle => NotifyEnd::Silent,
        End::Closed { .. } => NotifyEnd::Closed,
        End::Failed(Fault::Handshake(HandshakeFault::Refused(status))) => NotifyEnd::Refused { status },
        End::Refused(Refusal::Open(OpenError::Unavailable | OpenError::SerialTls)) => NotifyEnd::Unsupported,
        _ => NotifyEnd::Transport,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(n: &Notification) -> Vec<(&str, &str)> {
        match n {
            Notification::Play(s) => s
                .iter()
                .map(|p| (p.session_key.as_deref().unwrap(), p.rating_key.as_deref().unwrap()))
                .collect(),
            _ => Vec::new(),
        }
    }

    #[test]
    fn this_users_data_changes_report_each_item_stopped_under_its_own_key() {
        let mut m = Messages::new("5F0A-11");
        let r = m.read(r#"{"MessageType":"UserDataChanged","Data":{"UserId":"5f0a11","UserDataList":[{"ItemId":"e1","Played":true},{"ItemId":"e2"}]}}"#);
        assert_eq!(keys(&r.notification), [("jf-1", "e1"), ("jf-2", "e2")]);
        let Notification::Play(s) = &r.notification else { unreachable!() };
        assert!(s.iter().all(|p| p.state == PlayState::Stopped));
        // The same item again is news again.
        let r = m.read(r#"{"MessageType":"UserDataChanged","Data":{"UserId":"5f0a11","UserDataList":[{"ItemId":"e1"}]}}"#);
        assert_eq!(keys(&r.notification), [("jf-3", "e1")]);
        // Another user's changes are not this user's.
        let r = m.read(r#"{"MessageType":"UserDataChanged","Data":{"UserId":"other","UserDataList":[{"ItemId":"e1"}]}}"#);
        assert_eq!(r.notification, Notification::Ping);
    }

    #[test]
    fn library_changes_report_the_items_added_and_removed() {
        let mut m = Messages::new("u");
        let r = m.read(r#"{"MessageType":"LibraryChanged","Data":{"ItemsAdded":["A1","a2"],"ItemsRemoved":["r1"],"ItemsUpdated":["x"]}}"#);
        let Notification::Library(e) = &r.notification else { panic!("{:?}", r.notification) };
        let got: Vec<(&str, LibraryState)> = e.iter().map(|e| (e.item_id.as_str(), e.state)).collect();
        assert_eq!(got, [("a1", LibraryState::Created), ("a2", LibraryState::Created), ("r1", LibraryState::Deleted), ("x", LibraryState::Updated)]);
        assert!(e.iter().all(|e| e.kind == LibraryKind::Unknown));
        // An update alone moves the lists too: a scan that finishes an added item reports it so.
        let r = m.read(r#"{"MessageType":"LibraryChanged","Data":{"ItemsAdded":[],"ItemsUpdated":["x"]}}"#);
        assert!(matches!(&r.notification, Notification::Library(e) if e.len() == 1 && e[0].state == LibraryState::Updated));
        let r = m.read(r#"{"MessageType":"LibraryChanged","Data":{"ItemsAdded":[]}}"#);
        assert_eq!(r.notification, Notification::Ping);
        // A scan naming thousands is cut to the first few of each kind.
        let many: Vec<String> = (0..100).map(|i| format!("\"i{i}\"")).collect();
        let r = m.read(&format!(r#"{{"MessageType":"LibraryChanged","Data":{{"ItemsAdded":[{}]}}}}"#, many.join(",")));
        let Notification::Library(e) = &r.notification else { panic!() };
        assert_eq!(e.len(), LIBRARY_IDS);
    }

    #[test]
    fn the_server_sets_the_keep_alive_and_everything_else_is_a_ping() {
        let mut m = Messages::new("u");
        assert_eq!(m.read(r#"{"MessageType":"ForceKeepAlive","Data":60}"#).keepalive, Some(Duration::from_secs(30)));
        let r = m.read(r#"{"MessageType":"UserUpdated","Data":{"Id":"U","Name":"n"}}"#);
        assert!(r.user_updated && r.notification == Notification::Ping);
        assert!(!m.read(r#"{"MessageType":"UserUpdated","Data":{"Id":"other"}}"#).user_updated);
        for other in [r#"{"MessageType":"KeepAlive"}"#, r#"{"MessageType":"Play","Data":{}}"#, "not json"] {
            assert_eq!(m.read(other), Read { notification: Notification::Ping, keepalive: None, user_updated: false, command: None });
        }
        let r = m.read(r#"{"MessageType":"Playstate","Data":{"Command":"Pause"}}"#);
        assert_eq!((r.notification, r.command), (Notification::Ping, Some(Command::Key("pause".into()))));
    }

    #[test]
    fn a_keep_alive_is_said_at_once_when_the_server_names_a_rate_and_then_at_that_rate() {
        let beat = Beat::new(Duration::from_secs(3600));
        let said = std::sync::atomic::AtomicU32::new(0);
        std::thread::scope(|s| {
            s.spawn(|| beat.run(|| said.fetch_add(1, std::sync::atomic::Ordering::AcqRel) < 2));
            beat.set(Duration::from_millis(5));
        });
        assert_eq!(said.into_inner(), 3, "at once, then every 5 ms until nobody listens");
        let beat = Beat::new(Duration::from_secs(3600));
        beat.stop();
        beat.run(|| panic!("a stopped socket says nothing"));
    }

    #[test]
    fn the_transports_endings_are_the_feeds() {
        let refused = End::Failed(Fault::Handshake(HandshakeFault::Refused(401)));
        assert_eq!(notify_end(refused), NotifyEnd::Refused { status: 401 });
        assert_eq!(notify_end(End::Idle), NotifyEnd::Silent);
        assert_eq!(notify_end(End::Closed { code: Some(1000), reason: String::new() }), NotifyEnd::Closed);
        assert_eq!(notify_end(End::Cancelled), NotifyEnd::Cancelled);
        assert_eq!(notify_end(End::Refused(Refusal::Open(OpenError::Unavailable))), NotifyEnd::Unsupported);
        assert_eq!(notify_end(End::Lost), NotifyEnd::Transport);
        assert_eq!(notify_end(End::TimedOut), NotifyEnd::Transport);
    }
}
