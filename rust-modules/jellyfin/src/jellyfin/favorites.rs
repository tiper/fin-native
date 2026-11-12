//! **Favourite marks**: what the app's `plx_plex::plex::favorite_mark` and friends answer for a
//! Jellyfin server.
//!
//! The mark travels beside the item model rather than in it: [`super::translate`] records each
//! item's mark as it reads the item ([`note`]), and the item menu and the detail page read it back
//! by ratingKey ([`state`]). A press flips the mark at once and writes it on a worker ([`set`]); a
//! write the server refuses flips it back, and one it takes comes back on the live-update socket
//! as this user's data changing ([`super::live`]), which re-reads Home and its Favorites shelf.

use std::collections::HashMap;
use std::sync::Mutex;

/// Marks remembered at once. Every listing the backend reads notes its items, so this bounds a
/// long browse; a mark forgotten here reads as "not a favourite" until the item is read again.
const KEPT: usize = 4096;

static MARKS: Mutex<Option<HashMap<String, bool>>> = Mutex::new(None);

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
    plx_machine::idle::invalidate();
    let rk = rating_key.to_string();
    let spawned = plx_base::task::spawn_off_frame("favourite", move |_| {
        let ok = write(&rk, favourite);
        landed(&rk, favourite, ok);
    });
    if !spawned {
        note(rating_key, !favourite);
    }
}

/// The worker's answer for one write: refused, the mark flips back. A write the server took
/// needs nothing here: the server reports it on the socket.
fn landed(rating_key: &str, favourite: bool, ok: bool) {
    if !ok {
        note(rating_key, !favourite);
    }
    plx_machine::idle::invalidate();
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

    #[test]
    fn a_refused_write_flips_the_mark_back_and_a_taken_one_keeps_it() {
        note("fav-c", true);
        landed("fav-c", true, false);
        assert!(!state("fav-c"), "refused: the mark flips back");
        note("fav-c", true);
        landed("fav-c", true, true);
        assert!(state("fav-c"));
    }
}
