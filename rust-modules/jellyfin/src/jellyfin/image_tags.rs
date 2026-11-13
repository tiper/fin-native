//! **Image tags**: the cache tag Jellyfin gives each image, which changes when the image does.
//!
//! The app asks for some images by a Plex-shaped path built from a ratingKey alone (a clearLogo at
//! `/library/metadata/{id}/clearLogo`), so the request carries nothing that changes when the server
//! gets a new one, and the disk cache would serve the old picture for good. [`super::translate`]
//! records each tag as it reads the item ([`note`]) and [`super::client::plex_image_path`] reads it
//! back ([`tag`]) to put it in the request, so a new image is a new request and a new cache entry.

use std::collections::HashMap;
use std::sync::Mutex;

use super::paths::ImageKind;

/// Tags remembered at once. Every listing the backend reads notes its items, so this bounds a
/// long browse; a tag forgotten here reads as "unknown" until the item is read again.
const KEPT: usize = 4096;

static TAGS: Mutex<Option<HashMap<(String, &'static str), String>>> = Mutex::new(None);

/// Record `tag` as the current tag of `item_id`'s image of `kind`, as the server stated it.
pub fn note(item_id: &str, kind: ImageKind, tag: &str) {
    if item_id.is_empty() || tag.is_empty() {
        return;
    }
    let mut tags = TAGS.lock().unwrap_or_else(|p| p.into_inner());
    let tags = tags.get_or_insert_with(HashMap::new);
    let key = (item_id.to_string(), kind.as_str());
    if tags.len() >= KEPT && !tags.contains_key(&key) {
        tags.clear();
    }
    tags.insert(key, tag.to_string());
}

/// The tag `item_id`'s image of `kind` had when last read, if it was read.
pub fn tag(item_id: &str, kind: ImageKind) -> Option<String> {
    let tags = TAGS.lock().unwrap_or_else(|p| p.into_inner());
    tags.as_ref()?.get(&(item_id.to_string(), kind.as_str())).cloned()
}

/// The key an item's kind is kept under, beside its image tags.
const KIND: &str = "Kind";

/// Remember what an item is (`movie`, `show`, ...), so its page can ask in one round for exactly
/// what that kind needs. A kind never changes, so this is a fact about the id, not a cached answer.
pub fn note_kind(item_id: &str, kind: &'static str) {
    if item_id.is_empty() {
        return;
    }
    let mut tags = TAGS.lock().unwrap_or_else(|p| p.into_inner());
    let tags = tags.get_or_insert_with(HashMap::new);
    let key = (item_id.to_string(), KIND);
    if tags.len() >= KEPT && !tags.contains_key(&key) {
        tags.clear();
    }
    tags.insert(key, kind.to_string());
}

/// The kind noted for an item, when a listing or page has shown it.
pub fn kind(item_id: &str) -> Option<String> {
    let tags = TAGS.lock().unwrap_or_else(|p| p.into_inner());
    tags.as_ref()?.get(&(item_id.to_string(), KIND)).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_tag_replaces_the_old_and_kinds_are_apart() {
        note("img-t-1", ImageKind::Logo, "l1");
        note("img-t-1", ImageKind::Primary, "p1");
        assert_eq!(tag("img-t-1", ImageKind::Logo).as_deref(), Some("l1"));
        note("img-t-1", ImageKind::Logo, "l2");
        assert_eq!(tag("img-t-1", ImageKind::Logo).as_deref(), Some("l2"));
        assert_eq!(tag("img-t-1", ImageKind::Primary).as_deref(), Some("p1"));
        assert_eq!(tag("img-t-unknown", ImageKind::Logo), None);
    }
}
