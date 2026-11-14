//! **Favourite marks**: what the app's `plx_plex::plex::favorite_mark` and friends answer for a
//! Jellyfin server.
//!
//! The mark travels beside the item model rather than in it: [`super::translate`] records each
//! item's mark as it reads the item ([`note`]), and the item menu and the detail page read it back
//! by ratingKey ([`state`]). A press flips the mark at once and writes it on a worker ([`set`]); a
//! write the server refuses flips it back, and one it takes leaves Home owing a refetch
//! ([`take_landed`]) so its Favorites shelf gains or loses the item.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// Marks remembered at once. Every listing the backend reads notes its items, so this bounds a
/// long browse; a mark forgotten here reads as "not a favourite" until the item is read again.
const KEPT: usize = 4096;

static MARKS: Mutex<Option<HashMap<String, bool>>> = Mutex::new(None);

/// A write the server took since the frame thread last asked ([`take_landed`]).
static LANDED: AtomicBool = AtomicBool::new(false);

/// Record `rating_key`'s mark as the server stated it.
pub fn note(rating_key: &str, favourite: bool) {
    if rating_key.is_empty() {
        return;
    }
    let mut marks = MARKS.lock().unwrap_or_else(|p| p.into_inner());
    let marks = marks.get_or_insert_with(HashMap::new);
    if marks.len() >= KEPT && !marks.contains_key(rating_key) {
        marks.clear();
    }
    marks.insert(rating_key.to_string(), favourite);
}

/// Is `rating_key` a favourite, as last read or pressed?
pub fn state(rating_key: &str) -> bool {
    MARKS
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .as_ref()
        .and_then(|m| m.get(rating_key).copied())
        .unwrap_or(false)
}

/// Make `rating_key` a favourite (or not) with `write`, the server's own call: the mark flips now,
/// the server is told on a worker, and a refusal flips it back. Never blocks the caller.
pub fn set(rating_key: &str, favourite: bool, write: impl FnOnce(&str, bool) -> bool + Send + 'static) {
    note(rating_key, favourite);
    let rk = rating_key.to_string();
    let spawned = plx_base::task::spawn_off_frame("favourite", move |_| {
        let ok = write(&rk, favourite);
        landed(&rk, favourite, ok);
    });
    if !spawned {
        note(rating_key, !favourite);
    }
}

/// The worker's answer for one write: taken, Home owes a refetch; refused, the mark flips back.
fn landed(rating_key: &str, favourite: bool, ok: bool) {
    if ok {
        LANDED.store(true, Ordering::Release);
    } else {
        note(rating_key, !favourite);
    }
    plx_machine::present::wake_from_worker();
}

/// The server says `rating_key` is, or is not, a favourite (the socket's report of this user's
/// data changing, [`super::live`]). A mark that differs from the one held was made elsewhere, on
/// another device: it is taken, and Home owes the same refetch a write of this app's owes. This
/// app's own press already holds the mark it wrote, so its echo owes nothing more. Returns whether
/// the mark changed.
pub fn heard(rating_key: &str, favourite: bool) -> bool {
    if rating_key.is_empty() || state(rating_key) == favourite {
        return false;
    }
    note(rating_key, favourite);
    owe_home();
    true
}

/// Home's rows are out of date for a reason the server just named and the viewer should see at
/// once: a favourite made elsewhere ([`heard`]), or this user's Home settings edited
/// (`UserUpdated`: the order of the libraries, the ones left out of Recently Added, whether
/// played items show there). The app re-reads Home as it does after a favourite of its own.
pub fn owe_home() {
    LANDED.store(true, Ordering::Release);
    plx_machine::present::wake_from_worker();
}

/// Has a favourite write landed since the last call? FRAME THREAD: the app refetches Home's hubs
/// when it has, which is what puts the item on (or takes it off) the Favorites shelf; a mark is
/// in no listing the app already holds, so nothing short of the server's own answer can.
pub fn take_landed() -> bool {
    LANDED.swap(false, Ordering::AcqRel)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_noted_mark_reads_back_and_an_unknown_item_is_not_a_favourite() {
        note("fav-a", true);
        note("fav-b", false);
        assert!(state("fav-a") && !state("fav-b") && !state("fav-never-read"));
        note("fav-a", false);
        assert!(!state("fav-a"));
    }

    /// The landed flag is one per process: the tests that read it take turns.
    static FLAG: Mutex<()> = Mutex::new(());

    #[test]
    fn a_taken_write_owes_home_one_refetch_and_a_refused_one_flips_back() {
        let _turn = FLAG.lock().unwrap_or_else(|p| p.into_inner());
        note("fav-c", true);
        landed("fav-c", true, false);
        assert!(!state("fav-c"), "refused: the mark flips back");
        assert!(!take_landed(), "refused: Home's shelf did not change");
        note("fav-c", true);
        landed("fav-c", true, true);
        assert!(state("fav-c"));
        assert!(take_landed(), "taken: Home refetches");
        assert!(!take_landed(), "once");
    }

    #[test]
    fn a_mark_made_elsewhere_is_taken_and_owes_home_a_refetch_and_this_apps_echo_owes_nothing() {
        let _turn = FLAG.lock().unwrap_or_else(|p| p.into_inner());
        let _ = take_landed();
        assert!(heard("fav-d", true), "unknown here, a favourite there");
        assert!(state("fav-d") && take_landed());
        assert!(!heard("fav-d", true), "the echo of a mark already held");
        assert!(!heard("fav-e", false), "never a favourite, still not one");
        assert!(!take_landed());
        assert!(heard("fav-d", false) && !state("fav-d") && take_landed());
    }
}
