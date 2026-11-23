//! The Tizen media composition: what replaces `host::NO_MEDIA` when this crate is built with its `uri` feature.
//!
//! [`MEDIA`] is the platform's [`MediaBoot`]: no early init (the host's player needs nothing before the first
//! adapter), and a `compose` that installs the [`TizenUriDriver`] over the hosted contract and returns the resolver the
//! planner asks before it offers a URI source. No Feed is composed, no default backend is published (the set the
//! session is told is whatever the adapter's installed drivers form), and no LG binding is named or faked: the
//! webOS `UriBackend` stays in `plx_play_uri`, untouched.
//!
//! Three things keep the composition honest. A controlled run (a replay, a scripted Home) touches no native port, so it
//! installs nothing. A host whose table has no player group (a version-1 host, a NULL pointer) is not an error: the
//! composition logs the reason and returns `Composition::default()`, which is `compose: backends=none resolver=no`, the
//! same boot a build without `uri` has. And the resolver is returned only when the adapter accepted the driver, so it
//! always describes a driver that is installed.
use super::driver::{TizenUriDriver, Timing, CAPS};
use super::port::{HostPlayer, HostedPlayer};
use super::qualifier::TizenQualifier;
use plx_base::eventlog::log;
use plx_media::player::adapter::PlayerAdapter;
use plx_media::route::uri_preflight::{ProductEvidence, ResolvePort};
use plx_playback_contract::uri::{DisplayReadiness, UriDriver};
use plxnative_modules::host::{Composition, MainThread, MediaBoot};
use std::sync::Arc;

/// The platform's media boot.
pub(crate) const MEDIA: MediaBoot = MediaBoot { early_init: no_early_init, compose };

fn no_early_init(_: &MainThread) {}

fn compose(adapter: &mut PlayerAdapter, controlled: bool) -> Composition {
    compose_over(adapter, controlled, HostedPlayer::new(&crate::HOSTED))
}

/// The composition over `port`: the production [`compose`] passes the hosted contract's, a host test its fake.
pub(crate) fn compose_over<P: HostPlayer + 'static>(adapter: &mut PlayerAdapter, controlled: bool, port: P) -> Composition {
    if controlled {
        return Composition::default();
    }
    if let Err(why) = port.offered() {
        log(&format!("uri: no Tizen driver installed: {why}"));
        return Composition::default();
    }
    let driver = TizenUriDriver::new(port, Timing::PRODUCT);
    let display = driver.display();
    if adapter.install_uri_driver(Box::new(driver)).is_err() {
        log("uri: Tizen driver installation refused");
        return Composition::default();
    }
    let state = match display.readiness {
        DisplayReadiness::Ready => "ready".to_owned(),
        DisplayReadiness::NotReady(why) => format!("unavailable ({})", why.log_name()),
    };
    log(&format!("uri: display path={} {state}", display.tag));
    // The driver certifies itself: it is the platform's only path, so what it can call is what it certifies.
    let evidence = ProductEvidence { display, callable: CAPS, certified: CAPS };
    Composition { resolver: Some(ResolvePort::new(evidence, Arc::new(TizenQualifier))) }
}
