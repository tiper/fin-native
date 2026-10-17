//! **Launching straight into an item**: the launch parameters a launcher card, a home-screen
//! shortcut or `luna-send …/launch` hands this app, `{"itemId": "<ratingKey>", "play": true}`.
//!
//! The webOS shell starts a native app with its launch parameters as one JSON string in
//! `argv[1]`. Without `play` (or with `"play": false`) the item's page opens; with it the item
//! plays: a movie or episode from its resume point, a show from its next episode, the same choice a
//! card's Play makes (`input::activate_card` and its `MenuPlayAwait`, which this hands the request
//! to, falling back to the page when nothing on it plays).
//!
//! The request waits out the boot gates (sign-in, the profile picker, first-run consent and
//! onboarding) and is acted on the first frame Home is up with a server to ask; the item is read
//! from that server. A person who has already gone somewhere other than Home by then has moved on,
//! and the request is dropped rather than pulling them back.
//!
//! Only the cold launch is read. A launch that finds the app already running in the background
//! resumes it as it was.
//!
//! The idea and the parameter shape come from SchulzCode/WebOS-Jellyfin-Native (0BSD, `src/app.rs`
//! at 63daba7), which reads them from `parameters`; this reads that nesting and the flat object.

use plx_screens::registry::AppArg;

/// The longest item id accepted. A Plex `ratingKey` is a decimal number; the bound only keeps an
/// argument that is not an id at all from being carried into a request.
const KEY_MAX_LEN: usize = 64;

/// What a launch asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LaunchItem {
    /// The item's id on the server, as the server spells it.
    pub(crate) key: String,
    /// Play it rather than open its page.
    pub(crate) play: bool,
}

/// The launch parameters of this process, if they name an item.
pub(crate) fn from_process_args() -> Option<LaunchItem> {
    std::env::args_os().nth(1).and_then(|a| a.into_string().ok()).as_deref().and_then(parse)
}

/// One launch-parameter string read for an item: `itemId` (a string, or a number) and `play`,
/// either at the top level or under `parameters`. Anything else, including an id that is empty,
/// too long or not made of letters, digits and dashes, is no item.
pub(crate) fn parse(arg: &str) -> Option<LaunchItem> {
    let v: serde_json::Value = serde_json::from_str(arg).ok()?;
    let p = match v.get("parameters") {
        Some(nested) if nested.is_object() => nested,
        _ => &v,
    };
    let key = match p.get("itemId")? {
        serde_json::Value::String(s) => s.trim().to_string(),
        serde_json::Value::Number(n) if n.is_u64() => n.to_string(),
        _ => return None,
    };
    let well_formed = !key.is_empty()
        && key.len() <= KEY_MAX_LEN
        && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
    if !well_formed {
        return None;
    }
    let play = p.get("play").and_then(serde_json::Value::as_bool).unwrap_or(false);
    Some(LaunchItem { key, play })
}

/// What a pending launch does on this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    /// Still behind a boot gate, or Home has no server to ask yet.
    Wait,
    /// Home is up with a server: open or play the item now.
    Act,
    /// The person is already somewhere else: forget the request.
    Drop,
}

/// The decision, from the page on top and whether the current server has a client.
pub(crate) fn step(route: &AppArg, server_ready: bool) -> Step {
    match route {
        AppArg::Login | AppArg::Profiles | AppArg::Onboard | AppArg::FirstRunConsent(_) => Step::Wait,
        AppArg::Home if server_ready => Step::Act,
        AppArg::Home => Step::Wait,
        _ => Step::Drop,
    }
}

/// Act on a pending launch once [`step`] says so. Called every frame beside the card Play's own
/// continuation (`app/run.rs`); one `Option` check while nothing is pending.
pub(crate) fn tick(app: &mut super::App, now: u32) {
    let Some(item) = app.launch.as_ref() else { return };
    let sid = plx_plex::plex::current_server();
    match step(&app.route(), plx_plex::plex::client_for(sid).is_some()) {
        Step::Wait => {}
        Step::Drop => {
            plx_base::eventlog::log("launch: the item request was dropped (Home was left before it could open)");
            app.launch = None;
        }
        Step::Act => {
            let LaunchItem { key, play } = item.clone();
            app.launch = None;
            plx_base::eventlog::log(&format!("launch: item={key} play={play} server={}", sid.raw()));
            if play {
                app.bridge.metadata_mut().run(plx_data::stores::metadata::MetadataCmd::RequestDetail { sid, rk: key.clone() });
                app.menu_play_await = Some(super::input::MenuPlayAwait::new(
                    sid, key, None, plx_screens::player::input::HUD_LINGER_MS, now));
            } else {
                super::bridge::open_detail(&mut app.pages, &mut app.bridge, sid, &key, None, None);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(key: &str, play: bool) -> Option<LaunchItem> {
        Some(LaunchItem { key: key.into(), play })
    }

    #[test]
    fn an_item_id_opens_its_page_unless_play_is_asked() {
        assert_eq!(parse(r#"{"itemId":"12345"}"#), item("12345", false));
        assert_eq!(parse(r#"{"itemId":"12345","play":false}"#), item("12345", false));
        assert_eq!(parse(r#"{"itemId":"12345","play":true}"#), item("12345", true));
    }

    #[test]
    fn the_parameters_nesting_and_a_numeric_id_are_read() {
        assert_eq!(parse(r#"{"@system_native_app":true,"parameters":{"itemId":"77","play":true}}"#), item("77", true));
        assert_eq!(parse(r#"{"itemId":77}"#), item("77", false));
        assert_eq!(parse(r#"{"itemId":"f137a2dd-21bb-c1b9-9aa5-c0f6bf02a805"}"#),
            item("f137a2dd-21bb-c1b9-9aa5-c0f6bf02a805", false), "a GUID-shaped id is carried as written");
    }

    #[test]
    fn anything_that_is_not_an_item_id_is_no_launch_item() {
        for arg in [
            "", "not json", "{}", r#"{"play":true}"#, r#"{"itemId":""}"#, r#"{"itemId":"  "}"#,
            r#"{"itemId":"../library/sections"}"#, r#"{"itemId":"1?X-Plex-Token=a"}"#, r#"{"itemId":-3}"#,
            r#"{"itemId":1.5}"#, r#"{"itemId":true}"#, r#"{"itemId":["1"]}"#,
        ] {
            assert_eq!(parse(arg), None, "{arg}");
        }
        let long = format!(r#"{{"itemId":"{}"}}"#, "1".repeat(KEY_MAX_LEN + 1));
        assert_eq!(parse(&long), None, "an over-long id");
        let longest = format!(r#"{{"itemId":"{}"}}"#, "1".repeat(KEY_MAX_LEN));
        assert!(parse(&longest).is_some(), "the longest id still reads");
    }

    #[test]
    fn a_launch_waits_out_the_boot_gates_and_acts_on_home_with_a_server() {
        for (name, gate) in [("login", AppArg::Login), ("profiles", AppArg::Profiles), ("onboard", AppArg::Onboard),
            ("consent", AppArg::FirstRunConsent(0))] {
            assert_eq!(step(&gate, true), Step::Wait, "{name}");
        }
        assert_eq!(step(&AppArg::Home, false), Step::Wait, "Home before its server is up");
        assert_eq!(step(&AppArg::Home, true), Step::Act);
    }

    #[test]
    fn a_launch_is_dropped_once_the_person_is_elsewhere() {
        for (name, elsewhere) in [("library", AppArg::Library), ("search", AppArg::Search), ("player", AppArg::Player)] {
            assert_eq!(step(&elsewhere, true), Step::Drop, "{name}");
        }
    }
}
