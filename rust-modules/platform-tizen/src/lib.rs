//! The Tizen platform: what the application is handed on a Samsung television (emulator only, for now).
//!
//! This crate will hold the Tizen platform value the application's `host` module takes: a hosted `.so`
//! entry the .NET host loads, an Ecore window the application draws into, the remote's keys and the
//! UDP log channel. It is empty in this change: the crate, its place in the module layers
//! (`[platform tizen]` in `ci/module-layers.ini`) and every list that names a platform crate exist so
//! the code that follows lands in a gated crate (`docs/platform-ports.md`, W9).
//!
//! Like the other platform crates it reaches the application through `plxnative_modules::host` alone,
//! names no other platform, and is named by nothing. The library is compiled only under `tizen` (its
//! own feature, in no default set), so a default-feature pass that lists this package builds an empty
//! library and no build of the television's or the desktop's ever contains it.
#![cfg(feature = "tizen")]
