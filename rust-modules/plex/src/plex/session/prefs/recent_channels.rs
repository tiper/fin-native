//! **Per-profile Recent channels — the data layer.** The channels a profile tuned most recently,
//! remembered on this television so Live TV's "Recent" row survives a restart and a profile switch.
//! The Plex server has no such source (its EPG provider offers `whatsonnow`, `onrightnow`,
//! `whatsonlater` and so on, none for recent channels), and this is never a write to the Plex
//! account: it is a local per-profile preference, cleared with the credentials at sign-out.
//!
//! It is a tenant of the memory door (`ProfileMemory::upsert_with`, `route::{remember, recall_all}`),
//! so a record carries the account stamp, a write needs a seated profile, and the list is soft-parsed
//! per element. It is the ONLY list of a seated profile's Recent: the join-at-front rule and the cap
//! below are applied here, by the write, and Live TV's store is handed the result as an input and
//! resolves it against the roster and the lineup (`plx_data::livetv::resolve`); an entry that
//! resolution hides (its server away, its channel off the lineup) stays here and counts toward the cap. The list is `Preferences::recent_channels`, ONE entry per channel, **oldest first**:
//! a newly tuned channel is appended (it joins the FRONT of the row, which reads the list
//! reversed), a channel already in the list is left exactly where it is
//! ([`RecentChannelEntry::noted`] hands the entry back unchanged, which `upsert_with` does not
//! move), and the [`MAX_RECENT`] cap drains the oldest, so a sixth channel drops the one at the
//! END of the row.
//!
//! The key is DURABLE: the server's machine identifier plus the DVR and channel ids, never a
//! `ServerId` (a registry slot, stable only for the life of the process).
//!
//! ```json
//! { "machine_id": "…", "dvr": "7", "channel": "C1" }
//! ```
//!
//! An unknown field inside an entry is ignored and not written back; an entry that cannot be read
//! (a missing key, a wrong type, an over-long id) is dropped alone and the rest of the list survives.
use super::{MemoryEntry, Parse, ProfileList, Row, Scope, SettingsWrite};
use crate::plex::session::Session;
use serde::{Deserialize, Serialize};

/// The most channels one profile's Recent holds; the oldest is dropped first.
pub const MAX_RECENT: usize = 5;
/// The longest id (server machine id, DVR key, channel id) an entry keeps; text length.
pub const ID_MAX: usize = 64;

/// One remembered channel: which server, which DVR on it, which channel of that DVR.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RecentChannelWire")]
pub struct RecentChannelEntry {
    pub machine_id: String,
    pub dvr: String,
    pub channel: String,
}

/// The tolerant read shape: three required strings, every other field ignored.
#[derive(Deserialize)]
struct RecentChannelWire {
    machine_id: String,
    dvr: String,
    channel: String,
}

impl TryFrom<RecentChannelWire> for RecentChannelEntry {
    type Error = &'static str;
    fn try_from(wire: RecentChannelWire) -> Result<Self, Self::Error> {
        let entry = RecentChannelEntry { machine_id: wire.machine_id, dvr: wire.dvr, channel: wire.channel }.normalized();
        if entry.is_blank() { Err("a recent channel needs a server, a DVR and a channel") } else { Ok(entry) }
    }
}

impl RecentChannelEntry {
    pub fn new(machine_id: &str, dvr: &str, channel: &str) -> Self {
        RecentChannelEntry { machine_id: machine_id.to_owned(), dvr: dvr.to_owned(), channel: channel.to_owned() }.normalized()
    }

    /// An id longer than its bound is not stored at all (it reads as absent, which leaves the
    /// entry blank): a cut id would name some other channel.
    pub fn normalized(mut self) -> Self {
        for id in [&mut self.machine_id, &mut self.dvr, &mut self.channel] {
            if id.len() > ID_MAX {
                id.clear();
            }
        }
        self
    }

    pub fn is_blank(&self) -> bool {
        self.machine_id.is_empty() || self.dvr.is_empty() || self.channel.is_empty()
    }

    /// **The "note a tune" rule, as the memory door's merge closure**: a channel the list already
    /// holds is handed back as it is (an equal entry is neither rewritten nor moved), and a new one
    /// is appended as the most recent. It runs against the list the file holds when the write
    /// lands, so a channel noted on another run is not reordered by this one.
    pub fn noted(self) -> impl Fn(Option<&RecentChannelEntry>) -> Option<RecentChannelEntry> + Clone + Send + 'static {
        move |current| current.cloned().or_else(|| Some(self.clone()))
    }
}

impl MemoryEntry for RecentChannelEntry {
    type Key = (String, String, String);
    const CAP: usize = MAX_RECENT;
    const ACCOUNT_BOUND: bool = true;
    fn key(&self) -> Self::Key {
        (self.machine_id.clone(), self.dvr.clone(), self.channel.clone())
    }
    fn matches(&self, key: &Self::Key) -> bool {
        self.machine_id == key.0 && self.dvr == key.1 && self.channel == key.2
    }
    fn forgets(&self) -> bool {
        self.is_blank()
    }
    fn normalized(self) -> Self {
        RecentChannelEntry::normalized(self)
    }
    fn slot(session: &Session) -> &Vec<ProfileList<Self>> {
        &session.prefs.recent_channels
    }
    fn slot_mut(session: &mut Session) -> &mut Vec<ProfileList<Self>> {
        &mut session.prefs.recent_channels
    }
}

/// The `recent_channels` row: one profile's list, bound to the account (sign-out drops it, and every
/// record carries the account stamp), read by Live TV's store through the app and written only
/// through the memory door; a [`Row`] and deliberately not a `ListRow`, like `track_memory`.
pub struct RecentChannelsPref;
impl Row for RecentChannelsPref {
    const KEY: &'static str = "recent_channels";
    const SCOPE: Scope = Scope::Profile;
    const ACCOUNT_BOUND: bool = true;
    const LIVE: bool = false;
    const SETTINGS: Option<SettingsWrite> = None;
    const PARSE: Parse = Parse::SoftVec;
}
