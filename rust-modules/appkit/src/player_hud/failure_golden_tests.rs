//! **The failure read-out, pinned for every cause, setting and offer.**
//!
//! A snapshot (`failure_readout.golden`) of everything the read-out puts on screen or on the wire for
//! one failure: the stable code, the caption / panel / reason / detail words, the Plex Pass flag, the
//! glyph, the note, the action row (in draw order, as the accessibility words), and the local
//! measurement's `handled_failure`. It is enumerated over the product of
//!
//! * the CAUSE: every failtest arm, every `PlayVerdict`, the jail, the refused native starts and a
//!   session with no cause at all;
//! * the SETTING: Direct Play Auto, Forced and Disabled;
//! * the OFFERS: whether the failed attempt has a request to resolve again, whether it is a channel,
//!   whether the item has tracks and whether the sandbox repair is idle (and, for the jail's own
//!   words, every repair state).
//!
//! It exists so that the way the read-out is PRODUCED can change without its output changing: the
//! only function here that touches the producer is [`produce`]. A diff of the golden is a change of
//! behaviour, never of plumbing. Regenerate it (and say why in the commit) with
//! `PLX_BLESS_FAILURE_GOLDEN=1 make test-crate C=plx_appkit T=failure_readout_is_pinned`.

use super::{failure_action_word, FailureReadout};
use plx_media::player::{self as pl, FailureAction};
use plx_media::route as rt;
use plx_os::tv::native_video::{Failure as RepairFailure, State as Repair};
use plx_plex::plex::session::DirectPlayMode;

const GOLDEN: &str = include_str!("failure_readout.golden");

/// What one failure puts on screen and on the wire, apart from the action row (which depends on the
/// offers and is enumerated separately).
struct Produced {
    head: String,
    /// The read-out's glyph, reason and note for each repair state, which only the jail's words read.
    by_repair: Vec<(Repair, String)>,
    row: Vec<FailureAction>,
}

/// THE producer under test: the failure the session (or the failtest `arm`) stands for, through the
/// code that builds the read-out, with `tracks` and `repair` as the live inputs they are.
fn produce(ps: &rt::PlaybackSession, arm: Option<&str>, tracks: bool) -> Produced {
    let repair = ps.repair_status;
    // the derivation `observe_failure` records (the capture's gating on the state is graded in `plx_media`)
    let failure = pl::failure_under_for_test(ps, arm);
    let error = failure.shape().clone();
    let live = pl::LiveOffers { repair_idle: repair == Repair::Idle, tracks_available: tracks, versions_available: false };
    let row = pl::failure_actions(&failure, live);
    let handled = pl::backend::handled_failure(error.kind);
    let head = format!(
        "{} | {:?} | {:?} | {:?} | {:?} | {:?} | pass={} | {:?}",
        error.kind.code(), error.caption, error.panel, error.readout, error.detail, error.kind, error.no_pass, handled,
    );
    let code = error.kind.code();
    let support = pl::support_line(error.kind);
    assert!(support.contains(code), "the footer names the stable code: {support}");
    let readout = FailureReadout::captured(failure.attempt(), error, row.clone(), support, repair);
    let view = format!("{:?} | {:?} | {:?} | footer carries the code={}",
        readout.glyph, readout.reason, readout.note, readout.footer.to_str().unwrap_or("").contains(code));
    Produced { head, by_repair: vec![(repair, view)], row }
}

fn words(row: &[FailureAction]) -> String {
    row.iter().map(|a| failure_action_word(*a)).collect::<Vec<_>>().join(",")
}

enum Cause {
    Arm(&'static str),
    Verdict(rt::PlayVerdict),
    ForcedAudioAfterLoad,
    Jail,
    StartRefused(rt::StartRefusal),
    Bare,
}

fn causes() -> Vec<(String, Cause)> {
    let mut out = Vec::new();
    for arm in ["verdict", "audio", "novideo", "stream", "connection", "tv", "load_timeout", "jail", "policy", "disabled",
        "no_backend", "none", "an-unknown-arm"] {
        out.push((format!("arm:{arm}"), Cause::Arm(arm)));
    }
    for (name, v) in [
        ("server", rt::PlayVerdict::Server("Cannot convert this item.".into(), Default::default())),
        ("server-empty", rt::PlayVerdict::Server(String::new(), Default::default())),
        ("forced-no-original", rt::PlayVerdict::Forced(rt::ForcedFailure::NoOriginal)),
        ("forced-container", rt::PlayVerdict::Forced(rt::ForcedFailure::Container)),
        ("forced-video", rt::PlayVerdict::Forced(rt::ForcedFailure::Video)),
        ("forced-audio", rt::PlayVerdict::Forced(rt::ForcedFailure::Audio { codec: "DTS".into() })),
        ("forced-unauthorized", rt::PlayVerdict::Forced(rt::ForcedFailure::Unauthorized)),
        ("forced-open-failed", rt::PlayVerdict::Forced(rt::ForcedFailure::OpenFailed)),
        ("disabled", rt::PlayVerdict::DirectPlayDisabled),
        ("no-backend", rt::PlayVerdict::NoQualifiedBackend),
        ("preview", rt::PlayVerdict::PreviewNeedsDirectPlay {
            reason: plx_telemetry::telemetry::classes::PreviewRefusal::NotDirect }),
        ("tuners", rt::PlayVerdict::TunersBusy),
    ] {
        out.push((format!("verdict:{name}"), Cause::Verdict(v)));
    }
    out.push(("forced-audio-after-load".into(), Cause::ForcedAudioAfterLoad));
    out.push(("jail".into(), Cause::Jail));
    out.push(("start-refused:load".into(), Cause::StartRefused(rt::StartRefusal::LoadTimeout)));
    out.push(("start-refused:release".into(), Cause::StartRefused(rt::StartRefusal::ReleaseTimeout)));
    out.push(("bare".into(), Cause::Bare));
    out
}

fn session(cause: &Cause, mode: DirectPlayMode, request: bool, live: bool) -> rt::PlaybackSession {
    let mut ps = rt::PlaybackSession::IDLE;
    rt::set_direct_play_mode_for_test(&mut ps, mode);
    match cause {
        Cause::Arm(_) | Cause::Bare => {}
        Cause::Verdict(v) => rt::refuse_for_test(&mut ps, v.clone()),
        Cause::ForcedAudioAfterLoad => rt::name_refused_forced_audio(&mut ps, "dca"),
        Cause::Jail => ps.jail_load_blocked = true,
        Cause::StartRefused(r) => ps.native_start_refused = Some(*r),
    }
    if request {
        rt::seed_request_for_test(&mut ps, "42");
    }
    if live {
        rt::mark_live_for_test(&mut ps);
    }
    ps
}

fn arm_of(cause: &Cause) -> Option<&'static str> {
    match cause {
        Cause::Arm(a) => Some(a),
        _ => None,
    }
}

const REPAIRS: [Repair; 4] = [Repair::Idle, Repair::Running, Repair::Repaired, Repair::Failed(RepairFailure::Timeout)];

fn repair_name(r: Repair) -> &'static str {
    match r {
        Repair::Idle => "idle",
        Repair::Running => "running",
        Repair::Repaired => "repaired",
        Repair::Failed(_) => "failed",
    }
}

/// The masks of a group, or `every offer` when the one row answers all `of` of them.
fn masks_of(masks: &[String], of: usize) -> String {
    if masks.len() == of { "every offer".to_owned() } else { masks.join(" ") }
}

/// Add `mask` under `key` in first-seen order (a snapshot's order is its determinism).
fn push<K: PartialEq>(groups: &mut Vec<(K, Vec<String>)>, key: K, mask: String) {
    match groups.iter_mut().find(|(k, _)| *k == key) {
        Some((_, masks)) => masks.push(mask),
        None => groups.push((key, vec![mask])),
    }
}

fn snapshot() -> String {
    use std::fmt::Write;
    let mut out = String::new();
    // ---- 1. the table: every kind x setting x offer, without a session ----------------------------
    writeln!(out, "# the action table (kind, setting): rows by offers [r=can retry, q=quality carriable, t=tracks, i=repair idle]").unwrap();
    let mut table: Vec<((String, &str), Vec<(String, Vec<String>)>)> = Vec::new();
    // The same table with the failed item's versions listed (`LiveOffers::versions_available`): only the rows that
    // gain *Select Version* are written, under their own heading, so the one-file table above stays what it was.
    let mut with_versions: Vec<((String, &str), Vec<(String, Vec<String>)>)> = Vec::new();
    let mut without: std::collections::HashMap<(String, &str, String), String> = std::collections::HashMap::new();
    let every = pl::every_failure_row();
    let setting_of = |failure: &pl::PlaybackFailure| if failure.forced() { "forced" } else if failure.direct_play_disabled() { "disabled" } else { "auto" };
    let mask_of = |failure: &pl::PlaybackFailure, live: &pl::LiveOffers| {
        format!("r{}q{}t{}i{}", failure.can_retry() as u8, failure.quality_carriable() as u8, live.tracks_available as u8, live.repair_idle as u8)
    };
    for (failure, live, row) in every.iter().filter(|(_, live, _)| !live.versions_available) {
        let (setting, mask) = (setting_of(failure), mask_of(failure, live));
        let key = (format!("{:?}", failure.kind()), setting);
        without.insert((key.0.clone(), setting, mask.clone()), words(row));
        let entry = match table.iter_mut().find(|(k, _)| *k == key) {
            Some((_, rows)) => rows,
            None => {
                table.push((key, Vec::new()));
                &mut table.last_mut().unwrap().1
            }
        };
        push(entry, words(row), mask);
    }
    for (failure, live, row) in every.iter().filter(|(_, live, _)| live.versions_available) {
        let (setting, mask) = (setting_of(failure), mask_of(failure, live));
        let key = (format!("{:?}", failure.kind()), setting);
        if without.get(&(key.0.clone(), setting, mask.clone())) == Some(&words(row)) {
            continue;
        }
        let entry = match with_versions.iter_mut().find(|(k, _)| *k == key) {
            Some((_, rows)) => rows,
            None => {
                with_versions.push((key, Vec::new()));
                &mut with_versions.last_mut().unwrap().1
            }
        };
        push(entry, words(row), mask);
    }
    for ((kind, setting), rows) in &table {
        for (row, masks) in rows {
            writeln!(out, "{kind} {setting}: [{row}] <- {}", masks_of(masks, 16)).unwrap();
        }
    }
    writeln!(out, "\n# the same with the failed item's versions listed: only the rows that gain select_version (the row has four slots)").unwrap();
    for ((kind, setting), rows) in &with_versions {
        for (row, masks) in rows {
            writeln!(out, "{kind} {setting}: [{row}] <- {}", masks_of(masks, 16)).unwrap();
        }
    }
    // ---- 2. the sessions: every cause x setting x offer -------------------------------------------
    writeln!(out, "\n# the sessions (cause, setting)").unwrap();
    for (name, cause) in causes() {
        for (mode_name, mode) in [("auto", DirectPlayMode::Auto), ("forced", DirectPlayMode::Forced), ("disabled", DirectPlayMode::Disabled)] {
            writeln!(out, "\n## {name} / {mode_name}").unwrap();
            let arm = arm_of(&cause);
            let mut head: Option<String> = None;
            let mut rows: Vec<(String, Vec<String>)> = Vec::new();
            for request in [false, true] {
                for live in [false, true] {
                    for tracks in [false, true] {
                        for repair in [Repair::Idle, Repair::Running] {
                            let mut ps = session(&cause, mode, request, live);
                            ps.repair_status = repair;
                            let made = produce(&ps, arm, tracks);
                            // The words do not depend on the offers: one head per (cause, setting).
                            match &head {
                                None => head = Some(made.head.clone()),
                                Some(h) => assert_eq!(*h, made.head, "{name}/{mode_name}: the words moved with the offers"),
                            }
                            let mask = format!("r{}l{}t{}i{}", request as u8, live as u8, tracks as u8, (repair == Repair::Idle) as u8);
                            push(&mut rows, words(&made.row), mask);
                        }
                    }
                }
            }
            writeln!(out, "{}", head.unwrap()).unwrap();
            for (row, masks) in &rows {
                writeln!(out, "row [{row}] <- {}", masks_of(masks, 16)).unwrap();
            }
            // The read-out's own words, for every repair state (only the jail's change with it).
            let mut views: Vec<(String, Vec<String>)> = Vec::new();
            for repair in REPAIRS {
                let mut ps = session(&cause, mode, true, false);
                ps.repair_status = repair;
                let made = produce(&ps, arm, true);
                push(&mut views, made.by_repair[0].1.clone(), repair_name(repair).to_owned());
            }
            for (view, repairs) in &views {
                writeln!(out, "view {view} <- {}", repairs.join(",")).unwrap();
            }
        }
    }
    out
}

#[test]
fn failure_readout_is_pinned_for_every_cause_setting_and_offer() {
    let _serial = plx_base::testlock::serial();
    let now = snapshot();
    if std::env::var_os("PLX_BLESS_FAILURE_GOLDEN").is_some() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/player_hud/failure_readout.golden");
        std::fs::write(&path, &now).expect("write the golden");
        return;
    }
    if now != GOLDEN {
        let first = now.lines().zip(GOLDEN.lines()).position(|(a, b)| a != b);
        let at = first.unwrap_or(now.lines().count().min(GOLDEN.lines().count()));
        panic!("the failure read-out changed; first difference at line {}:\n  now:    {}\n  golden: {}",
            at + 1, now.lines().nth(at).unwrap_or("<end>"), GOLDEN.lines().nth(at).unwrap_or("<end>"));
    }
}
