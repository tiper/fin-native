use super::*;
use crate::stores::{Admission, StoreId, StoreJob, Stores};

fn sid(n: u16) -> ServerId { ServerId::from_raw(n) }
fn keys(n: u16) -> ClientKeys {
    ClientKeys::from_keys((0..n).map(|n| (sid(n), ClientKey { instance: n as u32 + 1, token: 1, grant: 1 })))
}
fn input(keys: &ClientKeys, now: i64) -> LiveTvInput<'_> {
    LiveTvInput { keys, now: Some(now), recent: RecentSeat::Unseated }
}
fn pump(stores: &mut Stores, keys: &ClientKeys, now: i64) -> bool { stores.livetv_pump(&input(keys, now)) }
fn run(stores: &mut Stores, cmd: LiveTvCmd, keys: &ClientKeys, now: i64) -> bool { stores.livetv_run(cmd, &input(keys, now)) }
fn take(stores: &mut Stores) -> (Job<LiveTvReq>, LiveTvPort) {
    let mut jobs = stores.take_jobs_of(StoreId::LiveTv);
    assert_eq!(jobs.len(), 1);
    let job = jobs.pop().unwrap();
    assert_eq!(job.admission(), Admission::Direct);
    let StoreJob::LiveTv(job, port) = job else { panic!("Live TV emits its own job") };
    (job, port)
}
fn guide(numbers: &[&str], end: i64) -> Guide {
    Guide { dvr: "7".into(), provider: "tv.plex.providers.epg.xmltv:7".into(), refresh_at: end, retry_cd: 0, strikes: 0, until: 0, far_at: 0, channels: numbers.iter()
        .map(|number| LiveChannel { dvr: "7".into(), channel: format!("C{number}"), vcn: number.to_string(),
            name: "Channel".into(), thumb: "/logo".into(),
            airings: Airings::of(vec![LiveAiring { thumb: "/show".into(), ..LiveAiring::new("Show", 0, end) }], 0) }).collect() }
}
fn complete(guides: Vec<Guide>) -> ReadResult { ReadResult { guides, roster: None, complete: true, retry: false } }
fn answer(stores: &mut Stores, result: Option<Vec<Guide>>) {
    let (job, port) = take(stores);
    finish_with(&port, job.addr, job.req, |_| result.map(complete));
}

#[test]
fn livetv_initial_job_and_ready_publication_keep_the_whole_sorted_lineup() {
    let mut stores = Stores::default();
    let keys = keys(2);
    assert!(!pump(&mut stores, &keys, 100));
    assert_eq!(stores.livetv_view().status, LiveTvStatus::Loading);
    let jobs = stores.take_jobs_of(StoreId::LiveTv);
    assert_eq!(jobs.len(), 2, "every registered server is asked, independently of Home");
    for job in jobs {
        assert_eq!(job.store(), StoreId::LiveTv);
        let StoreJob::LiveTv(job, port) = job else { unreachable!() };
        let (sid, key) = job.req.server();
        assert_eq!(keys.key(sid), Some(key));
        assert!(matches!(job.req.work, Work::Discover));
        assert_eq!(job.req.now, 100);
        assert_eq!(job.addr.to, MachineId::Store(StoreId::LiveTv.ord()));
        let numbers: Vec<_> = (1..=30).rev().map(|n| (n + sid.raw() as i32 * 30).to_string()).collect();
        finish_with(&port, job.addr, job.req, |_| Some(complete(vec![guide(&numbers.iter().map(String::as_str).collect::<Vec<_>>(), 200)])));
    }
    assert!(pump(&mut stores, &keys, 100));
    let view = stores.livetv_view();
    assert_eq!(view.status, LiveTvStatus::Ready);
    assert_eq!(view.channels.len(), 60, "no 24-card cap on the lineup");
    assert_eq!(view.channels.iter().map(|c| c.vcn.parse::<usize>().unwrap()).collect::<Vec<_>>(), (1..=60).collect::<Vec<_>>());
    assert_eq!(view.tiles.len(), 60);
    assert_eq!(view.tiles[0].title, "1 Channel");
    assert_eq!(view.tiles[0].show_title, "Show");
    assert_eq!(view.tiles[0].thumb, "/logo");
    assert_eq!(live_channel_of(&view.tiles[0]), Some(("7", "C1")));
    assert_eq!(channel_name_of(&view.tiles[0]), Some("Channel"));
    let revision = view.revision();
    assert!(stores.take_notices().iter().any(|(id, _)| *id == StoreId::LiveTv));
    assert!(!pump(&mut stores, &keys, 100));
    assert_eq!(stores.livetv_view().revision(), revision);
    assert!(stores.take_jobs_of(StoreId::LiveTv).is_empty());
    assert!(stores.take_notices().is_empty());
}

#[test]
fn livetv_subchannel_numbers_publish_in_guide_order() {
    let mut stores = Stores::default(); let keys = keys(1);
    pump(&mut stores, &keys, 100);
    answer(&mut stores, Some(vec![guide(&["10.1", "2.1", "2"], 200)]));
    assert!(pump(&mut stores, &keys, 100));
    assert_eq!(stores.livetv_view().channels.iter().map(|c| c.vcn.as_str()).collect::<Vec<_>>(), ["2", "2.1", "10.1"]);
}

#[test]
fn livetv_answered_no_dvr_is_empty_and_does_not_retry() {
    let mut stores = Stores::default(); let keys = keys(1);
    pump(&mut stores, &keys, 100);
    answer(&mut stores, Some(Vec::new()));
    assert!(pump(&mut stores, &keys, 100));
    assert_eq!(stores.livetv_view().status, LiveTvStatus::Empty);
    for _ in 0..RETRY_FRAMES + 1 { pump(&mut stores, &keys, 500); }
    assert!(!stores.has_jobs());
}

#[test]
fn livetv_failure_and_refusal_back_off_and_retry_once() {
    for refusal in [false, true] {
        let mut stores = Stores::default(); let keys = keys(1);
        pump(&mut stores, &keys, 100);
        let (job, port) = take(&mut stores);
        if refusal { port.refused(job.addr); } else { finish_with(&port, job.addr, job.req, |_| None); }
        assert!(pump(&mut stores, &keys, 100));
        assert_eq!(stores.livetv_view().status, LiveTvStatus::Failed);
        for _ in 0..RETRY_FRAMES - 1 {
            pump(&mut stores, &keys, 100);
            assert!(!stores.has_jobs(), "no per-frame launch loop");
        }
        pump(&mut stores, &keys, 100);
        let (job, port) = take(&mut stores);
        assert!(matches!(job.req.work, Work::Discover));
        port.refused(job.addr);
        // A second failure in a row waits twice as long before the next read.
        for _ in 0..2 * RETRY_FRAMES {
            pump(&mut stores, &keys, 100);
            assert!(!stores.has_jobs(), "the wait doubles after a second failure");
        }
        pump(&mut stores, &keys, 100);
        let (job, port) = take(&mut stores);
        finish_with(&port, job.addr, job.req, |_| Some(complete(vec![guide(&["1"], 200)])));
        pump(&mut stores, &keys, 100);
        assert_eq!(stores.livetv_view().status, LiveTvStatus::Ready);
    }
}

#[test]
fn livetv_missing_key_marks_failed_without_emitting_or_spinning() {
    let mut keys = ClientKeys::default();
    keys.refill_roster(ServerId::UNSET, 1, 1, [sid(0)], |_| None, |_| None, |_| None);
    let mut stores = Stores::default();
    assert!(pump(&mut stores, &keys, 100));
    assert_eq!(stores.livetv_view().status, LiveTvStatus::Failed);
    for _ in 0..RETRY_FRAMES { pump(&mut stores, &keys, 100); assert!(!stores.has_jobs()); }
}

#[test]
fn livetv_airing_end_emits_one_grid_only_refresh_and_failure_keeps_lineup() {
    let mut stores = Stores::default(); let keys = keys(1);
    pump(&mut stores, &keys, 100);
    answer(&mut stores, Some(vec![guide(&["10", "2"], 200)]));
    pump(&mut stores, &keys, 199);
    assert!(!stores.has_jobs());
    pump(&mut stores, &keys, 200);
    let (job, port) = take(&mut stores);
    assert!(matches!(&job.req.work, Work::Grid(g) if g.dvr == "7" && g.channels.len() == 2));
    assert_eq!(job.req.now, 200);
    for _ in 0..5 { pump(&mut stores, &keys, 210); assert!(!stores.has_jobs()); }
    finish_with(&port, job.addr, job.req, |_| None);
    let previous = stores.livetv_view().channels.to_vec();
    let revision = stores.livetv_view().revision();
    pump(&mut stores, &keys, 210);
    assert_eq!(stores.livetv_view().status, LiveTvStatus::Ready);
    assert_eq!(stores.livetv_view().channels, previous);
    assert_eq!(stores.livetv_view().revision(), revision);
    for _ in 0..RETRY_FRAMES - 1 { pump(&mut stores, &keys, 210); assert!(!stores.has_jobs()); }
    pump(&mut stores, &keys, 210);
    let (job, port) = take(&mut stores);
    assert!(matches!(job.req.work, Work::Grid(_)));
    let mut next = guide(&["10", "2"], 300);
    next.channels[0].airings = Airings::of(vec![LiveAiring::new("Next show", 0, 300)], 0);
    finish_with(&port, job.addr, job.req, |_| Some(complete(vec![next])));
    assert!(pump(&mut stores, &keys, 211));
    assert!(stores.livetv_view().revision() > revision);
    assert_eq!(stores.livetv_view().channels.len(), 2);
    assert!(stores.livetv_view().channels.iter().any(|c| c.airings.now_title() == "Next show"));
}

/// **A grid that keeps failing is not asked every two seconds for ever.** A failed grid read was
/// retried at a fixed [`RETRY_FRAMES`] for as long as it failed, with the page closed too; only the
/// lineup read backed off. Consecutive failures double the wait like the lineup's, and a good read
/// starts over (PR #993).
#[test]
fn livetv_consecutive_grid_failures_double_the_wait_and_a_good_read_resets_it() {
    let mut stores = Stores::default(); let keys = keys(1);
    pump(&mut stores, &keys, 100);
    answer(&mut stores, Some(vec![guide(&["10"], 200)]));
    pump(&mut stores, &keys, 200);
    // Each failed read lands on one pump, then waits `frames` pumps for the next job.
    let fail_then_wait = |stores: &mut Stores, frames: u32, now: i64| {
        let (job, port) = take(stores);
        assert!(matches!(job.req.work, Work::Grid(_)));
        finish_with(&port, job.addr, job.req, |_| None);
        pump(stores, &keys, now);
        for _ in 0..frames - 1 {
            pump(stores, &keys, now);
            assert!(!stores.has_jobs(), "the grid is not asked again inside {frames} frames");
        }
        pump(stores, &keys, now);
    };
    fail_then_wait(&mut stores, RETRY_FRAMES, 210);
    fail_then_wait(&mut stores, 2 * RETRY_FRAMES, 210);
    fail_then_wait(&mut stores, 4 * RETRY_FRAMES, 210);
    for _ in 0..RETRY_DOUBLINGS { let (job, port) = take(&mut stores); port.refused(job.addr);
        for _ in 0..=RETRY_FRAMES << RETRY_DOUBLINGS { pump(&mut stores, &keys, 210); } }
    let (job, port) = take(&mut stores);
    port.refused(job.addr);
    pump(&mut stores, &keys, 210);
    for _ in 0..(RETRY_FRAMES << RETRY_DOUBLINGS) - 1 {
        pump(&mut stores, &keys, 210);
        assert!(!stores.has_jobs(), "the wait is capped, not shortened");
    }
    pump(&mut stores, &keys, 210);
    // The good read goes through the production `read`, whose own answer starts the count over: a
    // hand-built guide here would carry `strikes: 0` whatever `read` did.
    let io = Script::new();
    execute(&mut stores, &io);
    assert_eq!(io.calls.borrow().as_slice(), ["grid:tv.plex.providers.epg.xmltv:7:210"]);
    pump(&mut stores, &keys, 210);
    assert!(!stores.has_jobs(), "a good read is not retried");
    // Past whatever the good read scheduled, the grid is due again; its next failure waits the
    // first step, not the cap it had reached.
    const LATER: i64 = 1_000_000;
    pump(&mut stores, &keys, LATER);
    fail_then_wait(&mut stores, RETRY_FRAMES, LATER);
    assert!(stores.has_jobs());
}

#[test]
fn livetv_fanout_bounds_requests_and_eventually_asks_every_registered_server() {
    let mut stores = Stores::default(); let keys = keys(11);
    let mut seen = Vec::new();
    for _ in 0..4 {
        pump(&mut stores, &keys, 100);
        let jobs = stores.take_jobs_of(StoreId::LiveTv);
        assert!(jobs.len() <= crate::stores::fanout::WIDTH);
        for job in jobs {
            let StoreJob::LiveTv(job, port) = job else { unreachable!() };
            seen.push(job.req.sid);
            finish_with(&port, job.addr, job.req, |_| Some(complete(Vec::new())));
        }
    }
    seen.sort_by_key(|s| s.raw());
    assert_eq!(seen, (0..11).map(sid).collect::<Vec<_>>());
    assert_eq!(stores.livetv_view().status, LiveTvStatus::Empty);
}

#[test]
fn livetv_panicking_worker_answers_failure_and_reset_orphans_the_old_port() {
    let mut stores = Stores::default(); let keys = keys(1);
    pump(&mut stores, &keys, 100);
    let (job, port) = take(&mut stores);
    finish_with(&port, job.addr, job.req, |_| panic!("read panicked"));
    pump(&mut stores, &keys, 100);
    assert_eq!(stores.livetv_view().status, LiveTvStatus::Failed);
    run(&mut stores, LiveTvCmd::Retry, &keys, 100);
    let (old, old_port) = take(&mut stores);
    run(&mut stores, LiveTvCmd::Reset, &keys, 100);
    let (new, new_port) = take(&mut stores);
    assert_ne!(old_port, new_port);
    finish_with(&old_port, old.addr, old.req, |_| Some(complete(vec![guide(&["1"], 200)])));
    pump(&mut stores, &keys, 100);
    assert_eq!(stores.livetv_view().status, LiveTvStatus::Loading);
    finish_with(&new_port, new.addr, new.req, |_| Some(complete(Vec::new())));
    pump(&mut stores, &keys, 100);
    assert_eq!(stores.livetv_view().status, LiveTvStatus::Empty);
}

struct Script {
    calls: std::cell::RefCell<Vec<String>>,
    second_dvr: bool,
    fail_lineup: std::cell::Cell<bool>,
    fail_grid: std::cell::Cell<bool>,
    /// The far end each grid read was asked for (`0`: the Guide's live window).
    untils: std::cell::RefCell<Vec<i64>>,
    empty_grid: bool,
    empty_lineup: bool,
    later: bool,
    far: bool,
    aired: bool,
}
impl Script {
    fn new() -> Self { Self { calls: Default::default(), second_dvr: false, fail_lineup: false.into(), fail_grid: false.into(), untils: Default::default(), empty_grid: false, empty_lineup: false, later: false, far: false, aired: false } }
    /// The grid answers a second airing after the one on now.
    fn with_later(mut self) -> Self { self.later = true; self }
    /// The grid answers a programme that has already ended, inside the Guide's window (it starts at the half hour at or
    /// before `now`).
    fn with_aired(mut self) -> Self { self.aired = true; self }
    /// The grid answers a programme far past the live window when it is asked that far (`until` past the window's end).
    fn with_far(mut self) -> Self { self.later = true; self.far = true; self }
}
impl LiveTvIo for Script {
    fn dvrs(&self) -> (Option<i32>, Option<Vec<plx_plex::plex::Dvr>>) {
        self.calls.borrow_mut().push("dvrs".into());
        (Some(200), Some((7..=if self.second_dvr { 8 } else { 7 }).map(|id| plx_plex::plex::Dvr {
            key: id.to_string(), epg_identifier: format!("tv.plex.providers.epg.xmltv:{id}"), ..Default::default()
        }).collect()))
    }
    fn channels(&self, provider: &str) -> (Option<i32>, Option<Vec<plx_plex::plex::EpgChannel>>) {
        self.calls.borrow_mut().push(format!("channels:{provider}"));
        if self.fail_lineup.get() && provider.ends_with('8') { return (Some(404), None); }
        if self.empty_lineup { return (Some(200), Some(Vec::new())); }
        (Some(200), Some(vec![plx_plex::plex::EpgChannel { grid_key: "C2".into(), vcn: "2".into(), title: "2 Channel".into(), thumb: "/logo".into() }]))
    }
    fn guide(&self, provider: &str, now: i64, until: i64) -> Option<Vec<plx_plex::plex::Metadata>> {
        self.calls.borrow_mut().push(format!("grid:{provider}:{now}"));
        self.untils.borrow_mut().push(until);
        if self.fail_grid.get() { None }
        else if self.empty_grid { Some(Vec::new()) }
        else {
            // with an aired programme before it, it begins after that one ended (otherwise the guide gives no start)
            let begins = if self.aired { livetv::window_start(now) + 30 } else { 0 };
            let mut rows = vec![serde_json::from_value(serde_json::json!({"title":"Programme","thumb":"/programme",
                "Media":[{"channelVcn":"2","channelIdentifier":"C2","beginsAt":begins,"endsAt":now + 50}]})).unwrap()];
            if self.aired {
                rows.push(serde_json::from_value(serde_json::json!({"title":"Aired",
                    "Media":[{"channelVcn":"2","channelIdentifier":"C2","beginsAt":livetv::window_start(now) - 900,"endsAt":livetv::window_start(now) + 20}]})).unwrap());
            }
            if self.later {
                rows.push(serde_json::from_value(serde_json::json!({"title":"Next",
                    "Media":[{"channelVcn":"2","channelIdentifier":"C2","beginsAt":now + 50,"endsAt":now + 800}]})).unwrap());
            }
            if self.far && until > livetv::window_end(now) {
                rows.push(serde_json::from_value(serde_json::json!({"title":"Far",
                    "Media":[{"channelVcn":"2","channelIdentifier":"C2","beginsAt":livetv::window_end(now) + 600,"endsAt":livetv::window_end(now) + 1_200}]})).unwrap());
            }
            Some(rows)
        }
    }
}
fn execute(stores: &mut Stores, io: &Script) {
    let (job, port) = take(stores);
    finish_with(&port, job.addr, job.req, |req| read(io, req));
}

#[test]
fn livetv_missing_initial_grid_preserves_lineup_and_retries_only_the_grid() {
    let mut stores = Stores::default(); let keys = keys(1); let io = Script::new();
    io.fail_grid.set(true);
    pump(&mut stores, &keys, 100);
    execute(&mut stores, &io);
    pump(&mut stores, &keys, 100);
    assert_eq!(stores.livetv_view().status, LiveTvStatus::Ready);
    assert_eq!(stores.livetv_view().channels[0].airings.on_air(), None);
    assert!(stores.livetv_view().channels[0].airings.all().is_empty());
    io.fail_grid.set(false); io.calls.borrow_mut().clear();
    for _ in 0..RETRY_FRAMES { pump(&mut stores, &keys, 101); }
    execute(&mut stores, &io);
    pump(&mut stores, &keys, 101);
    assert_eq!(io.calls.borrow().as_slice(), ["grid:tv.plex.providers.epg.xmltv:7:101"]);
    assert_eq!(stores.livetv_view().channels[0].airings.now_title(), "Programme");
    assert_eq!(stores.livetv_view().channels[0].airings.now_thumb(), "/programme");
}

#[test]
fn livetv_empty_successful_grid_is_sampled_again_when_programming_can_begin() {
    let mut stores = Stores::default(); let keys = keys(1); let mut io = Script::new();
    io.empty_grid = true;
    pump(&mut stores, &keys, 100); execute(&mut stores, &io); pump(&mut stores, &keys, 100);
    pump(&mut stores, &keys, 159); assert!(!stores.has_jobs());
    pump(&mut stores, &keys, 160);
    let (job, port) = take(&mut stores);
    assert!(matches!(job.req.work, Work::Grid(_)));
    port.refused(job.addr);
}

#[test]
fn livetv_one_failed_dvr_lineup_does_not_drop_the_dvr_that_answered() {
    let mut stores = Stores::default(); let keys = keys(1); let mut io = Script::new();
    io.second_dvr = true; io.fail_lineup.set(true);
    pump(&mut stores, &keys, 100); execute(&mut stores, &io); pump(&mut stores, &keys, 100);
    assert_eq!(stores.livetv_view().status, LiveTvStatus::Ready);
    assert_eq!(stores.livetv_view().channels.len(), 1);
    assert_eq!(stores.livetv_view().channels[0].dvr, "7");
    io.fail_lineup.set(false);
    for _ in 0..RETRY_FRAMES { pump(&mut stores, &keys, 101); }
    execute(&mut stores, &io); pump(&mut stores, &keys, 101);
    assert_eq!(stores.livetv_view().channels.len(), 2);
}

#[test]
fn livetv_refreshes_the_earliest_expired_dvr_and_reset_removal_notifies() {
    let mut stores = Stores::default(); let keys = keys(1);
    pump(&mut stores, &keys, 100);
    let first = guide(&["10"], 250);
    let mut earlier = guide(&["2"], 200); earlier.dvr = "8".into();
    earlier.channels[0].dvr = "8".into(); earlier.provider = "tv.plex.providers.epg.xmltv:8".into();
    answer(&mut stores, Some(vec![first, earlier])); pump(&mut stores, &keys, 100);
    pump(&mut stores, &keys, 300);
    let (job, port) = take(&mut stores);
    assert!(matches!(&job.req.work, Work::Grid(g) if g.dvr == "8"));
    port.refused(job.addr);
    stores.take_notices();
    let revision = stores.livetv_view().revision();
    assert!(run(&mut stores, LiveTvCmd::Reset, &keys, 300));
    assert_eq!(stores.livetv_view().status, LiveTvStatus::Loading);
    assert!(stores.livetv_view().channels.is_empty());
    assert!(stores.livetv_view().revision() > revision);
    assert!(stores.take_notices().iter().any(|(id, _)| *id == StoreId::LiveTv));
}

#[test]
fn livetv_reset_with_no_keys_clears_the_publication_and_notifies() {
    let mut stores = Stores::default(); let keys = keys(1);
    pump(&mut stores, &keys, 100); answer(&mut stores, Some(vec![guide(&["2"], 200)]));
    pump(&mut stores, &keys, 100); stores.take_notices();
    let revision = stores.livetv_view().revision();
    assert!(run(&mut stores, LiveTvCmd::Reset, &ClientKeys::default(), 101));
    assert_eq!(stores.livetv_view().status, LiveTvStatus::Loading);
    assert!(stores.livetv_view().channels.is_empty());
    assert!(stores.livetv_view().revision() > revision);
    assert!(stores.take_notices().iter().any(|(id, _)| *id == StoreId::LiveTv));
}

#[test]
fn livetv_a_failed_grid_does_not_starve_another_expired_dvr() {
    let mut stores = Stores::default(); let keys = keys(1);
    pump(&mut stores, &keys, 100);
    let first = guide(&["1"], 200);
    let mut second = guide(&["2"], 250); second.dvr = "8".into();
    second.channels[0].dvr = "8".into();
    answer(&mut stores, Some(vec![first, second])); pump(&mut stores, &keys, 100);
    pump(&mut stores, &keys, 300);
    let (job, port) = take(&mut stores);
    assert!(matches!(&job.req.work, Work::Grid(g) if g.dvr == "7"));
    finish_with(&port, job.addr, job.req, |_| None);
    pump(&mut stores, &keys, 300);
    let (job, port) = take(&mut stores);
    assert!(matches!(&job.req.work, Work::Grid(g) if g.dvr == "8"));
    port.refused(job.addr);
}

#[test]
fn livetv_a_successful_empty_lineup_retires_channels_during_partial_discovery() {
    let mut stores = Stores::default(); let keys = keys(1); let mut io = Script::new();
    io.second_dvr = true; io.fail_lineup.set(true);
    pump(&mut stores, &keys, 100); execute(&mut stores, &io); pump(&mut stores, &keys, 100);
    assert_eq!(stores.livetv_view().channels.len(), 1);
    io.empty_lineup = true;
    for _ in 0..RETRY_FRAMES { pump(&mut stores, &keys, 101); }
    execute(&mut stores, &io); pump(&mut stores, &keys, 101);
    assert!(stores.livetv_view().channels.is_empty());
}

#[test]
fn livetv_partial_discovery_does_not_block_an_expired_known_guide() {
    let mut stores = Stores::default(); let keys = keys(1); let mut io = Script::new();
    io.second_dvr = true; io.fail_lineup.set(true);
    pump(&mut stores, &keys, 100); execute(&mut stores, &io); pump(&mut stores, &keys, 100);
    pump(&mut stores, &keys, 151);
    let (job, port) = take(&mut stores);
    assert!(matches!(&job.req.work, Work::Grid(g) if g.dvr == "7"));
    port.refused(job.addr);
}

#[test]
fn livetv_play_title_preserves_the_unnumbered_name_and_stale_guides_do_not_spin() {
    for number in ["", "ABC", "10"] {
        let mut channel = Channel::of(sid(0), &guide(&[number], 200).channels[0]);
        channel.name = "Family Channel".into();
        assert_eq!(channel_name_of(&channel_tile(&channel)), Some("Family Channel"));
    }
    assert_eq!(Guide::next_refresh(&guide(&["1"], 200).channels, 201), 261);
}

/// The Guide's window slides every half hour, so the guide is read again when it does even if the airing on now
/// runs far past it: otherwise a programme entering the window's far end waits for some airing to end.
#[test]
fn the_guide_is_read_again_when_the_window_moves_if_that_comes_before_an_airing_ends() {
    let ends_much_later = 1_792_522_800 + 6 * 3_600;
    let channels = guide(&["1"], ends_much_later).channels;
    let now = 1_792_522_800 + 600;
    assert_eq!(Guide::next_refresh(&channels, now), 1_792_522_800 + 1_800, "the next half hour, not the airing's end");
    assert_eq!(Guide::next_refresh(&guide(&["1"], now + 60).channels, now), now + 60, "an airing that ends first is read at its end");
    assert_eq!(Guide::next_refresh(&[], 1_792_522_800 + 1_790), 1_792_524_600, "the poll never runs past the window's move");
}

#[test]
fn livetv_the_grid_read_publishes_the_programmes_after_the_one_on_now() {
    let mut stores = Stores::default(); let keys = keys(1); let io = Script::new().with_later();
    pump(&mut stores, &keys, 100); execute(&mut stores, &io); pump(&mut stores, &keys, 100);
    let channel = &stores.livetv_view().channels[0];
    assert_eq!(channel.airings.now_title(), "Programme");
    assert_eq!(channel.airings.all().iter().map(|a| (a.title.as_str(), a.begins_at, a.ends_at)).collect::<Vec<_>>(),
        [("Programme", 0, 150), ("Next", 150, 900)], "one list, the one on now first");
}

/// **A programme that has already aired inside the Guide's window is published with the rest**, ahead of the one on
/// now, which is still what the channel reads as on; and the grid read it came from asked for the half hour the
/// window starts at, not for now (the Script records the instant it was read at, and the path takes the window's
/// start from it: `livetv::guide_path`).
#[test]
fn livetv_the_grid_read_publishes_what_has_aired_in_the_window_and_the_one_on_now_is_unchanged() {
    let mut stores = Stores::default(); let keys = keys(1); let io = Script::new().with_aired().with_later();
    pump(&mut stores, &keys, 1_000); execute(&mut stores, &io); pump(&mut stores, &keys, 1_000);
    let channel = &stores.livetv_view().channels[0];
    assert_eq!(channel.airings.all().iter().map(|a| a.title.as_str()).collect::<Vec<_>>(), ["Aired", "Programme", "Next"]);
    assert_eq!((channel.airings.now_title(), channel.airings.on_air_at()), ("Programme", Some(1)));
    assert_eq!((channel.airings.begins_at(), channel.airings.ends_at()), (30, 1_050));
    assert_eq!(stores.livetv_view().tiles[0].show_title, "Programme", "What's on reads the same on now");
}

/// Memory stays bounded across scheduled reads: when the window rolls, the aired programmes of the new window replace
/// the old ones, which are not carried over (only the far programmes of a far read are).
#[test]
fn a_scheduled_read_after_the_window_rolls_holds_only_what_aired_in_the_new_window() {
    let mut stores = Stores::default(); let keys = keys(1); let io = Script::new().with_aired();
    pump(&mut stores, &keys, 1_000); execute(&mut stores, &io); pump(&mut stores, &keys, 1_000);
    assert_eq!(stores.livetv_view().channels[0].airings.all()[0].begins_at, livetv::window_start(1_000) - 900);
    // the window rolled to the half hour 1_800, and the airing on ended: the scheduled read is made at that window
    pump(&mut stores, &keys, 1_900);
    let (job, port) = take(&mut stores);
    finish_with(&port, job.addr, job.req, |req| read(&io, req));
    pump(&mut stores, &keys, 1_900);
    assert_eq!(all_titles(&stores), ["Aired", "Programme"], "one aired programme, the new window's");
    assert_eq!(stores.livetv_view().channels[0].airings.all()[0].begins_at, livetv::window_start(1_900) - 900);
}

#[test]
fn an_armed_landing_gate_asks_no_live_tv_lineup() {
    // A recording or replay owns the run and its tape carries no Live TV read.
    let mut stores = Stores::default();
    stores.landgate.arm_recording();
    let keys = keys(2);
    assert!(!pump(&mut stores, &keys, 100));
    assert!(stores.take_jobs_of(StoreId::LiveTv).is_empty());
    assert!(stores.livetv_view().channels.is_empty());
    stores.landgate.disarm();
    assert!(!pump(&mut stores, &keys, 100));
    assert_eq!(stores.take_jobs_of(StoreId::LiveTv).len(), 2);
}

/// The Guide scrolled ahead asks the store for the far end of the window it shows ([`LiveTvCmd::Reach`]); the store
/// reads the guide again to it when it grows, by the one re-read rule (a due guide is read), and never for a value
/// it has already read to.
#[test]
fn a_reach_past_what_the_guide_was_read_to_reads_it_again_to_that_end_and_only_when_it_grows() {
    let mut stores = Stores::default(); let keys = keys(1); let io = Script::new().with_later();
    pump(&mut stores, &keys, 100); execute(&mut stores, &io); pump(&mut stores, &keys, 100);
    assert_eq!(io.untils.borrow().as_slice(), [0], "the first read is the live window's");
    run(&mut stores, LiveTvCmd::Reach(7_200), &keys, 100);
    let (job, port) = take(&mut stores);
    assert!(matches!(&job.req.work, Work::Grid(_)), "the grid alone, not the lineup");
    assert_eq!(job.req.until, 7_200);
    finish_with(&port, job.addr, job.req, |req| read(&io, req));
    pump(&mut stores, &keys, 100);
    assert_eq!(io.untils.borrow().as_slice(), [0, 7_200]);
    for reach in [7_200, 5_400, 0] {
        run(&mut stores, LiveTvCmd::Reach(reach), &keys, 100);
        assert!(stores.take_jobs_of(StoreId::LiveTv).is_empty(), "a reach it has read to or past reads nothing: {reach}");
    }
    run(&mut stores, LiveTvCmd::Reach(9_000), &keys, 100);
    assert_eq!(stores.take_jobs_of(StoreId::LiveTv).len(), 1, "a reach that grows reads again");
}

/// A shift made while a read is in flight is not lost when the read lands short: the guide is still behind the reach.
#[test]
fn a_read_that_lands_short_of_the_reach_leaves_the_guide_due() {
    let mut stores = Stores::default(); let keys = keys(1); let io = Script::new().with_later();
    pump(&mut stores, &keys, 100); execute(&mut stores, &io); pump(&mut stores, &keys, 100);
    run(&mut stores, LiveTvCmd::Reach(9_000), &keys, 100);
    let (first, port) = take(&mut stores);
    run(&mut stores, LiveTvCmd::Reach(10_800), &keys, 100);
    assert!(stores.take_jobs_of(StoreId::LiveTv).is_empty(), "one read at a time");
    finish_with(&port, first.addr, first.req, |req| read(&io, req));
    pump(&mut stores, &keys, 100);
    let (second, _) = take(&mut stores);
    assert_eq!(second.req.until, 10_800, "the landed read was short of the reach, so the guide is read again");
}

/// Release: with the reach back at 0 the next scheduled read is the live window's again.
#[test]
fn releasing_the_reach_returns_the_scheduled_reads_to_the_live_window() {
    let mut stores = Stores::default(); let keys = keys(1); let io = Script::new().with_later();
    pump(&mut stores, &keys, 100); execute(&mut stores, &io); pump(&mut stores, &keys, 100);
    run(&mut stores, LiveTvCmd::Reach(7_200), &keys, 100);
    execute(&mut stores, &io); pump(&mut stores, &keys, 100);
    run(&mut stores, LiveTvCmd::Reach(0), &keys, 100);
    pump(&mut stores, &keys, 400);
    let (job, _) = take(&mut stores);
    assert_eq!(job.req.until, 0, "the airing ended: the scheduled read asks for no more than the live window");
}

/// A reach set before the lineup is read is carried by that read, which then holds the guide to it.
#[test]
fn a_reach_set_before_the_first_read_is_carried_by_it() {
    let mut stores = Stores::default(); let keys = keys(1); let io = Script::new().with_later();
    run(&mut stores, LiveTvCmd::Reach(9_000), &keys, 100);
    execute(&mut stores, &io); pump(&mut stores, &keys, 100);
    assert_eq!(io.untils.borrow().as_slice(), [9_000]);
    pump(&mut stores, &keys, 100);
    assert!(stores.take_jobs_of(StoreId::LiveTv).is_empty(), "read to the reach already");
}

/// The titles of every programme the store publishes for the first channel, in start order.
fn all_titles(stores: &Stores) -> Vec<String> {
    stores.livetv_view().channels[0].airings.all().iter().map(|airing| airing.title.clone()).collect()
}

/// **The far span is read once, not at every scheduled refresh** (Browse's Later today holds the reach at the end of the day):
/// the scheduled read of what is on now asks the live window alone, and the far programmes the far read answered stay
/// published meanwhile, so the set a reach shows does not shrink to the window and grow back.
#[test]
fn a_scheduled_read_under_a_held_reach_asks_the_live_window_and_keeps_the_far_programmes() {
    let mut stores = Stores::default(); let keys = keys(1); let io = Script::new().with_far();
    pump(&mut stores, &keys, 100); execute(&mut stores, &io); pump(&mut stores, &keys, 100);
    run(&mut stores, LiveTvCmd::Reach(20_000), &keys, 100);
    execute(&mut stores, &io); pump(&mut stores, &keys, 100);
    assert_eq!(all_titles(&stores), ["Programme", "Next", "Far"], "the far read answered the far programme");
    // the airing ended: the scheduled read is due, with the reach still held and the far read current
    pump(&mut stores, &keys, 400);
    let (job, port) = take(&mut stores);
    assert_eq!(job.req.until, 0, "the scheduled refresh asks the live window alone");
    finish_with(&port, job.addr, job.req, |req| read(&io, req));
    pump(&mut stores, &keys, 400);
    assert_eq!(all_titles(&stores), ["Programme", "Next", "Far"], "the far programme stays while its read is current");
    assert_eq!(io.untils.borrow().as_slice(), [0, 20_000, 0]);
}

/// The far span is read again by the store's own schedule once its read is a window step old, with the reach still held.
#[test]
fn the_far_span_is_read_again_a_window_step_after_it_was_read() {
    let mut stores = Stores::default(); let keys = keys(1); let io = Script::new().with_far();
    pump(&mut stores, &keys, 100); execute(&mut stores, &io); pump(&mut stores, &keys, 100);
    run(&mut stores, LiveTvCmd::Reach(20_000), &keys, 100);
    execute(&mut stores, &io); pump(&mut stores, &keys, 100);
    let step = livetv::WINDOW_STEP_SECONDS;
    pump(&mut stores, &keys, 100 + step - 1);
    let (job, port) = take(&mut stores);
    assert_eq!(job.req.until, 0, "a second short of the step: the scheduled read is the live window's");
    finish_with(&port, job.addr, job.req, |req| read(&io, req));
    pump(&mut stores, &keys, 100 + step - 1);
    assert!(stores.take_jobs_of(StoreId::LiveTv).is_empty());
    pump(&mut stores, &keys, 100 + step);
    let (job, _) = take(&mut stores);
    assert_eq!(job.req.until, 20_000, "the step has passed: the far span is read again");
}

/// With the reach released, the far programmes do not outlive their read: the next scheduled read after it is a step old
/// answers the live window and drops them.
#[test]
fn far_programmes_do_not_outlive_their_read_once_nothing_reaches_for_them() {
    let mut stores = Stores::default(); let keys = keys(1); let io = Script::new().with_far();
    pump(&mut stores, &keys, 100); execute(&mut stores, &io); pump(&mut stores, &keys, 100);
    run(&mut stores, LiveTvCmd::Reach(20_000), &keys, 100);
    execute(&mut stores, &io); pump(&mut stores, &keys, 100);
    run(&mut stores, LiveTvCmd::Reach(0), &keys, 100);
    let later = 100 + livetv::WINDOW_STEP_SECONDS + 400;
    pump(&mut stores, &keys, later);
    let (job, port) = take(&mut stores);
    assert_eq!(job.req.until, 0);
    finish_with(&port, job.addr, job.req, |req| read(&io, req));
    pump(&mut stores, &keys, later);
    assert_eq!(all_titles(&stores), ["Programme", "Next"], "the stale far programme is gone");
    assert!(stores.take_jobs_of(StoreId::LiveTv).is_empty());
}

/// A reach that ends inside the live window has no far span to hold: it does not make the far read due again and again.
#[test]
fn a_reach_inside_the_live_window_does_not_loop() {
    let mut stores = Stores::default(); let keys = keys(1); let io = Script::new().with_far();
    pump(&mut stores, &keys, 100); execute(&mut stores, &io); pump(&mut stores, &keys, 100);
    run(&mut stores, LiveTvCmd::Reach(5_000), &keys, 100);
    execute(&mut stores, &io); pump(&mut stores, &keys, 100);
    pump(&mut stores, &keys, 101);
    assert!(stores.take_jobs_of(StoreId::LiveTv).is_empty(), "read to the reach already, and no step of its own to wait out");
    // past the first scheduled refresh: the airing ended, one read answers it and the guide is not owed another
    pump(&mut stores, &keys, 151);
    let (job, port) = take(&mut stores);
    assert_eq!(job.req.until, 5_000, "the reach ends inside the live window: the read carries it, it is not dropped for 0");
    finish_with(&port, job.addr, job.req, |req| read(&io, req));
    pump(&mut stores, &keys, 151);
    assert!(stores.take_jobs_of(StoreId::LiveTv).is_empty(), "one grid read per refresh, not two");
}

/// A grid read that fails when the lineup is first read holds no far programmes: with the reach held, the far span is read
/// again by the retry, not left for a window step because the failed read recorded the reach as read.
#[test]
fn a_failed_first_grid_read_under_a_reach_leaves_the_far_read_due() {
    let mut stores = Stores::default(); let keys = keys(1); let io = Script::new().with_far();
    io.fail_grid.set(true);
    run(&mut stores, LiveTvCmd::Reach(20_000), &keys, 100);
    execute(&mut stores, &io); pump(&mut stores, &keys, 100);
    io.fail_grid.set(false);
    for _ in 0..RETRY_FRAMES { pump(&mut stores, &keys, 101); }
    let (job, port) = take(&mut stores);
    assert!(matches!(&job.req.work, Work::Grid(_)), "the grid alone, the lineup is held");
    assert_eq!(job.req.until, 20_000, "the far span is owed: the retry reads to the reach, not the live window");
    finish_with(&port, job.addr, job.req, |req| read(&io, req));
    pump(&mut stores, &keys, 101);
    assert_eq!(all_titles(&stores), ["Programme", "Next", "Far"]);
}
