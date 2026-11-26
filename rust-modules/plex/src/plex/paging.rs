//! The one place a Plex request spells its paging parameters.
//!
//! PMS answers a long listing in windows: `X-Plex-Container-Start` is the offset of the first row
//! wanted and `X-Plex-Container-Size` how many rows. The two travel together (see `params.rs`: a
//! lone size is ignored), so every paged request is built here, and a gate in `ci/check-deps.sh`
//! keeps the two names from being spelled anywhere else in non-test Rust.

/// A window of a server listing: `size` rows starting at row `start`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageReq {
    pub start: usize,
    pub size: usize,
}

impl PageReq {
    /// The window for the `i64` counts the Client methods take. A negative offset or size has no
    /// meaning on the wire, so it is sent as zero rather than as a wrapped `usize`.
    pub(super) fn window(start: i64, size: i64) -> Self {
        Self { start: start.max(0) as usize, size: size.max(0) as usize }
    }
}

/// `path` with the window's two parameters appended. A path that already carries a query string
/// takes `&`, a bare path takes `?`; the parameters always go last, in `Start`, `Size` order.
pub fn paged_path(path: &str, req: PageReq) -> String {
    let sep = if path.contains('?') { '&' } else { '?' };
    format!("{path}{sep}X-Plex-Container-Start={}&X-Plex-Container-Size={}", req.start, req.size)
}

/// `key` with any window parameters it already carries removed, every other part kept in order.
/// A provider's listing key can arrive with its own window (a hub's `…&X-Plex-Container-Size=12`),
/// and re-windowing it must replace that window rather than stack a second one on it.
pub(super) fn without_window(key: &str) -> String {
    let (path, query) = key.split_once('?').unwrap_or((key, ""));
    let query = query.split('&').filter(|part| !part.is_empty())
        .filter(|part| !matches!(part.split('=').next(), Some("X-Plex-Container-Start" | "X-Plex-Container-Size")))
        .collect::<Vec<_>>().join("&");
    if query.is_empty() { path.to_owned() } else { format!("{path}?{query}") }
}

/// The value of the query parameter `name` in `key`, as written (the first one when it is repeated):
/// `query_value("/hubs/home/recentlyAdded?type=2&sectionID=7", "sectionID")` is `Some("7")`. `None`
/// for a key with no query, no such parameter, or a bare `name` with no `=`; a parameter whose name only
/// begins with `name` (`sectionIDs=`) is another parameter.
pub fn query_value<'a>(key: &'a str, name: &str) -> Option<&'a str> {
    key.split_once('?')?.1.split('&').find_map(|pair| pair.strip_prefix(name)?.strip_prefix('='))
}

#[cfg(test)]
mod tests {
    use super::{paged_path, query_value, PageReq};

    #[test]
    fn a_query_value_is_the_text_after_the_equals_of_the_named_parameter() {
        let key = "/hubs/home/recentlyAdded?type=2&sectionID=7&sectionIDs=8";
        assert_eq!(query_value(key, "sectionID"), Some("7"));
        assert_eq!(query_value(key, "type"), Some("2"));
        assert_eq!(query_value(key, "sectionIDs"), Some("8"));
        assert_eq!(query_value("/a?x=1&x=2", "x"), Some("1"), "the first of a repeated parameter");
        assert_eq!(query_value("/a?x=", "x"), Some(""), "an empty value is a value");
    }

    #[test]
    fn a_query_value_is_none_without_that_parameter() {
        assert_eq!(query_value("/hubs/home/recentlyAdded", "sectionID"), None, "no query");
        assert_eq!(query_value("/hubs/home/recentlyAdded?type=2", "sectionID"), None);
        assert_eq!(query_value("/a?sectionID", "sectionID"), None, "a bare name has no value");
        assert_eq!(query_value("/a?xsectionID=3", "sectionID"), None, "a longer name is another parameter");
        assert_eq!(query_value("/a?sectionIDx=3", "sectionID"), None);
        assert_eq!(query_value("", "sectionID"), None);
    }

    #[test]
    fn a_bare_path_takes_the_window_after_a_question_mark() {
        assert_eq!(
            paged_path("/library/sections/7/collections", PageReq { start: 40, size: 20 }),
            "/library/sections/7/collections?X-Plex-Container-Start=40&X-Plex-Container-Size=20",
        );
    }

    #[test]
    fn a_path_with_a_query_takes_the_window_after_an_ampersand() {
        assert_eq!(
            paged_path("/library/sections/7/all?sort=addedAt%3Adesc", PageReq { start: 0, size: 60 }),
            "/library/sections/7/all?sort=addedAt%3Adesc&X-Plex-Container-Start=0&X-Plex-Container-Size=60",
        );
    }
}
