//! **The elements of Browse's cards**: one per programme (its server, DVR, channel key and start), allocated the first time the
//! programme is shown, counting up from [`BROWSE_FIRST_ELEM`] and never reused within a page. A card keeps its element while the
//! filter, the clock and the store change the set around it, so the focus follows the PROGRAMME and not its place in the list, and no
//! element ever names two programmes. Nothing caps what can be reached: the table grows by the programmes a page has shown.
//!
//! **It does not grow with the page's life**: when the local day turns ([`Idents::turn_day`]) the programmes that began before
//! the previous day and are not on show are forgotten. An element is only ever asked for by the programme that is shown or that
//! focus is on, so a forgotten programme (which no guide still lists) costs nothing; one that is shown again, were it to come back,
//! is a new card.
//!
//! **A lookup allocates nothing**: a programme is found by borrowing the channel's strings ([`KeyView`]), so a rebuild of a
//! lineup's whole set clones no DVR or channel key for the programmes it already knows.
use std::borrow::Borrow;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use plx_data::livetv::Channel;
use plx_plex::plex::ServerId;

use super::BROWSE_FIRST_ELEM;

/// A programme's identity across reads and rebuilds: server, DVR, channel key, start.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct ProgrammeKey {
    pub sid: ServerId,
    pub dvr: String,
    pub channel: String,
    pub begins_at: i64,
}

/// A programme key read without owning its strings: what a lookup hashes and compares by.
trait KeyView {
    fn view(&self) -> (ServerId, &str, &str, i64);
}

impl KeyView for ProgrammeKey {
    fn view(&self) -> (ServerId, &str, &str, i64) {
        (self.sid, &self.dvr, &self.channel, self.begins_at)
    }
}

/// The borrowed key of a channel's programme.
struct Borrowed<'a>(&'a Channel, i64);

impl KeyView for Borrowed<'_> {
    fn view(&self) -> (ServerId, &str, &str, i64) {
        (self.0.sid, &self.0.dvr, &self.0.channel, self.1)
    }
}

impl<'a> Borrow<dyn KeyView + 'a> for ProgrammeKey {
    fn borrow(&self) -> &(dyn KeyView + 'a) {
        self
    }
}

// the trait object hashes and compares exactly as the owned key does (same fields, same order), as `Borrow` requires
impl Hash for dyn KeyView + '_ {
    fn hash<H: Hasher>(&self, state: &mut H) {
        let (sid, dvr, channel, begins_at) = self.view();
        sid.hash(state);
        dvr.hash(state);
        channel.hash(state);
        begins_at.hash(state);
    }
}

impl PartialEq for dyn KeyView + '_ {
    fn eq(&self, other: &Self) -> bool {
        self.view() == other.view()
    }
}

impl Eq for dyn KeyView + '_ {}

#[derive(Debug, Default, PartialEq)]
pub(super) struct Idents {
    ids: HashMap<ProgrammeKey, u32>,
    keys: HashMap<u32, ProgrammeKey>,
    /// The next id to hand out; it only counts up.
    next: u32,
    /// The end of the local day the table was last turned for.
    day_end: i64,
}

impl Idents {
    /// The element of the programme of `channel` that begins at `begins_at`, allocated when it has none yet.
    pub fn elem(&mut self, channel: &Channel, begins_at: i64) -> u32 {
        let probe = Borrowed(channel, begins_at);
        if let Some(id) = self.ids.get(&probe as &dyn KeyView) {
            return BROWSE_FIRST_ELEM + id;
        }
        // an impossible exhaustion (some 250 million programmes in one page's life) is a loud failure, not a limit on the list
        let id = Some(self.next)
            .filter(|id| BROWSE_FIRST_ELEM.checked_add(*id).is_some_and(|elem| elem < plx_ui::geom::NAMESPACE_FLOOR))
            .expect("browse card key space exhausted");
        self.next += 1;
        let key = ProgrammeKey { sid: channel.sid, dvr: channel.dvr.clone(), channel: channel.channel.clone(), begins_at };
        self.ids.insert(key.clone(), id);
        self.keys.insert(id, key);
        BROWSE_FIRST_ELEM + id
    }

    /// The programme a card element stands for, if one was allocated and is remembered.
    pub fn programme(&self, elem: u32) -> Option<&ProgrammeKey> {
        self.keys.get(&elem.checked_sub(BROWSE_FIRST_ELEM)?)
    }

    /// **The local day turned** (`day_end` is not the one the table was last turned for): forget the programmes that began
    /// before the previous day, except those `keep` names (the ones on show). Returns how many were forgotten.
    pub fn turn_day(&mut self, day_end: i64, keep: impl Fn(u32) -> bool) -> usize {
        if day_end == std::mem::replace(&mut self.day_end, day_end) {
            return 0;
        }
        let cutoff = day_end.saturating_sub(2 * super::browse::DAY);
        let stale: Vec<u32> = self.keys.iter()
            .filter(|(id, key)| key.begins_at < cutoff && !keep(BROWSE_FIRST_ELEM + **id))
            .map(|(id, _)| *id)
            .collect();
        for id in &stale {
            if let Some(key) = self.keys.remove(id) {
                self.ids.remove(&key);
            }
        }
        stale.len()
    }

    /// How many programmes the table remembers.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.keys.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::livetv::browse::DAY;

    fn channel(key: &str) -> Channel {
        Channel {
            sid: ServerId::from_raw(1), dvr: "7".into(), channel: key.into(), vcn: "2".into(), name: "Name".into(), thumb: String::new(),
            now: String::new(), now_thumb: String::new(), now_summary: String::new(), begins_at: 0, ends_at: 0, later: Vec::new(),
        }
    }

    /// A programme keeps its element however often it is asked for, and two programmes never share one.
    #[test]
    fn a_programme_keeps_its_element_and_no_two_programmes_share_one() {
        let mut idents = Idents::default();
        let (a, b) = (channel("A"), channel("B"));
        let first = idents.elem(&a, 100);
        assert_eq!(idents.elem(&a, 100), first);
        let others = [idents.elem(&a, 200), idents.elem(&b, 100)];
        assert!(others.iter().all(|e| *e != first) && others[0] != others[1]);
        assert_eq!(idents.len(), 3);
        let key = idents.programme(first).expect("allocated");
        assert_eq!((key.channel.as_str(), key.begins_at), ("A", 100));
    }

    /// **The table does not grow with the page's life**: a day turning forgets what began before the previous day and is not
    /// on show; what is on show, and what began since, stays. Red while nothing was ever forgotten.
    #[test]
    fn the_local_day_turning_forgets_the_programmes_older_than_the_previous_day() {
        let mut idents = Idents::default();
        let a = channel("A");
        let day = 10 * DAY;
        assert_eq!(idents.turn_day(day, |_| false), 0, "the first day forgets nothing");
        let old = idents.elem(&a, day - 3 * DAY);
        let on_show = idents.elem(&a, day - 3 * DAY + 60);
        let yesterday = idents.elem(&a, day - 60);
        let today = idents.elem(&a, day + 60);
        assert_eq!(idents.turn_day(day, |_| false), 0, "the same day turns nothing");
        assert_eq!(idents.turn_day(day + DAY, |elem| elem == on_show), 1, "only the old programme that is not shown goes");
        assert!(idents.programme(old).is_none());
        assert!([on_show, yesterday, today].iter().all(|e| idents.programme(*e).is_some()));
        // elements are never reused: a programme that comes back is a new card, not the forgotten one's
        let back = idents.elem(&a, day - 3 * DAY);
        assert!(back != old && idents.programme(old).is_none());
    }
}
