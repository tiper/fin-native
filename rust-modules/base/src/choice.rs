//! A value a platform publishes ONCE, before anything reads it, and the process then keeps.
//!
//! The shape `paths` first needed for [`Layout`](crate::paths::Layout) and `dynlib` needs again for
//! [`Libraries`](crate::dynlib::Libraries): a layer below the platform table (`paths`, `dynlib`) cannot
//! read `plx_os::tv::Services`, so the application's entry pushes the group in, once, and the layer
//! reads it from then on. Two users make it one mechanism rather than two copies of a `OnceLock` pair.
//!
//! The cell holds a choice, not a value: [`Choice::publish`] fills it with `Some`, and the first
//! [`Choice::get`] with nothing published fills it with `None`. Whichever arrives first stands, so a
//! publish that loses changes nothing and a read never sees a value that `publish` reported as refused.
//! **Neither call logs**: the layers that use this are the ones the event log itself resolves its path
//! and libraries through, so the caller reports a refusal (`Platform::install_services`).
use std::sync::OnceLock;

/// The one choice between what a platform published and what its layer derives or defaults to.
pub struct Choice<T>(OnceLock<Option<T>>);

/// Why [`Choice::publish`] refused. The loser changes nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublishError {
    /// A value was published already; the first one stands.
    AlreadyPublished,
    /// The value was read before it arrived, so the layer's own default is what the process uses.
    AlreadyRead,
}

impl std::fmt::Display for PublishError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::AlreadyPublished => "already published; the first one stands",
            Self::AlreadyRead => "read before it was published; the default stands",
        })
    }
}

impl<T> Choice<T> {
    pub const fn new() -> Self {
        Self(OnceLock::new())
    }

    /// The published value, if any. The first call with nothing published freezes the answer to `None`.
    pub fn get(&self) -> Option<&T> {
        self.0.get_or_init(|| None).as_ref()
    }

    /// Publish `value`, once, before the first [`get`](Self::get). A second publish and a publish after
    /// the first read are refused and change nothing.
    pub fn publish(&self, value: T) -> Result<(), PublishError> {
        if self.0.set(Some(value)).is_ok() {
            return Ok(());
        }
        // Lost: `wait` returns once whoever won has finished filling the cell.
        match self.0.wait() {
            Some(_) => Err(PublishError::AlreadyPublished),
            None => Err(PublishError::AlreadyRead),
        }
    }
}

impl<T> Default for Choice<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A second publish is refused and the first value stands; a publish after the first read is refused
    /// and leaves nothing behind; a read with nothing published answers `None` and freezes it.
    #[test]
    fn the_first_arrival_stands_and_the_loser_changes_nothing() {
        let c = Choice::new();
        assert_eq!(c.publish(1), Ok(()));
        assert_eq!(c.publish(2), Err(PublishError::AlreadyPublished));
        assert_eq!(c.get(), Some(&1), "the first value stands");

        let c = Choice::new();
        assert_eq!(c.get(), None, "nothing published: the layer's default");
        assert_eq!(c.publish(3), Err(PublishError::AlreadyRead));
        assert_eq!(c.get(), None, "a late publish leaves no value behind");
    }

    #[test]
    fn a_refusal_names_itself() {
        assert_eq!(PublishError::AlreadyPublished.to_string(), "already published; the first one stands");
        assert_eq!(PublishError::AlreadyRead.to_string(), "read before it was published; the default stands");
    }
}
