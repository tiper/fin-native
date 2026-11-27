//! Live TV's server-fanned read model. Steps receive keys and time; only the job body reads PMS.
use crate::pms::PmsMovie;
use crate::stores::fanout::{Fanout, Standing};
use plx_machine::job::{Job, Port};
use plx_machine::machine::{Addr, MachineId, RequestId};
use plx_plex::plex::livetv::{self, LiveChannel};
pub use plx_plex::plex::livetv::LiveAiring;
use plx_plex::plex::{Client, ClientKey, ClientKeys, ServerId};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;

mod recent;
pub use recent::{entry_of, resolve, RecentKey, RecentSeat, MAX_RECENT};
use recent::Recent;

const RETRY_FRAMES: u32 = 120;
/// A server that keeps refusing its lineup (a shared server, one with no Live TV grant) is asked
/// again at a doubling interval: 2 s, then up to 2^5 of that (about a minute at 60 Hz).
const RETRY_DOUBLINGS: u32 = 5;
/// A guide with no current airing is sampled again so a later programme can appear.
const GUIDE_POLL_SECONDS: i64 = 60;
pub const KIND_LIVE_CHANNEL: i32 = 5;
pub const LIVE_TV_HUB: &str = "home.livetv";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiveTvStatus { Loading, Ready, Empty, Failed }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Channel {
    pub sid: ServerId,
    pub dvr: String,
    pub channel: String,
    pub vcn: String,
    pub name: String,
    pub thumb: String,
    pub now: String,
    pub now_thumb: String,
    pub now_summary: String,
    pub begins_at: i64,
    pub ends_at: i64,
    /// The programmes after the current one that begin by the end the guide was last read to (the Guide's
    /// window, or further while it is scrolled ahead: [`LiveTvCmd::Reach`]), earliest first (empty: the guide
    /// has none, or no data at all).
    pub later: Vec<LiveAiring>,
}

impl Channel {
    fn of(sid: ServerId, ch: &LiveChannel) -> Self {
        Self { sid, dvr: ch.dvr.clone(), channel: ch.channel.clone(), vcn: ch.vcn.clone(),
            name: ch.name.clone(), thumb: ch.thumb.clone(), now: ch.now.clone(),
            now_thumb: ch.now_thumb.clone(), now_summary: ch.now_summary.clone(), begins_at: ch.begins_at, ends_at: ch.ends_at,
            later: ch.later.clone() }
    }
}

pub fn channel_tile(ch: &Channel) -> PmsMovie {
    PmsMovie { sid: ch.sid, kind: KIND_LIVE_CHANNEL, rk: format!("livetv:{}:{}", ch.dvr, ch.channel),
        title: format!("{} {}", ch.vcn, ch.name).trim().to_owned(), show_title: ch.now.clone(),
        // The unnumbered identity is retained for Play, including a channel with no vcn.
        summary: ch.name.clone(), thumb: ch.thumb.clone(), still: ch.thumb.clone(), ..Default::default() }
}

pub fn live_channel_of(m: &PmsMovie) -> Option<(&str, &str)> {
    if m.kind != KIND_LIVE_CHANNEL { return None; }
    let (dvr, channel) = m.rk.strip_prefix("livetv:")?.split_once(':')?;
    (livetv::is_path_id(dvr) && livetv::is_path_id(channel)).then_some((dvr, channel))
}

/// A tile's channel name, without its displayed virtual number.
pub fn channel_name_of(m: &PmsMovie) -> Option<&str> {
    live_channel_of(m)?;
    Some(&m.summary)
}

#[derive(Clone, Copy)]
pub struct LiveTvView<'a> {
    pub status: LiveTvStatus,
    pub channels: &'a [Channel],
    pub tiles: &'a [Arc<PmsMovie>],
    /// The seated profile's Recent channels, newest first, at most [`MAX_RECENT`]: lineup channels,
    /// so a remembered channel the lineup does not list is not here. Empty is a row to hide.
    pub recent: &'a [Channel],
    revision: u32,
}
impl<'a> LiveTvView<'a> {
    pub fn revision(self) -> u32 { self.revision }
    #[cfg(any(test, feature = "test-support"))]
    pub fn for_test(status: LiveTvStatus, channels: &'a [Channel], tiles: &'a [Arc<PmsMovie>], revision: u32) -> Self {
        Self { status, channels, tiles, recent: &[], revision }
    }
    /// [`for_test`](Self::for_test) with a Recent row.
    #[cfg(any(test, feature = "test-support"))]
    pub fn for_test_with_recent(mut self, recent: &'a [Channel]) -> Self {
        self.recent = recent;
        self
    }
    pub const LOADING: Self = Self { status: LiveTvStatus::Loading, channels: &[], tiles: &[], recent: &[], revision: 0 };
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LiveTvCmd {
    Reset,
    Retry,
    /// **How far ahead the Guide shows**: the Unix second the guide must be read to (the far end of the window the
    /// Guide is on, a step more so the next shift has its programmes), or `0` when nothing shows beyond the live
    /// window. It is the screen's to set and to release; the store holds it, reads a guide that was read to less
    /// again by the one re-read rule ([`Guide::due`]) and asks every read it makes for at least this much. Setting
    /// what it already holds, or less than a guide has been read to, reads nothing.
    Reach(i64),
}

/// **What the app captured for one step of the store**: the clients, the time and the seat. The step
/// reads no registry, no clock and no preference of its own; the same input to the same state is the
/// same step.
#[derive(Clone, Debug)]
pub struct LiveTvInput<'a> {
    pub keys: &'a ClientKeys,
    /// The frame's wall-clock Unix second, or `None` when the frame has none (a replay, a zone the
    /// platform cannot read). With no time nothing is scheduled: the guide is neither asked nor
    /// refreshed (a due time cannot be judged), while the row and the lineup already held still publish.
    pub now: Option<i64>,
    /// Who is seated and what the memory door holds for them ([`recent`]).
    pub recent: RecentSeat,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Guide { dvr: String, provider: String, channels: Vec<LiveChannel>, refresh_at: i64, retry_cd: u32,
    /// Consecutive failed grid reads, which set the next wait ([`Guide::backoff`]).
    strikes: u32,
    /// The far end the last read asked for ([`LiveTvReq::until`]): the guide is due again while the reach is past it.
    until: i64 }
impl Guide {
    /// A grid that keeps failing is asked again at the lineup's doubling interval
    /// ([`RETRY_DOUBLINGS`]), not every [`RETRY_FRAMES`] for as long as the server is down.
    fn backoff(&mut self) {
        self.retry_cd = RETRY_FRAMES << self.strikes.min(RETRY_DOUBLINGS);
        self.strikes = self.strikes.saturating_add(1);
    }
    /// Due when its time has come, or when the Guide reaches further than it was read to (`reach`).
    fn due(&self, now: i64, reach: i64) -> bool { self.retry_cd == 0 && (now >= self.refresh_at || self.until < reach) }
    /// When the guide is next read: when the soonest airing on now ends, or when the Guide's window next
    /// moves (every half hour, [`livetv::window_start`]) if that comes first, so a programme that enters the
    /// window's far end is asked for the half hour it does and not only when some airing happens to end.
    fn next_refresh(channels: &[LiveChannel], now: i64) -> i64 {
        let roll = livetv::window_start(now).saturating_add(livetv::WINDOW_STEP_SECONDS);
        channels.iter().filter_map(|c| (c.ends_at > now).then_some(c.ends_at)).min()
            .unwrap_or_else(|| now.saturating_add(GUIDE_POLL_SECONDS)).min(roll)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Work { Discover, Grid(Guide) }

#[derive(Debug, PartialEq, Eq)]
pub struct LiveTvReq {
    sid: ServerId, key: ClientKey, now: i64, work: Work,
    /// The far end the grid is read to: the store's reach when the read was made (`0` for the live window, which the
    /// read widens to the window's own end).
    until: i64,
}
impl LiveTvReq {
    pub fn server(&self) -> (ServerId, ClientKey) { (self.sid, self.key) }
}

struct ReadResult { guides: Vec<Guide>, roster: Option<Vec<String>>, complete: bool, retry: bool }
pub struct LiveTvAnswer { result: Option<ReadResult>, grid: bool }
struct Mail { request: u32, answer: LiveTvAnswer }
#[derive(Default)]
pub struct LiveTvAdapter { fetch: crate::stores::Fetch<Mail> }

#[derive(Clone)]
pub struct LiveTvPort(Arc<LiveTvAdapter>);
impl PartialEq for LiveTvPort {
    fn eq(&self, other: &Self) -> bool { Arc::ptr_eq(&self.0, &other.0) }
}
impl std::fmt::Debug for LiveTvPort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "LiveTvPort({:p})", Arc::as_ptr(&self.0))
    }
}
impl Port<LiveTvAnswer> for LiveTvPort {
    fn done(&self, addr: Addr, answer: LiveTvAnswer) {
        self.0.fetch.post(Mail { request: addr.req.0, answer }, |old| addr.req.0 >= old.request);
    }
    fn refused(&self, addr: Addr) { self.0.fetch.refuse_for(addr.req.0); }
}

struct Source {
    sid: ServerId,
    key: Option<ClientKey>,
    adapter: Arc<LiveTvAdapter>,
    request: u32,
    guides: Vec<Guide>,
    reading_grid: Option<String>,
    answered: bool,
    complete: bool,
    failed: bool,
    retry_cd: u32,
    /// Consecutive failed lineup reads, which set the next wait ([`Source::backoff`]).
    strikes: u32,
}
impl Source {
    fn new(sid: ServerId, key: Option<ClientKey>) -> Self {
        Self { sid, key, adapter: Arc::new(Default::default()), request: 0, guides: Vec::new(),
            reading_grid: None, answered: false, complete: false, failed: false, retry_cd: 0, strikes: 0 }
    }
    fn work(&self, now: i64, reach: i64) -> Option<Work> {
        self.guides.iter().filter(|g| g.due(now, reach)).min_by_key(|g| g.refresh_at).cloned().map(Work::Grid)
            .or_else(|| (!self.complete && self.retry_cd == 0).then_some(Work::Discover))
    }
    fn failed(&mut self) {
        self.failed = true;
        if let Some(dvr) = self.reading_grid.take() {
            if let Some(guide) = self.guides.iter_mut().find(|g| g.dvr == dvr) { guide.backoff(); }
        } else { self.backoff(); }
    }
    fn backoff(&mut self) {
        self.retry_cd = RETRY_FRAMES << self.strikes.min(RETRY_DOUBLINGS);
        self.strikes = self.strikes.saturating_add(1);
    }
    fn standing(&self, now: i64, reach: i64) -> Standing {
        if self.adapter.fetch.busy() { Standing::Running }
        else if (!self.complete && self.retry_cd == 0) || self.guides.iter().any(|g| g.due(now, reach)) { Standing::Wanting }
        else { Standing::Idle }
    }
}

pub struct LiveTvState {
    sources: Vec<Source>,
    channels: Vec<Channel>,
    tiles: Vec<Arc<PmsMovie>>,
    status: LiveTvStatus,
    revision: u32,
    fanout: Fanout,
    /// The Recent mode the last seat input put the store in, and the row it resolves to (see [`recent`]).
    recent: Recent,
    recent_row: Vec<Channel>,
    /// How far ahead the Guide shows ([`LiveTvCmd::Reach`]); `0` when it shows no more than the live window.
    reach: i64,
    pub(crate) jobs: Vec<crate::stores::StoreJob>,
}
impl Default for LiveTvState {
    fn default() -> Self {
        Self { sources: Vec::new(), channels: Vec::new(), tiles: Vec::new(), status: LiveTvStatus::Loading,
            revision: 0, fanout: Default::default(), recent: Default::default(), recent_row: Vec::new(),
            reach: 0, jobs: Vec::new() }
    }
}
impl LiveTvState {
    pub fn view(&self) -> LiveTvView<'_> {
        LiveTvView { status: self.status, channels: &self.channels, tiles: &self.tiles, recent: &self.recent_row,
            revision: self.revision }
    }
    pub fn run(&mut self, cmd: LiveTvCmd, gate: &plx_machine::landgate::Gate, input: &LiveTvInput<'_>) -> bool {
        let reset = matches!(cmd, LiveTvCmd::Reset);
        match cmd {
            LiveTvCmd::Reset => {
                self.sources.clear();
                self.jobs.clear();
                self.fanout = Fanout::default();
            }
            LiveTvCmd::Retry => {
                for s in &mut self.sources {
                    s.retry_cd = 0;
                    for guide in &mut s.guides { guide.retry_cd = 0; }
                }
            }
            // held, not acted on here: a guide read to less is due, and the pump below asks for it
            LiveTvCmd::Reach(until) => self.reach = until.max(0),
        }
        let changed = self.pump(gate, input);
        changed || (reset && self.publish(input.keys, &input.recent))
    }
    /// A live channel was tuned with nobody seated: noted for the session. A seated tune is written to
    /// the memory door by the app and arrives in the next input, so it is not the store's to note (it
    /// answers false). The seat input is applied first, so the note is judged under the seat it is made in.
    pub fn note_unseated(&mut self, key: RecentKey, input: &LiveTvInput<'_>) -> bool {
        self.recent.drive(&input.recent);
        let _ = self.recent.note_unseated(key);
        self.publish_recent(input.keys, &input.recent)
    }
    pub fn pump(&mut self, gate: &plx_machine::landgate::Gate, input: &LiveTvInput<'_>) -> bool {
        let keys = input.keys;
        // The seat and the stored list are not the guide's: they are followed (and the row re-resolved
        // against the roster and the lineup) whatever owns the run.
        self.recent.drive(&input.recent);
        let recent = self.publish_recent(keys, &input.recent);
        // A recording, a replay or a dump owns the run, and the tape carries no Live TV read
        // (Notify stands down the same way): no tuner lineup is asked while the gate is armed.
        if gate.is_armed() { return recent; }
        let slots: Vec<_> = keys.slots().filter(|s| *s != ServerId::UNSET).collect();
        let before = self.sources.len();
        self.sources.retain(|s| slots.contains(&s.sid));
        let mut dirty = before != self.sources.len();
        for sid in slots {
            let key = keys.key(sid);
            match self.sources.iter_mut().find(|s| s.sid == sid) {
                Some(s) if s.key != key => { *s = Source::new(sid, key); dirty = true; }
                Some(_) => {}
                None => { self.sources.push(Source::new(sid, key)); dirty = true; }
            }
        }
        for s in &mut self.sources {
            s.retry_cd = s.retry_cd.saturating_sub(1);
            for guide in &mut s.guides { guide.retry_cd = guide.retry_cd.saturating_sub(1); }
            let adapter = Arc::clone(&s.adapter);
            let fetch = &adapter.fetch;
            let mail = crate::stores::take_landing_owed(gate, crate::stores::StoreId::LiveTv,
                || fetch.busy(), || fetch.take_current(|m| m.request));
            if let Some(mail) = mail.filter(|m| m.request == s.request) {
                dirty = true;
                match mail.answer.result {
                    Some(result) => {
                        s.reading_grid = None;
                        if let Some(roster) = result.roster { s.guides.retain(|g| roster.contains(&g.dvr)); }
                        if mail.answer.grid || !result.complete {
                            for guide in result.guides {
                                if let Some(old) = s.guides.iter_mut().find(|g| g.dvr == guide.dvr) { *old = guide; }
                                else { s.guides.push(guide); }
                            }
                        } else { s.guides = result.guides; }
                        if !mail.answer.grid {
                            if result.retry { s.backoff(); } else { s.strikes = 0; }
                        }
                        s.answered = true;
                        if !mail.answer.grid { s.complete = result.complete; }
                        s.failed = result.retry;
                    }
                    None => s.failed(),
                }
            }
            if fetch.take_refused(s.request) { s.failed(); dirty = true; }
        }
        // With no time to judge a due refresh by, nothing is asked (the next frame that has one asks).
        if let Some(now) = input.now {
            let reach = self.reach;
            let mut turn = self.fanout.turn(self.sources.iter().enumerate().map(|(i, s)| (i, s.standing(now, reach))));
            for (i, s) in self.sources.iter_mut().enumerate() {
                if s.standing(now, reach) != Standing::Wanting || !turn.admit(i) { continue; }
                let Some(key) = s.key else {
                    dirty |= !s.failed;
                    s.failed = true;
                    s.backoff();
                    turn.finished();
                    continue;
                };
                let work = s.work(now, reach).expect("a wanting Live TV source has work");
                s.reading_grid = match &work { Work::Discover => None, Work::Grid(g) => Some(g.dvr.clone()) };
                s.request = s.request.wrapping_add(1);
                s.adapter.fetch.claim(s.request);
                self.jobs.push(crate::stores::StoreJob::LiveTv(Job {
                    addr: Addr { to: MachineId::Store(crate::stores::StoreId::LiveTv.ord()), req: RequestId(s.request) },
                    req: LiveTvReq { sid: s.sid, key, now, work, until: reach },
                }, LiveTvPort(Arc::clone(&s.adapter))));
            }
        }
        (dirty && self.publish(keys, &input.recent)) | recent
    }
    /// The row alone, after the list moved: the lineup did not, so nothing else is rebuilt.
    fn publish_recent(&mut self, keys: &ClientKeys, seat: &RecentSeat) -> bool {
        let row = self.recent.row(seat, keys, &self.channels);
        if row.iter().copied().eq(self.recent_row.iter()) { return false; }
        self.recent_row = row.into_iter().cloned().collect();
        self.revision = self.revision.wrapping_add(1);
        true
    }
    fn publish(&mut self, keys: &ClientKeys, seat: &RecentSeat) -> bool {
        let mut channels: Vec<_> = self.sources.iter().flat_map(|s| s.guides.iter()
            .flat_map(move |g| g.channels.iter().map(move |c| Channel::of(s.sid, c)))).collect();
        channels.sort_by(|a, b| livetv::vcn_order(&a.vcn, &b.vcn));
        let status = if !channels.is_empty() { LiveTvStatus::Ready }
            else if self.sources.iter().any(|s| s.failed) { LiveTvStatus::Failed }
            else if self.sources.is_empty() || self.sources.iter().any(|s| !s.answered) { LiveTvStatus::Loading }
            else { LiveTvStatus::Empty };
        let recent = self.recent.row(seat, keys, &channels);
        if channels == self.channels && status == self.status && recent.iter().copied().eq(self.recent_row.iter()) { return false; }
        let recent: Vec<Channel> = recent.into_iter().cloned().collect();
        self.tiles = channels.iter().map(|c| Arc::new(channel_tile(c))).collect();
        self.recent_row = recent;
        self.channels = channels;
        self.status = status;
        self.revision = self.revision.wrapping_add(1);
        true
    }
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn install_for_test(&mut self, channels: Vec<Channel>) {
        self.status = if channels.is_empty() { LiveTvStatus::Empty } else { LiveTvStatus::Ready };
        self.tiles = channels.iter().map(|c| Arc::new(channel_tile(c))).collect();
        // A seated row is resolved by the next step, which holds the list and the roster it joins through.
        self.recent_row = self.recent.row(&RecentSeat::Unseated, &ClientKeys::default(), &channels).into_iter().cloned().collect();
        self.channels = channels;
        self.revision = self.revision.wrapping_add(1);
    }
}

/// The only IO in this model; even a panic answers once so the next pump releases the claim.
pub fn finish_livetv_job(port: &LiveTvPort, addr: Addr, client: &'static Client, req: LiveTvReq) {
    finish_with(port, addr, req, |req| read(client, req));
}
fn finish_with(port: &LiveTvPort, addr: Addr, req: LiveTvReq,
    read: impl FnOnce(&LiveTvReq) -> Option<ReadResult>) {
    let grid = matches!(req.work, Work::Grid(_));
    let result = catch_unwind(AssertUnwindSafe(|| read(&req))).unwrap_or(None);
    port.done(addr, LiveTvAnswer { result, grid });
}
trait LiveTvIo {
    fn dvrs(&self) -> (Option<i32>, Option<Vec<plx_plex::plex::Dvr>>);
    fn channels(&self, provider: &str) -> (Option<i32>, Option<Vec<plx_plex::plex::EpgChannel>>);
    fn guide(&self, provider: &str, now: i64, until: i64) -> Option<Vec<plx_plex::plex::Metadata>>;
}
impl LiveTvIo for Client {
    fn dvrs(&self) -> (Option<i32>, Option<Vec<plx_plex::plex::Dvr>>) { self.live_tv_dvrs() }
    fn channels(&self, provider: &str) -> (Option<i32>, Option<Vec<plx_plex::plex::EpgChannel>>) { self.epg_channels(provider) }
    fn guide(&self, provider: &str, now: i64, until: i64) -> Option<Vec<plx_plex::plex::Metadata>> { self.epg_guide(provider, now, until) }
}

/// What one lineup read says its HTTP status was: `none` when the transport never answered (so the
/// line cannot be mistaken for a refusal the server actually sent).
fn status_str(status: Option<i32>) -> String {
    status.map_or_else(|| "none".to_owned(), |s| s.to_string())
}

/// **One line per lineup read** (PR #993 review): a server, its HTTP status, how many DVRs and
/// channels it answered with, and how many channels were dropped and why. Without it a tab that
/// never appeared left nothing in the event log to say which read refused.
fn log_lineup(sid: ServerId, dvr: &str, status: Option<i32>, mapped: usize, kept: usize) {
    let dropped = mapped.saturating_sub(kept);
    plx_base::eventlog::log(&format!(
        "livetv: lineup sid={} dvr={dvr} status={} mapped={mapped} kept={kept} dropped={dropped}{}",
        sid.raw(),
        status_str(status),
        if dropped > 0 { " (no grid key)" } else { "" },
    ));
}

fn read(client: &impl LiveTvIo, req: &LiveTvReq) -> Option<ReadResult> {
    match &req.work {
        Work::Discover => {
            let (status, dvrs) = client.dvrs();
            let Some(dvrs) = dvrs else {
                plx_base::eventlog::log(&format!(
                    "livetv: dvrs sid={} status={} dvrs=none",
                    req.sid.raw(),
                    status_str(status),
                ));
                return None;
            };
            let total = dvrs.len();
            let dvrs: Vec<_> = livetv::usable_dvrs(&dvrs).collect();
            plx_base::eventlog::log(&format!(
                "livetv: dvrs sid={} status={} dvrs={total} usable={}",
                req.sid.raw(),
                status_str(status),
                dvrs.len(),
            ));
            let mut result = ReadResult { guides: Vec::new(), roster: Some(dvrs.iter().map(|d| d.key.clone()).collect()), complete: true, retry: false };
            for dvr in dvrs {
                let (status, channels) = client.channels(&dvr.epg_identifier);
                let Some(channels) = channels else {
                    plx_base::eventlog::log(&format!(
                        "livetv: lineup sid={} dvr={} status={} channels=none",
                        req.sid.raw(),
                        dvr.key,
                        status_str(status),
                    ));
                    result.complete = false; result.retry = true; continue;
                };
                let mapped = channels.len();
                let mut channels = livetv::lineup(dvr, &channels, &[], req.now);
                log_lineup(req.sid, &dvr.key, status, mapped, channels.len());
                if channels.is_empty() {
                    result.guides.push(Guide { dvr: dvr.key.clone(), provider: dvr.epg_identifier.clone(), channels,
                        refresh_at: i64::MAX, retry_cd: 0, strikes: 0, until: req.until });
                    continue;
                }
                let airings = client.guide(&dvr.epg_identifier, req.now, req.until);
                if let Some(airings) = airings.as_deref() { livetv::update_airings(&mut channels, airings, req.now); }
                let refresh_at = if airings.is_some() { Guide::next_refresh(&channels, req.now) }
                    else { result.retry = true; req.now };
                result.guides.push(Guide { dvr: dvr.key.clone(), provider: dvr.epg_identifier.clone(), channels, refresh_at, retry_cd: if airings.is_some() { 0 } else { RETRY_FRAMES }, strikes: 0, until: req.until });
            }
            Some(result)
        }
        Work::Grid(guide) => {
            let airings = client.guide(&guide.provider, req.now, req.until)?;
            let mut updated = guide.clone();
            livetv::update_airings(&mut updated.channels, &airings, req.now);
            updated.refresh_at = Guide::next_refresh(&updated.channels, req.now);
            updated.strikes = 0;
            updated.until = req.until;
            Some(ReadResult { guides: vec![updated], roster: None, complete: true, retry: false })
        }
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod recent_tests;
