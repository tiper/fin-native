//! The simulator's present seam: the two hooks the desktop platform states as its [`Present`]
//! (`host::SIM_PRESENT`), which run between a presenting frame's last draw and the swap.
//!
//! They live in the application because they draw: they name `plx_gfx` and the simulator's decoded
//! picture (`adapters::hostsim`), which a platform crate cannot reach. The platform states WHICH
//! hooks it runs (`Shell::frame`); the loop (`run::present_and_swap`) owns where they run. A television
//! states `Present::NONE` instead, so none of this is in its call path. `sim` builds only.

use crate::app::platform::Present;

/// **The television's compositor, over the frame the straighten pass just made.** The decoded
/// picture under the UI when `plxnative-simvideo` holds one, black otherwise
/// ([`plx_gfx::gfx::draw_under`]'s straight-alpha mix either way). The pass has no simulator switch, so
/// this is not optional on a bound frame: a shot of the straight buffer would be the pass's own
/// intermediate picture.
fn compose_under() {
    #[cfg(feature = "play-feed")]
    if crate::app::adapters::hostsim::composite_under() {
        return;
    }
    plx_gfx::gfx::draw_under_black();
}

/// Copy the supersampled offscreen frame to the window, so the loop still presents something to look at.
///
/// `present_supersampled` disables the scissor test itself (a clip left armed would crop the window
/// copy) - below gfx's `LIVE_SCISSOR`, which it cannot name. Clearing the clip through gfx first leaves
/// the two in step for the next frame.
fn before_swap() {
    plx_gfx::gfx::clip_clear();
    plx_base::surface::present_supersampled();
}

/// The simulator's [`Present`], which `host::SIM_PRESENT` hands the desktop platform.
pub(crate) const PRESENT: Present = Present { compose_under, before_swap };
