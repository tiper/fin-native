//! The persisted half of Library order: the record keyed by profile, its soft parse, and the
//! canonical store carrying it. The rules that read it are `plex::library_order`.
use super::*;
use serde_json::json;

fn order(session: &Session, user: &str) -> Vec<(String, i64)> {
    session.order_for(user).map(|record| record.libs.iter().map(|lib| (lib.machine_id.clone(), lib.key)).collect())
        .unwrap_or_default()
}

fn pairs(list: &[(&str, i64)]) -> Vec<(String, i64)> {
    list.iter().map(|(machine, key)| ((*machine).to_owned(), *key)).collect()
}

#[test]
fn a_session_that_never_reordered_writes_no_key_and_an_old_file_reads_as_server_order() {
    let value = serde_json::to_value(Session::default()).unwrap();
    assert!(value.get("library_order").is_none(), "absent while empty, so an older session round-trips as before");
    let old: Session = serde_json::from_value(json!({"client_id":"c","account_token":"t","home_pins":[]})).unwrap();
    assert!(old.prefs.library_order.is_empty() && old.order_for("").is_none());
}

#[test]
fn the_record_round_trips_and_is_kept_per_profile() {
    let mut session = Session::default();
    session.set_order_for("", &pairs(&[("m1", 2), ("m2", 7), ("m1", 1)]));
    session.set_order_for("kid", &pairs(&[("m2", 7)]));
    assert_eq!(order(&session, ""), pairs(&[("m1", 2), ("m2", 7), ("m1", 1)]));
    assert_eq!(order(&session, "kid"), pairs(&[("m2", 7)]), "another profile's record is its own");
    assert!(session.order_for("other").is_none());

    session.set_order_for("", &pairs(&[("m1", 1)]));
    assert_eq!(order(&session, ""), pairs(&[("m1", 1)]), "a later set replaces, never appends");
    assert_eq!(order(&session, "kid"), pairs(&[("m2", 7)]), "and leaves the other profile alone");

    let back: Session = serde_json::from_value(serde_json::to_value(&session).unwrap()).unwrap();
    assert_eq!(back.prefs.library_order, session.prefs.library_order);

    session.clear_order_for("");
    assert!(session.order_for("").is_none(), "reset deletes the profile's record");
    assert_eq!(order(&session, "kid"), pairs(&[("m2", 7)]), "reset is this profile's only");
}

#[test]
fn a_library_with_no_machine_id_is_not_recorded_and_an_empty_record_is_not_kept() {
    let mut session = Session::default();
    session.set_order_for("", &pairs(&[("", 1), ("m", 2)]));
    assert_eq!(order(&session, ""), pairs(&[("m", 2)]));
    session.set_order_for("", &pairs(&[("", 1)]));
    assert!(session.order_for("").is_none());
}

#[test]
fn one_hand_edited_entry_costs_that_entry_never_the_credentials() {
    let session: Session = serde_json::from_value(json!({
        "client_id": "c", "account_token": "t",
        "library_order": [{"user":"", "libs":[{"machine_id":"m","key":3}, 17, "x", {"machine_id":"n","key":1}]}, 5],
    })).unwrap();
    assert_eq!(session.account_token, "t");
    assert_eq!(order(&session, ""), pairs(&[("m", 3), ("n", 1)]));
    let garbage: Session = serde_json::from_value(json!({"account_token":"t","library_order":"zzz"})).unwrap();
    assert_eq!(garbage.account_token, "t");
    assert!(garbage.prefs.library_order.is_empty());
}

#[test]
fn the_canonical_store_carries_the_record_in_its_public_preferences() {
    let mut session = Session {
        client_id: "synthetic-client".into(), account_token: "synthetic-account".into(), ..Default::default()
    };
    session.set_order_for("", &pairs(&[("m1", 2), ("m2", 7)]));
    let (public, protected) = split_canonical(&session).unwrap();
    assert!(public.preferences.get("library_order").is_some(), "public: the record names no credential");
    assert!(!protected.contains("library_order"));
    let reopened = join_canonical(&public, &protected).unwrap();
    assert_eq!(order(&reopened, ""), pairs(&[("m1", 2), ("m2", 7)]));
}
