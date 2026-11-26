//! Browse preference writes: pure requests, addressed receipts, worker-side atomic merges.
use std::sync::{Arc, Mutex};
use plx_machine::{job::{Job, Port}, machine::{Addr, MachineId, RequestId}};
use plx_plex::plex::session::{self, Session};
use super::saved_view::{self, ViewField, ViewSnapshot};
#[derive(Clone, Default, PartialEq, Eq)]
struct Account { client_id: String, token: String }
impl Account {
    fn of(s: &Session) -> Self { Self { client_id: s.client_id.clone(), token: s.account_token.clone() } }
    /// The one account test: a named account (never the empty one) and the same credentials.
    fn is(&self, client_id: &str, token: &str) -> bool { !self.client_id.is_empty() && self.client_id == client_id && self.token == token }
    fn matches(&self, s: &Session) -> bool { self.is(&s.client_id, &s.account_token) }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Edit {
    Library { kind: String, machine: String, key: i64 },
    View { machine: String, key: i64, snapshot: ViewSnapshot, field: ViewField },
    Pins { libraries: Vec<(String, i64, bool, bool)>, on: Vec<bool>, touched: Vec<bool>, asked: bool },
    /// The profile's Library order: `libs` is the favourites `(machine id, section key)` as the page
    /// showed them, first shown first, and `present` is every library the table held then, in the
    /// server's own order. `plex::library_order::merged` is the one place that turns them, with the
    /// record the write meets, into what is stored (or into no record).
    Order { libs: Vec<(String, i64)>, present: Vec<(String, i64)> },
    /// Back to the server order: the profile's record is deleted.
    OrderReset,
}
impl Edit {
    /// A Library order write ([`Edit::Order`] or [`Edit::OrderReset`]): what [`Preferences::order_write`]
    /// answers about.
    fn is_order(&self) -> bool { matches!(self, Self::Order { .. } | Self::OrderReset) }
    /// A later edit of this class takes the earlier one's place ([`Self::replaces`]), so the overlay
    /// keeps the newest of the class past its save and lets the next one retire it. A class that never
    /// replaces itself (a view edit, the pins) is done with once it is saved.
    fn replaces_itself(&self) -> bool { self.replaces(self) }
    fn merge(&self, current: &Session, user: &str) -> Option<Session> {
        let mut next = current.clone();
        match self {
            Self::Library { kind, machine, key } => {
                if !next.prefs.last_library.iter().any(|slot| slot.user == user) {
                    next.prefs.last_library.push(session::LastLibrary { user: user.to_owned(), libs: Vec::new(), extensions: Default::default() });
                }
                next.prefs.last_library.iter_mut().find(|slot| slot.user == user)?.set(kind, machine, *key);
            }
            Self::View { machine, key, snapshot, field } => {
                let stored = current.views_for(user).and_then(|views| views.get(machine, *key));
                let view = saved_view::merge(stored, machine, *key, snapshot, *field);
                if stored.map_or_else(|| view.is_default(), |stored| *stored == view) { return None; }
                next.set_view_for(user, view);
            }
            Self::Pins { libraries, on, touched, asked } => {
                let libraries: Vec<_> = libraries.iter().map(|(machine_id, key, household, household_type)|
                    plx_plex::plex::pins::LibRef { machine_id, key: *key, household: *household, household_type: *household_type }).collect();
                let previous = current.pins_for(user);
                let answers = plx_plex::plex::pins::answers(&libraries, on, touched, previous);
                let fresh = plx_plex::plex::pins::record(user, *asked, &libraries, &answers);
                next.set_pins_for(user, plx_plex::plex::pins::carry_forward(fresh, previous, &libraries));
            }
            Self::Order { libs, present } => {
                match plx_plex::plex::library_order::merged(current.order_for(user), libs, present) {
                    Some(list) => next.set_order_for(user, &list),
                    None => next.clear_order_for(user),
                }
                if next.prefs.library_order == current.prefs.library_order { return None; }
            }
            Self::OrderReset => {
                next.clear_order_for(user);
                if next.prefs.library_order == current.prefs.library_order { return None; }
            }
        }
        Some(next)
    }
    fn replaces(&self, other: &Self) -> bool {
        // Partial view edits must retain their order, including an earlier listing's sort.
        match (self, other) {
            (Self::Library { kind: a, .. }, Self::Library { kind: b, .. }) => a == b,
            // A later order, or a reset, supersedes an earlier one whole.
            (Self::Order { .. } | Self::OrderReset, Self::Order { .. } | Self::OrderReset) => true,
            _ => false,
        }
    }
}
#[derive(Clone, PartialEq, Eq)]
pub struct BrowsePreferencesReq { account: Account, user: String, edit: Arc<Edit>, tenure: u64 }
impl std::fmt::Debug for BrowsePreferencesReq {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.debug_struct("BrowsePreferencesReq").field("user", &self.user).field("edit", &self.edit).finish_non_exhaustive() }
}
impl BrowsePreferencesReq {
    pub fn matches_account(&self, s: &Session) -> bool { self.account.matches(s) }
    pub fn matches_tenure(&self, tenure: u64) -> bool { self.tenure == tenure }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteOutcome { NotStarted, Finished(bool) }
/// **Where the profile's last Library order write stands**: what Settings reads to tell a saved choice
/// from one that only stands for this run. It is read from the overlay's own entry for the write
/// ([`Preferences::order_write`]), so it cannot disagree with what the overlay presents.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WriteState {
    /// No Library order write by this profile is held (none this run, or the account or the
    /// revocation tenure it was made under has ended).
    #[default]
    Idle,
    /// Queued or running; the choice shows, the disk does not have it yet.
    InFlight,
    /// The session holds it.
    Saved,
    /// The write did not persist (`Finished(false)`: the file could not be written, or the worker
    /// panicked), or its job was refused (`NotStarted`: the runner refuses a job whose account or
    /// tenure moved, the same two tests the overlay already applies, so this seldom reads before the
    /// next `prepare` retires the entry). The choice still shows (the overlay keeps it), only for this
    /// run, and a Reset that failed still shows the order gone while the disk keeps its record.
    /// Nothing retries; the next commit is a new write.
    Failed,
}
#[derive(Default)]
pub(super) struct Receipt { result: Option<WriteOutcome>, observed: Option<WriteOutcome> }
impl Receipt {
    pub(super) fn observe(&mut self) { self.observed = self.result; }
    pub(super) fn saved(&self) -> bool { self.observed == Some(WriteOutcome::Finished(true)) }
    /// The write's standing as of the last [`Self::observe`].
    pub(super) fn state(&self) -> WriteState {
        match self.observed {
            None => WriteState::InFlight,
            Some(WriteOutcome::Finished(true)) => WriteState::Saved,
            Some(WriteOutcome::Finished(false) | WriteOutcome::NotStarted) => WriteState::Failed,
        }
    }
}
#[derive(Clone)]
pub struct BrowsePreferencesPort { addr: Addr, receipt: Arc<Mutex<Receipt>> }
impl PartialEq for BrowsePreferencesPort {
    fn eq(&self, other: &Self) -> bool { self.addr == other.addr && Arc::ptr_eq(&self.receipt, &other.receipt) }
}
impl std::fmt::Debug for BrowsePreferencesPort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.debug_tuple("BrowsePreferencesPort").field(&self.addr).finish() }
}
impl Port<WriteOutcome> for BrowsePreferencesPort {
    fn done(&self, addr: Addr, out: WriteOutcome) {
        if addr != self.addr { return; }
        let mut receipt = self.receipt.lock().unwrap_or_else(|e| e.into_inner());
        if receipt.result.is_none() { receipt.result = Some(out); }
    }
    fn refused(&self, addr: Addr) { self.done(addr, WriteOutcome::NotStarted); }
}
#[derive(Clone)]
struct Pending { account: Account, tenure: u64, user: String, edit: Arc<Edit>, receipt: Arc<Mutex<Receipt>> }
#[derive(Clone, Default)]
pub(super) struct Preferences {
    account: Account,
    tenure: u64,
    next: u32,
    jobs: Vec<(Job<BrowsePreferencesReq>, BrowsePreferencesPort)>,
    pending: Vec<Pending>,
}
impl Preferences {
    pub(super) fn prepare(&mut self, session: &Session, tenure: u64) {
        self.prepare_account(session);
        self.tenure = tenure;
    }

    pub(super) fn prepare_account(&mut self, session: &Session) { if self.account.client_id != session.client_id || self.account.token != session.account_token {
            self.account = Account::of(session);
        } }
    pub(super) fn emit(&mut self, user: &str, edit: Edit) -> Option<Arc<Mutex<Receipt>>> {
        if self.account.client_id.is_empty() { return None; }
        self.next = self.next.wrapping_add(1);
        let addr = Addr { to: MachineId::Store(crate::stores::StoreId::Browse.ord()), req: RequestId(self.next) };
        let receipt = Arc::new(Mutex::new(Receipt::default()));
        let edit = Arc::new(edit);
        let req = BrowsePreferencesReq { account: self.account.clone(), user: user.to_owned(), edit: Arc::clone(&edit), tenure: self.tenure };
        self.jobs.push((Job { addr, req }, BrowsePreferencesPort { addr, receipt: receipt.clone() }));
        // A saved write is done with, except the newest of a class that replaces itself (for each
        // profile): the next one retires it, and until then its receipt is what `order_write` reads.
        self.pending.retain(|p| (!p.receipt.lock().unwrap_or_else(|e| e.into_inner()).saved() || p.edit.replaces_itself())
            && p.tenure == self.tenure
            && (p.account != self.account || p.user != user || !edit.replaces(&p.edit)));
        if !matches!(edit.as_ref(), Edit::Pins { .. }) {
            self.pending.push(Pending { account: self.account.clone(), tenure: self.tenure, user: user.to_owned(), edit, receipt: receipt.clone() });
        }
        Some(receipt)
    }
    pub(super) fn take_jobs(&mut self) -> Vec<crate::stores::StoreJob> {
        std::mem::take(&mut self.jobs).into_iter().map(|(job, port)| crate::stores::StoreJob::BrowsePreferences(job, port)).collect()
    }
    pub(super) fn has_jobs(&self) -> bool { !self.jobs.is_empty() }
    pub(super) fn observe(&self) {
        for pending in &self.pending { pending.receipt.lock().unwrap_or_else(|e| e.into_inner()).observe(); }
    }
    /// One predicate for "this write is the profile's, under `account` (a client id and its token) and
    /// the tenure now": the overlay presents an entry only if it holds, and [`Self::order_write`] reads
    /// only entries it holds for.
    fn holds(&self, pending: &Pending, (client_id, token): (&str, &str), user: &str) -> bool {
        pending.user == user && pending.tenure == self.tenure && pending.account.is(client_id, token)
    }
    /// Where `user`'s newest Library order write stands, through the overlay's own filter.
    pub(super) fn order_write(&self, user: &str) -> WriteState {
        self.pending.iter().rev()
            .find(|p| p.edit.is_order() && self.holds(p, (&self.account.client_id, &self.account.token), user))
            .map_or(WriteState::Idle, |p| p.receipt.lock().unwrap_or_else(|e| e.into_inner()).state())
    }
    pub(super) fn overlay<'a>(&self, session: &'a Session, user: &str) -> std::borrow::Cow<'a, Session> {
        let mut next = std::borrow::Cow::Borrowed(session);
        for pending in self.pending.iter().filter(|p| self.holds(p, (&session.client_id, &session.account_token), user)
            && !p.receipt.lock().unwrap_or_else(|e| e.into_inner()).saved()) {
            if let Some(merged) = pending.edit.merge(next.as_ref(), user) { next = std::borrow::Cow::Owned(merged); }
        }
        next
    }
}
pub fn finish_browse_preferences_job(port: &BrowsePreferencesPort, addr: Addr, req: BrowsePreferencesReq) {
    let tenure = req.tenure;
    let saved = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut unchanged = false;
        let write = session::update_guarded_with_outcome(|current| {
            if !req.matches_account(current) || session::tenure() != tenure { return None; }
            let next = req.edit.merge(current, &req.user);
            unchanged = next.is_none();
            next
        }, || session::tenure() == tenure);
        unchanged || write.is_some_and(|write| write.outcome.persisted())
    })).unwrap_or(false);
    port.done(addr, WriteOutcome::Finished(saved));
}

#[cfg(test)]
mod tests {
    use super::*;
    fn account() -> Session { Session { client_id: "preferences-client".into(), account_token: "preferences-account".into(), ..Default::default() } }
    fn library(key: i64) -> Edit { Edit::Library { kind: super::super::SecKind::Movie.wire().into(), machine: "machine".into(), key } }
    fn key(s: &Session, user: &str) -> i64 { s.prefs.last_library.iter().find(|l| l.user == user).unwrap().libs[0].key }
    #[test]
    fn preference_steps_emit_jobs_and_only_the_exact_address_can_settle_once() {
        let mut writes = Preferences::default();
        writes.prepare(&account(), 0);
        let step = plx_base::step::StepScope::enter();
        let receipt = writes.emit("viewer", library(7)).unwrap();
        let mut jobs = writes.take_jobs();
        assert_eq!(jobs.len(), 1);
        let crate::stores::StoreJob::BrowsePreferences(job, port) = jobs.remove(0) else { panic!("preference job") };
        assert_eq!(job.addr.to, MachineId::Store(crate::stores::StoreId::Browse.ord()));
        let wrong = Addr { req: RequestId(job.addr.req.0.wrapping_add(1)), ..job.addr };
        port.refused(wrong);
        assert_eq!(receipt.lock().unwrap().result, None);
        port.refused(job.addr);
        port.done(job.addr, WriteOutcome::Finished(true));
        assert_eq!(receipt.lock().unwrap().result, Some(WriteOutcome::NotStarted));
        drop(step);
        writes.observe();
        assert!(!receipt.lock().unwrap().saved());
        assert_eq!(key(&writes.overlay(&account(), "viewer"), "viewer"), 7);
    }

    #[test]
    fn the_overlay_keeps_the_newest_saved_write_of_a_class_that_replaces_itself() {
        let pins = Edit::Pins { libraries: Vec::new(), on: Vec::new(), touched: Vec::new(), asked: true };
        assert!(!pins.replaces_itself());
        for edit in [library(1), Edit::Order { libs: Vec::new(), present: Vec::new() }, Edit::OrderReset] { assert!(edit.replaces_itself()); }
        let mut writes = Preferences::default();
        writes.prepare(&account(), 0);
        let settle = |writes: &mut Preferences, receipt: &Arc<Mutex<Receipt>>| {
            let mut jobs = writes.take_jobs();
            let crate::stores::StoreJob::BrowsePreferences(job, port) = jobs.remove(0) else { panic!("preference job") };
            port.done(job.addr, WriteOutcome::Finished(true));
            writes.observe();
            assert!(receipt.lock().unwrap().saved());
        };
        let first = writes.emit("viewer", library(1)).unwrap();
        settle(&mut writes, &first);
        let second = writes.emit("viewer", library(2)).unwrap();
        assert_eq!(writes.pending.len(), 1, "the next write of the class retires the saved one");
        settle(&mut writes, &second);
        let _ = writes.emit("viewer", Edit::OrderReset).unwrap();
        assert_eq!(writes.pending.len(), 2, "the newest saved library write stays until another replaces it");
    }

    #[test]
    fn pins_request_keeps_supplied_vector_allocations_without_retaining_an_overlay() {
        let captured = account();
        let mut writes = Preferences::default();
        writes.prepare(&captured, 42);
        let libraries = vec![("machine-a".to_owned(), 7, true, false), ("machine-b".to_owned(), 9, false, true)];
        let on = vec![true, false];
        let touched = vec![false, true];
        let library_rows = libraries.as_ptr();
        let machine = libraries[0].0.as_ptr();
        let on_rows = on.as_ptr();
        let touched_rows = touched.as_ptr();
        let expected = Edit::Pins { libraries: libraries.clone(), on: on.clone(), touched: touched.clone(), asked: true };
        let _ = writes.emit("viewer", Edit::Pins { libraries, on, touched, asked: true });
        let crate::stores::StoreJob::BrowsePreferences(job, _) = writes.take_jobs().remove(0) else { panic!("preference job") };
        let Edit::Pins { libraries, on, touched, asked } = job.req.edit.as_ref() else { panic!("pins") };
        assert_eq!(libraries.as_ptr(), library_rows);
        assert_eq!(libraries[0].0.as_ptr(), machine);
        assert_eq!(on.as_ptr(), on_rows);
        assert_eq!(touched.as_ptr(), touched_rows);
        assert!(*asked);
        assert_eq!(job.req.edit.as_ref(), &expected);
        assert!(writes.pending.is_empty(), "pins keep their existing separate receipt, not an overlay");
        let cloned = job.req.clone();
        assert!(Arc::ptr_eq(&job.req.edit, &cloned.edit));
        let mut equal = cloned;
        equal.edit = Arc::new(expected);
        assert_eq!(job.req, equal, "request equality compares edits, not allocation identity");
        assert!(!Arc::ptr_eq(&job.req.edit, &equal.edit));
        assert!(job.req.matches_account(&captured));
        assert!(job.req.matches_tenure(42));
        assert!(!job.req.matches_tenure(43));
        let debug = format!("{:?}", job.req);
        assert!(!debug.contains(&captured.account_token));
        assert!(!debug.contains(&captured.client_id));
    }

    #[test]
    fn library_and_view_requests_share_original_edits_with_the_retained_overlay() {
        use super::super::LibraryType;
        let captured = account();
        let mut writes = Preferences::default();
        writes.prepare(&captured, 42);
        let snapshot = ViewSnapshot { listing: LibraryType::Primary, sort_resolved: true, genre_resolved: true,
            sort: Some(("titleSort".into(), false)), unwatched: false, genre: Some("genre".into()) };
        for edit in [library(7), Edit::View { machine: "machine".into(), key: 7, snapshot, field: ViewField::Sort }] {
            let original_machine = match &edit {
                Edit::Library { machine, .. } | Edit::View { machine, .. } => machine.as_ptr(),
                Edit::Pins { .. } | Edit::Order { .. } | Edit::OrderReset => unreachable!(),
            };
            let expected = edit.clone();
            let _ = writes.emit("viewer", edit);
            let crate::stores::StoreJob::BrowsePreferences(job, _) = writes.take_jobs().remove(0) else { panic!("preference job") };
            let pending = writes.pending.last().unwrap();
            assert!(Arc::ptr_eq(&pending.edit, &job.req.edit));
            assert_eq!(job.req.edit.as_ref(), &expected);
            let machine = match job.req.edit.as_ref() {
                Edit::Library { machine, .. } | Edit::View { machine, .. } => machine,
                Edit::Pins { .. } | Edit::Order { .. } | Edit::OrderReset => unreachable!(),
            };
            assert_eq!(machine.as_ptr(), original_machine);
            assert!(job.req.matches_account(&captured));
            assert!(job.req.matches_tenure(42));
            let mut equal = job.req.clone();
            equal.edit = Arc::new(expected);
            assert_eq!(job.req, equal);
            assert!(!Arc::ptr_eq(&job.req.edit, &equal.edit));
        }
        let overlaid = writes.overlay(&captured, "viewer");
        assert_eq!(key(&overlaid, "viewer"), 7);
        assert_eq!(overlaid.views_for("viewer").unwrap().get("machine", 7).unwrap().primary_sort(), Some(("titleSort", false)));
    }

    #[test]
    fn real_browse_commands_with_a_captured_account_emit_all_three_writes_under_step_scope() {
        use crate::stores::browse::{BrowseCmd, LibraryWork, QueryEdit, SectionAddress};
        use super::super::{BrowseAdapter, BrowseState, SecKind, a_source};
        let _serial = plx_base::testlock::serial();
        let _session = session::TempSession::new("browse-preference-step");
        let captured = account();
        let mut state = BrowseState::default();
        let mut source = a_source("machine", "", true);
        let sid = plx_plex::plex::ServerId::from_raw(7); source.sid = sid;
        state.sources.push(source);
        state.append_sections_for_test(0, vec![(1, "Movies".into(), SecKind::Movie)]);
        state.sections[0].pinned = true;
        state.profile_key = "captured-profile".into();
        let target = SectionAddress { epoch: state.table_epoch(), sid, section: 1 };
        let adapter = Arc::new(BrowseAdapter::default());
        let keys = plx_plex::plex::ClientKeys::default();
        let mut other_jobs = Vec::new();
        state.preference_writes.prepare(&captured, session::tenure());
        let step = plx_base::step::StepScope::enter();
        assert!(state.run_owned(&adapter, BrowseCmd::Addressed { target, work: LibraryWork::Commit {
            select: true, choice: true, query: Some(QueryEdit::Unwatched(true)),
        } }, &keys, &captured, &mut other_jobs));
        assert!(state.run_owned(&adapter, BrowseCmd::ApplyPins(vec![(0, true)]), &keys, &captured, &mut other_jobs));
        let jobs = state.take_preference_jobs();
        assert_eq!(jobs.len(), 3, "library, query and pin persistence all leave as jobs");
        assert!(other_jobs.is_empty());
        assert!(state.states[0].unwatched, "the UI applies its edit before persistence");
        assert!(state.pending_pins.is_some());
        for job in jobs {
            let crate::stores::StoreJob::BrowsePreferences(job, port) = job else { panic!("preference job") };
            assert_eq!(job.req.user, "captured-profile");
            assert!(job.req.matches_account(&captured));
            port.refused(job.addr);
        }
        drop(step);
    }
    #[test]
    fn newer_choices_and_other_profiles_survive_old_completions_and_stale_captures() {
        let session = account();
        let mut writes = Preferences::default();
        writes.prepare(&session, 0);
        writes.emit("viewer-a", library(1)).unwrap();
        writes.emit("viewer-b", library(2)).unwrap();
        writes.emit("viewer-a", library(3)).unwrap();
        let jobs = writes.take_jobs();
        assert_eq!(jobs.len(), 3);
        let crate::stores::StoreJob::BrowsePreferences(job, port) = &jobs[0] else { panic!("preference job") };
        port.done(job.addr, WriteOutcome::Finished(true));
        writes.observe();
        assert_eq!(key(&writes.overlay(&session, "viewer-a"), "viewer-a"), 3);
        assert_eq!(key(&writes.overlay(&session, "viewer-b"), "viewer-b"), 2);
        let crate::stores::StoreJob::BrowsePreferences(job, port) = &jobs[2] else { panic!("preference job") };
        port.done(job.addr, WriteOutcome::Finished(true));
        assert_eq!(key(&writes.overlay(&session, "viewer-a"), "viewer-a"), 3,
            "a completed worker does not pair itself with an older captured session");
        writes.observe();
        assert!(writes.overlay(&session, "viewer-a").prefs.last_library.is_empty());
        let mut replacement = session.clone(); replacement.account_token = "replacement".into();
        assert!(writes.overlay(&replacement, "viewer-b").prefs.last_library.is_empty());
    }

    #[test]
    fn pending_view_edits_merge_in_order_across_a_listing_change() {
        use super::super::LibraryType;
        let stored = account();
        let mut writes = Preferences::default(); writes.prepare(&stored, 0);
        let primary = ViewSnapshot { listing: LibraryType::Primary, sort_resolved: true, genre_resolved: true,
            sort: Some(("viewCount".into(), true)), unwatched: false, genre: None };
        let collections = ViewSnapshot { listing: LibraryType::Collections, sort_resolved: false,
            sort: None, ..primary.clone() };
        let collection_sort = ViewSnapshot { sort_resolved: true, sort: Some(("titleSort".into(), false)), ..collections.clone() };
        for (snapshot, field) in [(primary, ViewField::Sort), (collections, ViewField::Listing), (collection_sort, ViewField::Sort)] {
            let _ = writes.emit("viewer", Edit::View { machine: "machine".into(), key: 1, snapshot, field });
        }
        let overlaid = writes.overlay(&stored, "viewer");
        let view = overlaid.views_for("viewer").unwrap().get("machine", 1).unwrap();
        assert_eq!(view.primary_sort(), Some(("viewCount", true)), "the unresolved listing must merge over earlier pending edits");
        assert_eq!(view.listing_sort, "titleSort");
    }
    #[test]
    fn worker_merges_the_original_profile_over_current_authority_without_losing_other_edits() {
        let _serial = plx_base::testlock::serial();
        let _session = session::TempSession::new("browse-preference-merge");
        let stored = account();
        session::save(&stored);
        let mut writes = Preferences::default(); writes.prepare(&stored, session::tenure());
        let _ = writes.emit("original-profile", library(1));
        let _ = writes.emit("original-profile", library(2));
        session::save(&stored.with_auto_sign_in(true));
        for job in writes.take_jobs() {
            let crate::stores::StoreJob::BrowsePreferences(job, port) = job else { panic!("preference job") };
            assert!(job.req.matches_tenure(session::tenure()));
            finish_browse_preferences_job(&port, job.addr, job.req);
        }
        let persisted = session::peek();
        assert_eq!(key(&persisted, "original-profile"), 2);
        assert!(persisted.auto_sign_in(), "the worker merges, never replaces its captured record");
        writes.observe();
        assert!(writes.pending.iter().all(|p| p.receipt.lock().unwrap().saved()));
    }
    #[test]
    fn an_obsolete_account_or_revocation_lease_never_writes() {
        let _serial = plx_base::testlock::serial();
        let _session = session::TempSession::new("browse-preference-stale");
        let stored = account(); session::save(&stored);
        let mut writes = Preferences::default(); writes.prepare(&stored, session::tenure());
        let receipt = writes.emit("viewer", library(4)).unwrap();
        let crate::stores::StoreJob::BrowsePreferences(mut job, port) = writes.take_jobs().remove(0) else { panic!("preference job") };
        job.req.tenure = session::tenure().wrapping_add(1);
        finish_browse_preferences_job(&port, job.addr, job.req);
        assert_eq!(receipt.lock().unwrap().result, Some(WriteOutcome::Finished(false)));
        assert!(session::peek().prefs.last_library.is_empty());
        let receipt = writes.emit("viewer", library(5)).unwrap();
        let crate::stores::StoreJob::BrowsePreferences(job, port) = writes.take_jobs().remove(0) else { panic!("preference job") };
        assert!(job.req.matches_tenure(session::tenure()));
        let mut replacement = stored; replacement.account_token = "other-account".into(); session::save(&replacement);
        finish_browse_preferences_job(&port, job.addr, job.req);
        assert_eq!(receipt.lock().unwrap().result, Some(WriteOutcome::Finished(false)));
        assert!(session::peek().prefs.last_library.is_empty());
    }
}
