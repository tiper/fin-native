//! Settings > Libraries > Library order: the lifecycle ([`Mode`]) and the sentence each state says,
//! the one write a drop sends, Back and the keys that must not leak while a library is held, Reset
//! (inert without a record, confirmed by a second OK), the header preview's overflow, the root's
//! value, and the fit of every string in every shipped language.
use super::*;
use super::super::test_support::*;
use plx_data::browse::{OrderSlot, OrderState, SecKind, WriteState};
use plx_data::stores::browse::DirectorySnapshot;
use plx_machine::present::Present;
use plx_os::i18n::{language_on_this_thread_for_test, SHIPPED};

const RECORDED: OrderState = OrderState::Recorded { in_effect: true };

fn row(section: usize, title: &str, kind: SecKind, handle: &str) -> LibraryOrderRow {
    LibraryOrderRow { section, machine_id: format!("m{}", if handle.is_empty() { 0 } else { 1 }), key: section as i64 + 1, title: title.into(), kind, handle: handle.into() }
}

/// Six favourites across the two kinds, the server's order: Movies, TV Shows, 4K Movies, Kids, Anime (a
/// friend's), Documentaries.
fn six() -> Vec<LibraryOrderRow> {
    vec![
        row(0, "Movies", SecKind::Movie, ""), row(1, "TV Shows", SecKind::Show, ""), row(2, "4K Movies", SecKind::Movie, ""),
        row(3, "Kids", SecKind::Show, ""), row(4, "Anime", SecKind::Show, "anna"), row(5, "Documentaries", SecKind::Show, ""),
    ]
}

fn snap(rows: Vec<LibraryOrderRow>, state: OrderState, write: WriteState) -> DirectorySnapshot {
    // a fixture's table is its favourites, in section order (the server's order when it is the default)
    let mut slots: Vec<OrderSlot> = rows.iter()
        .map(|row| OrderSlot { section: row.section, machine_id: row.machine_id.clone(), key: row.key }).collect();
    slots.sort_by_key(|slot| slot.section);
    DirectorySnapshot::default().with_library_order(rows, slots, state, write)
}

/// What a step sent to the Browse store.
#[derive(Debug, PartialEq)]
enum Sent {
    Set(Vec<(String, i64)>),
    Reset,
}

fn sent(out: &[Stamped<InnerHost>]) -> Vec<Sent> {
    out.iter().filter_map(|e| match &e.fx {
        Fx::App(AppFx::Store(StoreId::Browse, StoreCmd::Browse(BrowseCmd::SetLibraryOrder(libs)))) => Some(Sent::Set(libs.clone())),
        Fx::App(AppFx::Store(StoreId::Browse, StoreCmd::Browse(BrowseCmd::ResetLibraryOrder))) => Some(Sent::Reset),
        _ => None,
    }).collect()
}

fn step_page(page: &mut LibraryOrderPage, snap: &DirectorySnapshot, ev: ScreenEvent<InnerHost>) -> (Handled, Vec<Stamped<InnerHost>>) {
    let mut out = Vec::new();
    let mut present = Present::new();
    let mut fx = Effects::new(&mut out, MachineId::Instance(InstanceId(0)), &mut present);
    let handled = page.step(&ev, &cx_with(None, snap.view()), &mut fx);
    (handled, out)
}

fn key_ev(key: Key) -> ScreenEvent<InnerHost> {
    ScreenEvent::Input(InputEvent {
        at: plx_machine::machine::Tick::default(),
        source: plx_machine::machine::Source::Sdl,
        kind: InputKind::Key { key, sym: 0, wcode: 0, edge: Edge::Down, at_edge: false },
    })
}

fn press(page: &mut LibraryOrderPage, snap: &DirectorySnapshot, key: Key) -> (Handled, Vec<Stamped<InnerHost>>) {
    step_page(page, snap, key_ev(key))
}

/// OK on a row: the activation a press commit delivers.
fn ok_on(page: &mut LibraryOrderPage, snap: &DirectorySnapshot, elem: u32) -> Vec<Stamped<InnerHost>> {
    step_page(page, snap, ScreenEvent::Activate(elem)).1
}

fn focus_on(page: &mut LibraryOrderPage, snap: &DirectorySnapshot, elem: u32) {
    let to = FocusKey { entry: page.entry, elem };
    step_page(page, snap, ScreenEvent::FocusMoved { from: None, to, by: plx_ui::screen::By::Dir });
}

/// OK on the row of the library that opens table index `section`.
fn ok_lib(page: &mut LibraryOrderPage, snap: &DirectorySnapshot, section: usize) -> Vec<Stamped<InnerHost>> {
    let elem = page.elem_of(section);
    ok_on(page, snap, elem)
}

fn focus_lib(page: &mut LibraryOrderPage, snap: &DirectorySnapshot, section: usize) {
    let elem = page.elem_of(section);
    focus_on(page, snap, elem);
}

/// The identity of the library that opens table index `section`, as the page shows it now.
fn lib(page: &LibraryOrderPage, section: usize) -> LibraryRef {
    LibraryRef::of(page.state.shown.iter().find(|row| row.section == section).expect("shown")).expect("a named library")
}

fn order(page: &LibraryOrderPage) -> Vec<usize> {
    page.state.shown.iter().map(|r| r.section).collect()
}

/// **What the real store publishes** for the order `sent` over the table `default` (the server's
/// order): plex's own rule, [`plx_plex::plex::library_order::effective`], which places a library no
/// order names (no machine id yet) right after its default predecessor. A fixture of an echo is built
/// by it, never written by hand, so it is a publication the store can produce.
fn published(default: &[LibraryOrderRow], sent: &[(String, i64)]) -> Vec<LibraryOrderRow> {
    use plx_plex::plex::library_order::{effective, Lib};
    use plx_plex::plex::session::{LibraryOrder, OrderedLib};
    let libs: Vec<Lib<'_>> = default.iter().map(|row| Lib { machine_id: &row.machine_id, key: row.key }).collect();
    let record = LibraryOrder {
        libs: sent.iter().map(|(machine_id, key)| OrderedLib { machine_id: machine_id.clone(), key: *key, ..Default::default() }).collect(),
        ..Default::default()
    };
    effective(&libs, Some(&record)).into_iter().map(|index| default[index].clone()).collect()
}

fn page_of(snap: &DirectorySnapshot) -> LibraryOrderPage {
    LibraryOrderPage::new(EntryId(0), snap.view())
}

fn pick_up(page: &mut LibraryOrderPage, snap: &DirectorySnapshot, section: usize) {
    focus_lib(page, snap, section);
    assert!(sent(&ok_lib(page, snap, section)).is_empty(), "picking up writes nothing");
}

// ---- the lifecycle ----------------------------------------------------------------------------

#[test]
fn pickup_move_and_drop_sends_exactly_one_set_with_the_whole_shown_order() {
    let s = snap(six(), OrderState::Server, WriteState::Idle);
    let mut page = page_of(&s);
    pick_up(&mut page, &s, 4);
    assert!(matches!(&page.state.mode, Mode::Moving { lifted, .. } if *lifted == lib(&page, 4)));
    for _ in 0..3 {
        let (handled, out) = press(&mut page, &s, Key::Up);
        assert_eq!(handled, Handled::Yes, "the page takes Up while a library is held");
        assert!(sent(&out).is_empty(), "moving writes nothing");
    }
    press(&mut page, &s, Key::Down);
    assert_eq!(order(&page), [0, 1, 4, 2, 3, 5], "Anime went up three and down one");
    let out = ok_lib(&mut page, &s, 4);
    assert_eq!(sent(&out), [Sent::Set(vec![
        ("m0".into(), 1), ("m0".into(), 2), ("m1".into(), 5), ("m0".into(), 3), ("m0".into(), 4), ("m0".into(), 6),
    ])], "one command, the whole shown order, by (machine, section key)");
    assert_eq!(page.state.mode, Mode::Dropped(lib(&page, 4)));
}

#[test]
fn a_drop_that_moved_nothing_writes_nothing() {
    let s = snap(six(), OrderState::Server, WriteState::Idle);
    let mut page = page_of(&s);
    pick_up(&mut page, &s, 2);
    press(&mut page, &s, Key::Up);
    press(&mut page, &s, Key::Down);
    assert!(sent(&ok_lib(&mut page, &s, 2)).is_empty());
    assert_eq!(page.state.mode, Mode::Idle, "nothing was sent, so no place is claimed");
    assert!(page.state.description().starts_with("Press OK to move 4K Movies"), "{}", page.state.description());
}

/// A library whose server has not reported its machine id is no library an order can name. The
/// command a drop sends is the order of the libraries that CAN be named, in the order shown (here
/// five of six); the unnamed one is not in it, cannot be picked up, and is laid out by the store's
/// rule while the named ones move. An empty command is never sent.
#[test]
fn a_library_whose_server_has_no_machine_id_yet_is_left_out_of_the_command() {
    let mut rows = six();
    rows[1].machine_id.clear();
    let s = snap(rows, OrderState::Server, WriteState::Idle);
    let mut page = page_of(&s);
    pick_up(&mut page, &s, 3);
    press(&mut page, &s, Key::Up);
    let Some(Sent::Set(libs)) = sent(&ok_lib(&mut page, &s, 3)).pop() else { panic!("a set") };
    assert_eq!(libs.len(), 5);
    assert!(libs.iter().all(|(machine, _)| !machine.is_empty()), "never an empty machine id: {libs:?}");
    let keys: Vec<i64> = libs.iter().map(|(_, key)| *key).collect();
    assert_eq!(keys, [1, 4, 3, 5, 6], "the named libraries, in the order shown (Kids moved up one)");
}

#[test]
fn a_move_steps_over_a_library_no_order_can_name_and_the_page_shows_what_the_store_will_publish() {
    // shown = [A, U (unnamed), B, C, ...]: A down trades with B, never with U, and U follows its
    // default predecessor A (the store's rule), so the page reads [B, A, U, C, ...] before any echo
    let mut table = six();
    table[1].machine_id.clear();
    let s = snap(table.clone(), OrderState::Server, WriteState::Idle);
    let mut page = page_of(&s);
    pick_up(&mut page, &s, 0);
    press(&mut page, &s, Key::Down);
    assert_eq!(order(&page), [2, 0, 1, 3, 4, 5], "[B, A, U, C, ..]: A took B's place and U kept following A");
    let Some(Sent::Set(libs)) = sent(&ok_lib(&mut page, &s, 0)).pop() else { panic!("a move of the named order sends it") };
    let keys: Vec<i64> = libs.iter().map(|(_, key)| *key).collect();
    assert_eq!(keys, [3, 1, 4, 5, 6]);
    assert!(matches!(page.state.mode, Mode::Dropped(_)));
    assert!(matches!(page.state.basis, Shown::AwaitingEcho { .. }));
    // the store's echo of that order, built by plex's own rule (not by the page's): the same rows
    let echo = snap(published(&table, &libs), RECORDED, WriteState::InFlight);
    let echoed: Vec<usize> = echo.view().library_order().iter().map(|r| r.section).collect();
    assert_eq!(echoed, order(&page), "the page already shows the store's publication");
    // let the move's glide finish, then the echo lands: it moves no row
    for _ in 0..400 {
        step_page(&mut page, &s, tick_ev());
        if !page.form.table.gliding() { break; }
    }
    assert!(!page.form.table.gliding(), "the move's glide settled");
    step_page(&mut page, &echo, tick_ev());
    assert_eq!(page.state.basis, Shown::Published);
    assert_eq!(order(&page), echoed);
    assert!(!page.form.table.gliding(), "nothing glides at the echo");
}

#[test]
fn an_echo_is_still_the_order_sent_when_a_library_gains_its_machine_id_during_the_wait() {
    // shown = [A, U (unnamed), B, ...]; the drop names the five that can be named
    let mut table = six();
    table[1].machine_id.clear();
    let s = snap(table.clone(), OrderState::Server, WriteState::Idle);
    let mut page = page_of(&s);
    pick_up(&mut page, &s, 0);
    press(&mut page, &s, Key::Down);
    let Some(Sent::Set(libs)) = sent(&ok_lib(&mut page, &s, 0)).pop() else { panic!("a set") };
    assert_eq!(libs.len(), 5);
    // the machine id of U is learned while the write is out, so the publication names six libraries
    let mut learned = table.clone();
    learned[1].machine_id = "m0".into();
    let echo = snap(published(&learned, &libs), RECORDED, WriteState::InFlight);
    assert_eq!(echo.view().library_order().iter().filter(|r| LibraryRef::of(r).is_some()).count(), 6);
    step_page(&mut page, &echo, tick_ev());
    assert_eq!(page.state.basis, Shown::Published, "the order sent is in the publication, among the six: the wait ends on it");
    assert!(matches!(page.state.mode, Mode::Dropped(_)), "and the place is announced, not given up on");
}

#[test]
fn a_round_trip_and_a_drop_that_sends_nothing_leave_the_rows_the_store_published() {
    use plx_data::browse::library_order_publication;
    // the REAL store's publication. Server order [A, X, U, B]: X is no favourite but the record names
    // it, U is a favourite whose server has no machine id. The record [B, X, A] publishes [B, U, A].
    let libs = [
        ("home", 1, "A", SecKind::Movie, true), ("home", 2, "X", SecKind::Show, false),
        ("", 3, "U", SecKind::Show, true), ("nas", 4, "B", SecKind::Movie, true),
    ];
    let s = library_order_publication(&libs, &[("nas", 4), ("home", 2), ("home", 1)], &[]);
    let titles = |page: &LibraryOrderPage| page.state.shown.iter().map(|row| row.title.clone()).collect::<Vec<_>>();
    let mut page = page_of(&s);
    assert_eq!(titles(&page), ["B", "U", "A"], "the store's publication");
    let b = s.view().library_order().iter().find(|row| row.title == "B").map(|row| row.section).expect("B");
    pick_up(&mut page, &s, b);
    press(&mut page, &s, Key::Down);
    assert_eq!(titles(&page), ["A", "U", "B"], "laid out as if exactly [A, B] were recorded: X's place is the store's, not the page's");
    press(&mut page, &s, Key::Up);
    assert_eq!(titles(&page), ["B", "A", "U"], "back in the named order, but not the order the record published");
    assert!(sent(&ok_lib(&mut page, &s, b)).is_empty(), "the named order is the one that was held: nothing is sent");
    assert_eq!(page.state.mode, Mode::Idle);
    assert_eq!(page.state.basis, Shown::Published);
    assert_eq!(titles(&page), ["B", "U", "A"], "no order was written, so the rows are the ones the store published");
}

#[test]
fn a_drop_that_sends_an_order_shows_the_rows_the_real_store_publishes_for_it() {
    use plx_data::browse::library_order_publication;
    // the same table and record as above (X, no favourite, is in the record), but the order is changed
    let libs = [
        ("home", 1, "A", SecKind::Movie, true), ("home", 2, "X", SecKind::Show, false),
        ("", 3, "U", SecKind::Show, true), ("nas", 4, "B", SecKind::Movie, true),
    ];
    let s = library_order_publication(&libs, &[("nas", 4), ("home", 2), ("home", 1)], &[]);
    let mut page = page_of(&s);
    let b = s.view().library_order().iter().find(|row| row.title == "B").map(|row| row.section).expect("B");
    pick_up(&mut page, &s, b);
    press(&mut page, &s, Key::Down);
    let Some(Sent::Set(sent_order)) = sent(&ok_lib(&mut page, &s, b)).pop() else { panic!("the named order changed: it is sent") };
    // the SAME store, which already holds [B, X, A], is told the order the page sent: X is a present
    // server's library the order does not name, so the writer drops it
    let echo = library_order_publication(&libs, &[("nas", 4), ("home", 2), ("home", 1)],
        &sent_order.iter().map(|(machine, key)| (machine.as_str(), *key)).collect::<Vec<_>>());
    let published: Vec<String> = echo.view().library_order().iter().map(|row| row.title.clone()).collect();
    let shown: Vec<String> = page.state.shown.iter().map(|row| row.title.clone()).collect();
    assert_eq!(shown, published, "what the page shows after the drop is what the store publishes for it");
    step_page(&mut page, &echo, tick_ev());
    assert_eq!(page.state.basis, Shown::Published, "and the echo is that order");
    assert_eq!(page.state.shown.iter().map(|row| row.title.clone()).collect::<Vec<_>>(), published);
}

#[test]
fn a_drop_that_changes_no_named_order_sends_nothing_and_claims_no_place() {
    // the one named library has nowhere to go: the walls are the named libraries', not the rows'
    let mut rows = six();
    rows.truncate(2);
    rows[1].machine_id.clear();
    let s = snap(rows, OrderState::Server, WriteState::Idle);
    let mut page = page_of(&s);
    pick_up(&mut page, &s, 0);
    press(&mut page, &s, Key::Down);
    assert_eq!(order(&page), [0, 1], "no named library to trade with");
    assert!(sent(&ok_lib(&mut page, &s, 0)).is_empty());
    assert_eq!(page.state.mode, Mode::Idle);
    assert_eq!(page.state.basis, Shown::Published);
}

#[test]
fn the_ends_are_walls_for_the_held_library() {
    let s = snap(six(), OrderState::Server, WriteState::Idle);
    let mut page = page_of(&s);
    pick_up(&mut page, &s, 0);
    press(&mut page, &s, Key::Up);
    assert_eq!(order(&page), [0, 1, 2, 3, 4, 5]);
    ok_lib(&mut page, &s, 0);
    pick_up(&mut page, &s, 5);
    press(&mut page, &s, Key::Down);
    assert_eq!(order(&page), [0, 1, 2, 3, 4, 5]);
}

#[test]
fn back_while_moving_restores_sends_nothing_and_leaves_the_page_where_it_is() {
    let s = snap(six(), OrderState::Server, WriteState::Idle);
    let mut page = page_of(&s);
    pick_up(&mut page, &s, 3);
    press(&mut page, &s, Key::Up);
    press(&mut page, &s, Key::Up);
    assert_ne!(order(&page), [0, 1, 2, 3, 4, 5]);
    let (handled, out) = press(&mut page, &s, Key::Back);
    assert_eq!(handled, Handled::Yes, "Back is the page's while a library is held");
    assert!(sent(&out).is_empty() && !out.iter().any(|e| matches!(e.fx, Fx::Nav(_))), "nothing written, nothing popped");
    assert_eq!(order(&page), [0, 1, 2, 3, 4, 5], "the library is back where it was picked up");
    assert_eq!(page.state.mode, Mode::Idle);
    assert_eq!(press(&mut page, &s, Key::Back).0, Handled::No, "the next Back is the surface's");
}

#[test]
fn left_and_right_are_swallowed_while_a_library_is_held_and_the_table_says_so() {
    let s = snap(six(), OrderState::Server, WriteState::Idle);
    let mut page = page_of(&s);
    assert!(!page.view().table.uncommitted);
    assert_eq!(press(&mut page, &s, Key::Left).0, Handled::No);
    pick_up(&mut page, &s, 1);
    assert!(page.view().table.uncommitted, "Left is a Stop at the table's edge, not a Back");
    for key in [Key::Left, Key::Right] {
        assert_eq!(press(&mut page, &s, key).0, Handled::Yes);
    }
    assert_eq!(order(&page), [0, 1, 2, 3, 4, 5]);
}

#[test]
fn the_back_key_through_the_surface_restores_while_moving_and_pops_when_idle() {
    let _g = plx_base::testlock::serial();
    let _sess = scratch_session("library-order-back");
    let s = snap(six(), OrderState::Server, WriteState::Idle);
    let mut surface = RouteSurface::new(EntryId(7), InstanceId(0), Family::Settings, SettingsPage::Root);
    let drive = |surface: &mut RouteSurface, ev: ScreenEvent<InnerHost>| {
        let mut out = Vec::new();
        let mut present = Present::new();
        let mut fx = Effects::new(&mut out, MachineId::Instance(InstanceId(0)), &mut present);
        let handled = <RouteSurface as Machine<InnerHost>>::step(surface, &ev, &cx_with(None, s.view()), &mut fx);
        (handled, out)
    };
    drive(&mut surface, ScreenEvent::Mount);
    let mut out = Vec::new();
    let mut present = Present::new();
    let mut sink = Effects::new(&mut out, MachineId::Instance(InstanceId(0)), &mut present);
    surface.forward(vec![Stamped { from: MachineId::Instance(InstanceId(1)), fx: Fx::Nav(NavOp::Push(SettingsPage::LibraryOrder)) }], &cx_with(None, s.view()), &mut sink);
    assert_eq!(surface.inner.depth(), 2, "the page is on the root");
    assert_eq!(name(&surface), crate::registry::word::LIBRARY_ORDER);
    drive(&mut surface, ScreenEvent::Mount);
    let elem = LIB_KEY_BASE;
    drive(&mut surface, ScreenEvent::Activate(elem));
    drive(&mut surface, key_ev(Key::Down));
    let (handled, out) = drive(&mut surface, back_key());
    assert_eq!(handled, Handled::Yes);
    assert_eq!(surface.inner.depth(), 2, "Back while a library is held does not pop");
    assert!(!out.iter().any(|e| matches!(e.fx, Fx::Nav(_))) && sent(&out).is_empty());
    drive(&mut surface, back_key());
    assert_eq!(surface.inner.depth(), 1, "and the next Back pops as usual");
}

// ---- Reset ------------------------------------------------------------------------------------

fn reset_stops(page: &LibraryOrderPage) -> bool {
    page.form.step_key(page.state.keys.key_of(&LibId::of(page.state.shown.last().expect("rows"))), 1) == Some(LibId::Reset.key())
}

#[test]
fn focus_skips_reset_when_nothing_is_recorded_and_stops_on_it_when_something_is() {
    let none = snap(six(), OrderState::Server, WriteState::Idle);
    assert!(!reset_stops(&page_of(&none)), "no record: Reset is inert");
    let recorded = snap(six(), RECORDED, WriteState::Saved);
    assert!(reset_stops(&page_of(&recorded)));
}

#[test]
fn reset_stays_offered_after_a_failed_write_and_a_recorded_order_that_moves_nothing() {
    let failed = snap(six(), OrderState::Recorded { in_effect: true }, WriteState::Failed);
    assert!(reset_stops(&page_of(&failed)), "a failed write leaves a record standing");
    let idle = snap(six(), OrderState::Recorded { in_effect: false }, WriteState::Saved);
    assert!(reset_stops(&page_of(&idle)), "a record that moves nothing now still has something to delete");
}

#[test]
fn ok_then_ok_resets_and_any_other_key_keeps_the_order() {
    let s = snap(six(), RECORDED, WriteState::Saved);
    // a second OK resets
    let mut page = page_of(&s);
    focus_on(&mut page, &s, reset_elem_for_test());
    assert!(sent(&ok_on(&mut page, &s, reset_elem_for_test())).is_empty(), "the first OK only asks");
    assert_eq!(page.state.mode, Mode::ConfirmReset);
    assert_eq!(press(&mut page, &s, Key::Ok).0, Handled::No, "the second OK reaches the press");
    assert_eq!(sent(&ok_on(&mut page, &s, reset_elem_for_test())), [Sent::Reset]);
    assert_eq!(page.state.mode, Mode::ResetDone);
    // every other key cancels
    for key in [Key::Up, Key::Down, Key::Left, Key::Right, Key::Back] {
        let mut page = page_of(&s);
        focus_on(&mut page, &s, reset_elem_for_test());
        ok_on(&mut page, &s, reset_elem_for_test());
        let (handled, out) = press(&mut page, &s, key);
        assert_eq!(handled, Handled::Yes, "{key:?} is swallowed, not acted on");
        assert!(sent(&out).is_empty());
        assert_eq!(page.state.mode, Mode::Idle, "{key:?} keeps the order");
    }
}

#[test]
fn after_a_reset_the_publication_takes_reset_away_and_focus_lands_on_the_last_library() {
    let recorded = snap(six(), RECORDED, WriteState::Saved);
    let mut page = page_of(&recorded);
    focus_on(&mut page, &recorded, reset_elem_for_test());
    ok_on(&mut page, &recorded, reset_elem_for_test());
    ok_on(&mut page, &recorded, reset_elem_for_test());
    let done = snap(six(), OrderState::Server, WriteState::Saved);
    step_page(&mut page, &done, ScreenEvent::Tick(plx_machine::machine::Tick { ms: 16, dt_us: 16_000 }));
    assert!(!reset_stops(&page), "the store has no record now: Reset is inert");
    assert_eq!(page.form.selected_key(), Some(page.state.keys.key_of(&LibId::of(page.state.shown.last().unwrap()))), "focus is on the last library");
}

// ---- what the page shows until the store says otherwise ---------------------------------------

#[test]
fn the_page_keeps_its_shown_order_until_the_store_publishes_something_new() {
    let s = snap(six(), OrderState::Server, WriteState::Idle);
    let mut page = page_of(&s);
    pick_up(&mut page, &s, 2);
    press(&mut page, &s, Key::Up);
    ok_lib(&mut page, &s, 2);
    let tick = || ScreenEvent::Tick(plx_machine::machine::Tick { ms: 16, dt_us: 16_000 });
    step_page(&mut page, &s, tick());
    assert_eq!(order(&page), [0, 2, 1, 3, 4, 5], "the stale publication does not snap it back");
    let mut moved = six();
    moved.swap(1, 2);
    let after = snap(moved, RECORDED, WriteState::InFlight);
    step_page(&mut page, &after, tick());
    assert_eq!(order(&page), [0, 2, 1, 3, 4, 5], "the new publication agrees");
    assert!(page.state.seen.reset_offered(), "and Reset is offered now");
}

#[test]
fn a_publication_that_lands_while_a_library_is_held_waits_for_the_drop() {
    let s = snap(six(), OrderState::Server, WriteState::Idle);
    let mut page = page_of(&s);
    pick_up(&mut page, &s, 1);
    press(&mut page, &s, Key::Down);
    let other = snap(six().into_iter().rev().collect(), RECORDED, WriteState::Saved);
    step_page(&mut page, &other, ScreenEvent::Tick(plx_machine::machine::Tick { ms: 16, dt_us: 16_000 }));
    assert_eq!(order(&page), [0, 2, 1, 3, 4, 5], "the held order is not replaced under the viewer's hands");
}

// ---- the sentences ----------------------------------------------------------------------------

#[test]
fn every_mode_says_its_sentence() {
    use plx_os::i18n::msg;
    let s = snap(six(), RECORDED, WriteState::Saved);
    let mut page = page_of(&s);
    let say = |page: &LibraryOrderPage| page.state.description();
    focus_lib(&mut page, &s, 0);
    assert_eq!(say(&page), msg::library_order_state_idle(msg::browse_kind_movies(), "Movies"), "idle, on a library");
    focus_lib(&mut page, &s, 4);
    assert_eq!(say(&page), msg::library_order_state_idle(msg::browse_kind_tv_shows(), "Anime"));
    assert!(say(&page).starts_with("Press OK to move Anime"), "the title is the focused library's: {}", say(&page));
    focus_on(&mut page, &s, reset_elem_for_test());
    assert_eq!(say(&page), msg::library_order_state_reset(), "idle, on Reset");
    ok_on(&mut page, &s, reset_elem_for_test());
    assert_eq!(say(&page), msg::library_order_state_confirm());
    press(&mut page, &s, Key::Up);
    assert_eq!(page.state.mode, Mode::Idle);
    pick_up(&mut page, &s, 4);
    assert_eq!(say(&page), msg::library_order_state_moving("Anime"));
    for _ in 0..3 {
        press(&mut page, &s, Key::Up);
    }
    ok_lib(&mut page, &s, 4);
    // Anime now stands second of all six on Home and first among the TV shows in the header
    assert_eq!(order(&page), [0, 4, 1, 2, 3, 5]);
    assert_eq!(say(&page), msg::library_order_state_dropped("first", "second", msg::browse_kind_tv_shows(), "Anime"));
    // the arguments land in their own places (the generated function orders them by name)
    let said = say(&page);
    let (title_at, home_at, header_at) = (said.find("Anime").unwrap(), said.find("second").unwrap(), said.find("first").unwrap());
    assert!(title_at < home_at && home_at < header_at, "title, then its place on Home, then in the header: {said}");
    press(&mut page, &s, Key::Down);
    assert_eq!(page.state.mode, Mode::Idle, "the sentence lasts until the next key");
    focus_on(&mut page, &s, reset_elem_for_test());
    ok_on(&mut page, &s, reset_elem_for_test());
    ok_on(&mut page, &s, reset_elem_for_test());
    assert_eq!(say(&page), msg::library_order_state_reset_done());
}

#[test]
fn a_failed_write_says_so_in_words_and_never_in_the_middle_of_a_move() {
    use plx_os::i18n::msg;
    let s = snap(six(), RECORDED, WriteState::Failed);
    let mut page = page_of(&s);
    let idle = page.state.description();
    assert!(idle.ends_with(msg::library_order_state_unsaved()), "{idle}");
    pick_up(&mut page, &s, 0);
    assert_eq!(page.state.description(), msg::library_order_state_moving("Movies"));
    let ok = snap(six(), RECORDED, WriteState::Saved);
    assert!(!page_of(&ok).state.description().contains(msg::library_order_state_unsaved()));
}

// ---- the preview ------------------------------------------------------------------------------

fn movies(n: usize) -> Vec<LibraryOrderRow> {
    (0..n).map(|i| row(i, &format!("Movie library number {i}"), SecKind::Movie, "")).collect()
}

fn tick_ev() -> ScreenEvent<InnerHost> {
    ScreenEvent::Tick(plx_machine::machine::Tick { ms: 16, dt_us: 16_000 })
}

#[test]
fn the_preview_draws_a_more_only_when_names_are_folded_and_the_note_follows_the_same_answer() {
    for count in [1, 2, 3, 5, 12] {
        let s = snap(movies(count), OrderState::Server, WriteState::Idle);
        let mut page = page_of(&s);
        step_page(&mut page, &s, tick_ev());
        assert_eq!(page.preview.draws_more(), page.preview.overflowed(), "{count} libraries: the ⋯ and the note are one answer");
        assert!(page.preview.slot().is_some());
    }
    let few = snap(movies(2), OrderState::Server, WriteState::Idle);
    let mut two = page_of(&few);
    step_page(&mut two, &few, tick_ev());
    assert!(!two.preview.overflowed() && !two.preview.draws_more(), "names that fit draw no ⋯");
    assert_eq!(two.preview.shown_names().len(), 2);
    let many = snap(movies(12), OrderState::Server, WriteState::Idle);
    let mut twelve = page_of(&many);
    step_page(&mut twelve, &many, tick_ev());
    assert!(twelve.preview.overflowed() && twelve.preview.draws_more(), "twelve past the cap of eight always fold");
}

#[test]
fn the_preview_shows_the_kind_of_the_library_in_view_and_the_last_ones_with_reset_in_focus() {
    let s = snap(six(), RECORDED, WriteState::Saved);
    let mut page = page_of(&s);
    focus_lib(&mut page, &s, 2);
    step_page(&mut page, &s, tick_ev());
    assert_eq!(page.preview.shown_names().len(), 2, "the two Movies libraries");
    assert_eq!(page.preview.plated().as_deref(), Some("4K Movies"));
    focus_lib(&mut page, &s, 4);
    step_page(&mut page, &s, tick_ev());
    assert!(page.preview.shown_names().iter().any(|n| n == "Anime anna"), "the plated name is on the row: {:?}", page.preview.shown_names());
    assert!(page.preview.shown_names().iter().all(|n| n != "Movies"), "and no Movies library is in a TV shows header");
    assert_eq!(page.preview.plated().as_deref(), Some("Anime anna"), "an owner's credit travels with the name");
    focus_on(&mut page, &s, reset_elem_for_test());
    step_page(&mut page, &s, tick_ev());
    assert_eq!(page.preview.plated().as_deref(), Some("Documentaries"), "Reset: the last library's kind");
    focus_lib(&mut page, &s, 0);
    pick_up(&mut page, &s, 0);
    press(&mut page, &s, Key::Down);
    press(&mut page, &s, Key::Down);
    step_page(&mut page, &s, tick_ev());
    assert_eq!(page.preview.shown_names(), ["4K Movies", "Movies"], "the held library moves in the preview with it");
}

// ---- a library no order can name yet --------------------------------------------------------------

fn unnamed(rows: &mut [LibraryOrderRow], at: &[usize]) {
    for &i in at { rows[i].machine_id.clear(); }
}

fn description_of(page: &LibraryOrderPage) -> String { page.state.description() }

#[test]
fn an_unnamed_library_is_not_pickable_says_why_and_sends_nothing() {
    use plx_os::i18n::msg;
    let mut rows = six();
    unnamed(&mut rows, &[1]);
    let s = snap(rows, RECORDED, WriteState::Saved);
    let mut page = page_of(&s);
    focus_lib(&mut page, &s, 1);
    assert_eq!(description_of(&page), msg::library_order_state_unnamed("TV Shows"), "the sentence says why, not 'Press OK to move'");
    assert!(sent(&ok_lib(&mut page, &s, 1)).is_empty());
    assert_eq!(page.state.mode, Mode::Idle, "OK on it picks nothing up");
    let row = &page.form.table.sections[0].rows[1];
    assert!(row.disabled && row.moving == RowMove::Off, "a disabled row that offers no move");
    // a named neighbour still moves, and the unnamed one is never the one held
    pick_up(&mut page, &s, 0);
    assert!(matches!(&page.state.mode, Mode::Moving { lifted, .. } if *lifted == lib(&page, 0)));
}

#[test]
fn a_page_of_only_unnamed_libraries_moves_nothing_sends_nothing_and_still_has_a_stop() {
    let mut rows = six();
    unnamed(&mut rows, &[0, 1, 2, 3, 4, 5]);
    assert!(order_command(&rows).is_none(), "no command names no library");
    let s = snap(rows, OrderState::Server, WriteState::Idle);
    let mut page = page_of(&s);
    assert_eq!(page.form.focusable_len(), 6, "every library row is a stop (to read why); Reset is inert");
    for section in 0..6 {
        focus_lib(&mut page, &s, section);
        assert!(sent(&ok_lib(&mut page, &s, section)).is_empty());
        assert_eq!(page.state.mode, Mode::Idle, "section {section}");
    }
    assert!(!matches!(page.state.mode, Mode::Moving { .. }));
    assert!(page.form.selected_id().is_some(), "focus has a row to be on");
}

#[test]
fn a_drop_never_sends_an_empty_command_and_says_a_place_only_when_it_sent_one() {
    use plx_os::i18n::msg;
    let s = snap(six(), OrderState::Server, WriteState::Idle);
    let mut page = page_of(&s);
    pick_up(&mut page, &s, 1);
    press(&mut page, &s, Key::Down);
    // the shown rows go unnamed under the hand (a publication cannot land while held, so force the state)
    for row in &mut page.state.shown { row.machine_id.clear(); }
    assert!(sent(&ok_lib(&mut page, &s, 1)).is_empty(), "no library an order can name: nothing sent");
    assert_eq!(page.state.mode, Mode::Idle, "and no place is claimed");
    assert_ne!(page.state.description(), msg::library_order_state_reset_done());
}

// ---- a library is its machine id and key, not its table index -------------------------------------

/// The same publication after the section table was rebuilt: every `section` index moved.
fn renumbered(rows: &[LibraryOrderRow], by: usize) -> Vec<LibraryOrderRow> {
    rows.iter().cloned().map(|mut row| { row.section += by; row }).collect()
}

#[test]
fn a_table_rebuild_after_a_drop_keeps_the_sentence_and_the_focus_on_the_right_library() {
    use plx_os::i18n::msg;
    let s = snap(six(), OrderState::Server, WriteState::Idle);
    let mut page = page_of(&s);
    pick_up(&mut page, &s, 4);
    for _ in 0..3 { press(&mut page, &s, Key::Up); }
    ok_lib(&mut page, &s, 4);
    let said = msg::library_order_state_dropped("first", "second", msg::browse_kind_tv_shows(), "Anime");
    assert_eq!(page.state.description(), said);
    // the store echoes the order, from a table rebuilt under new indices
    let mut echoed = six();
    let anime = echoed.remove(4);
    echoed.insert(1, anime);
    let rebuilt = snap(renumbered(&echoed, 10), RECORDED, WriteState::InFlight);
    step_page(&mut page, &rebuilt, tick_ev());
    assert_eq!(page.state.basis, Shown::Published, "the echo ended the wait");
    assert_eq!(page.state.description(), said, "still Anime, still second: the mode follows the library, not index 4");
    assert!(matches!(&page.state.mode, Mode::Dropped(lib) if lib.key == 5));
    assert_eq!(page.state.previewed().map(|row| row.title.as_str()), Some("Anime"));
    // focus on a library survives a rebuild by identity too
    let mut page = page_of(&s);
    focus_lib(&mut page, &s, 3);
    let moved = snap(renumbered(&six(), 7), RECORDED, WriteState::Saved);
    step_page(&mut page, &moved, tick_ev());
    assert_eq!(page.form.selected_id().map(|id| matches!(id, LibId::Lib(lib) if lib.key == 4)), Some(true), "Kids is still focused");
    assert_eq!(page.state.sel, LibId::Lib(LibraryRef { machine_id: "m0".into(), key: 4 }));
}

#[test]
fn a_held_library_keeps_its_row_key_through_every_move() {
    let s = snap(six(), OrderState::Server, WriteState::Idle);
    let mut page = page_of(&s);
    let key = page.elem_of(3);
    pick_up(&mut page, &s, 3);
    for _ in 0..2 {
        press(&mut page, &s, Key::Up);
        assert_eq!(page.form.selected_key(), Some(RowKey(key)), "the engine's key still names the held row");
        assert_eq!(page.elem_of(3), key);
    }
}

// ---- pointer focus and the page's invariants ----------------------------------------------------------

fn focus_key(page: &mut LibraryOrderPage, snap: &DirectorySnapshot, elem: u32) {
    focus_on(page, snap, elem);
}

#[test]
fn a_pointer_over_another_row_while_a_library_is_held_changes_nothing() {
    let s = snap(six(), OrderState::Server, WriteState::Idle);
    let mut page = page_of(&s);
    pick_up(&mut page, &s, 2);
    let held = page.elem_of(2);
    let other = page.elem_of(4);
    assert!(!page.form.lands_on(RowKey(other)), "the table says no row but the held one is a stop");
    assert_eq!(plx_ui::geom::table_elems(&page.form.table, Some(&page.form)).count(), 1);
    focus_key(&mut page, &s, other);
    assert!(matches!(page.state.mode, Mode::Moving { .. }), "still holding");
    assert_eq!(page.form.selected_key(), Some(RowKey(held)), "the selection did not leave the held row");
    assert_eq!(page.state.sel, LibId::of(&page.state.shown[2]));
    // the band is not a row: a pointer there puts the library back, as Back does, and writes nothing
    let entry = page.entry;
    let (_, out) = step_page(&mut page, &s, ScreenEvent::FocusMoved {
        from: None, to: FocusKey { entry, elem: crate::registry::BAND }, by: plx_ui::screen::By::Dir,
    });
    assert_eq!(page.state.mode, Mode::Idle);
    assert!(sent(&out).is_empty());
    assert_eq!(order(&page), [0, 1, 2, 3, 4, 5]);
}

#[test]
fn a_pointer_over_a_library_while_a_reset_is_asked_withdraws_the_question() {
    use plx_os::i18n::msg;
    let s = snap(six(), RECORDED, WriteState::Saved);
    let mut page = page_of(&s);
    focus_on(&mut page, &s, reset_elem_for_test());
    ok_on(&mut page, &s, reset_elem_for_test());
    assert_eq!(page.state.mode, Mode::ConfirmReset);
    focus_on(&mut page, &s, reset_elem_for_test());
    assert_eq!(page.state.mode, Mode::ConfirmReset, "focus staying on Reset keeps the question");
    let elem = page.elem_of(1);
    focus_on(&mut page, &s, elem);
    assert_eq!(page.state.mode, Mode::Idle, "focus left Reset: the question is withdrawn");
    assert_ne!(page.state.description(), msg::library_order_state_confirm());
    // and a click that follows is a pick-up, not the confirmation
    assert!(sent(&ok_on(&mut page, &s, elem)).is_empty());
    assert!(matches!(page.state.mode, Mode::Moving { .. }));
}

#[test]
fn a_reset_that_turns_inert_under_a_question_withdraws_it() {
    let recorded = snap(six(), RECORDED, WriteState::Saved);
    let mut page = page_of(&recorded);
    focus_on(&mut page, &recorded, reset_elem_for_test());
    ok_on(&mut page, &recorded, reset_elem_for_test());
    assert_eq!(page.state.mode, Mode::ConfirmReset);
    // the record went away elsewhere (a sign-out re-read, another reset): Reset is inert now
    let gone = snap(six(), OrderState::Server, WriteState::Saved);
    step_page(&mut page, &gone, tick_ev());
    assert_eq!(page.state.mode, Mode::Idle, "no question about an action that is gone");
    assert!(sent(&ok_on(&mut page, &gone, reset_elem_for_test())).is_empty(), "and OK on the inert row does nothing");
}

// ---- awaiting the store's echo -------------------------------------------------------------------------

#[test]
fn a_second_quick_drop_does_not_snap_back_to_the_first_echo() {
    let s = snap(six(), OrderState::Server, WriteState::Idle);
    let mut page = page_of(&s);
    // drop 1: 4K Movies (2) up one
    pick_up(&mut page, &s, 2);
    press(&mut page, &s, Key::Up);
    assert_eq!(sent(&ok_lib(&mut page, &s, 2)).len(), 1);
    assert!(matches!(page.state.basis, Shown::AwaitingEcho { .. }));
    // pick up again at once: Kids (3) up one
    pick_up(&mut page, &s, 3);
    press(&mut page, &s, Key::Up);
    // drop 1's save lands while Kids is held (the page does not take it in)
    let mut first = six();
    first.swap(1, 2);
    let echo1 = snap(first, RECORDED, WriteState::InFlight);
    step_page(&mut page, &echo1, tick_ev());
    assert_eq!(order(&page), [0, 2, 3, 1, 4, 5], "held: untouched");
    // drop 2
    assert_eq!(sent(&ok_lib(&mut page, &echo1, 3)).len(), 1);
    // a stale tick before the store steps: the publication still says drop 1's order
    step_page(&mut page, &echo1, tick_ev());
    assert_eq!(order(&page), [0, 2, 3, 1, 4, 5], "no snap back to drop 1's order");
    assert!(!page.form.table.gliding(), "and nothing glided back and forth");
    // the store's echo of drop 2
    let mut second = six();
    second.swap(1, 2);
    second.swap(2, 3);
    let echo2 = snap(second, RECORDED, WriteState::InFlight);
    step_page(&mut page, &echo2, tick_ev());
    assert_eq!(order(&page), [0, 2, 3, 1, 4, 5]);
    assert_eq!(page.state.basis, Shown::Published, "the wait ended on the echo of the order that was sent");
}

#[test]
fn a_dropped_order_the_store_never_echoes_is_given_up_on_by_the_tick_clock() {
    let s = snap(six(), OrderState::Server, WriteState::Idle);
    let mut page = page_of(&s);
    pick_up(&mut page, &s, 2);
    press(&mut page, &s, Key::Up);
    assert_eq!(sent(&ok_lib(&mut page, &s, 2)).len(), 1);
    let at = |ms: u32| ScreenEvent::<InnerHost>::Tick(plx_machine::machine::Tick { ms, dt_us: 16_000 });
    // the store refused the command: it keeps publishing the old order
    step_page(&mut page, &s, at(10_000));
    assert_eq!(order(&page), [0, 2, 1, 3, 4, 5], "the page keeps its own rows while it waits");
    step_page(&mut page, &s, at(12_999));
    assert_eq!(order(&page), [0, 2, 1, 3, 4, 5], "still inside the patience, counted from the first tick after the drop");
    step_page(&mut page, &s, at(13_000));
    assert_eq!(order(&page), [0, 1, 2, 3, 4, 5], "past it the page shows what the store says");
    assert_eq!(page.state.basis, Shown::Published);
    assert_eq!(page.state.mode, Mode::Idle, "a refused order has no place to announce: the sentence is the focused row's");
    assert!(!page.state.description().is_empty());
}

#[test]
fn a_dropped_library_that_leaves_the_publication_ends_its_sentence_without_a_stray_space() {
    let s = snap(six(), OrderState::Server, WriteState::Idle);
    let mut page = page_of(&s);
    pick_up(&mut page, &s, 2);
    press(&mut page, &s, Key::Up);
    ok_lib(&mut page, &s, 2);
    let mut echoed = six();
    echoed.swap(1, 2);
    step_page(&mut page, &snap(echoed.clone(), RECORDED, WriteState::InFlight), tick_ev());
    assert!(matches!(page.state.mode, Mode::Dropped(_)), "the echo landed: the place is announced");
    // the library is gone from the next publication (its server left), and the write failed
    echoed.retain(|row| row.section != 2);
    step_page(&mut page, &snap(echoed, RECORDED, WriteState::Failed), tick_ev());
    assert_eq!(page.state.mode, Mode::Idle);
    let said = page.state.description();
    assert!(!said.starts_with(' ') && !said.is_empty(), "{said:?}");
}

#[test]
fn a_pointer_over_the_band_while_a_library_is_held_leaves_the_band_focused() {
    let s = snap(six(), OrderState::Server, WriteState::Idle);
    let mut page = page_of(&s);
    pick_up(&mut page, &s, 2);
    let entry = page.entry;
    step_page(&mut page, &s, ScreenEvent::FocusMoved { from: None, to: FocusKey { entry, elem: crate::registry::BAND }, by: plx_ui::screen::By::Dir });
    assert_eq!(page.state.mode, Mode::Idle, "the held library went back");
    assert!(!page.form.table.list_focused, "and the rebuild did not hand focus back to the list");
}

#[test]
fn while_an_order_waits_for_its_echo_the_record_and_write_state_still_follow_the_store() {
    let s = snap(six(), OrderState::Server, WriteState::Idle);
    let mut page = page_of(&s);
    pick_up(&mut page, &s, 2);
    press(&mut page, &s, Key::Up);
    ok_lib(&mut page, &s, 2);
    assert!(!reset_stops(&page), "no record yet");
    // the record state landed before the rows did (the order is the stale one)
    let partly = snap(six(), RECORDED, WriteState::Failed);
    step_page(&mut page, &partly, tick_ev());
    assert_eq!(order(&page), [0, 2, 1, 3, 4, 5], "rows hold");
    assert!(reset_stops(&page), "Reset is offered the moment the store says there is a record");
    assert!(page.state.seen.write_failed());
}

#[test]
fn a_reset_supersedes_an_order_waiting_for_its_echo() {
    let s = snap(six(), RECORDED, WriteState::Saved);
    let mut page = page_of(&s);
    pick_up(&mut page, &s, 2);
    press(&mut page, &s, Key::Up);
    ok_lib(&mut page, &s, 2);
    press(&mut page, &s, Key::Down);
    focus_on(&mut page, &s, reset_elem_for_test());
    ok_on(&mut page, &s, reset_elem_for_test());
    assert_eq!(sent(&ok_on(&mut page, &s, reset_elem_for_test())), [Sent::Reset]);
    assert_eq!(page.state.basis, Shown::Published);
}

// ---- the root row -----------------------------------------------------------------------------

#[test]
fn the_root_row_reads_server_order_unless_a_saved_order_is_in_effect() {
    use plx_os::i18n::msg;
    let rows = || { let mut r = six(); r.swap(0, 4); r };
    let value = |state, rows: Vec<LibraryOrderRow>| root_value(snap(rows, state, WriteState::Saved).view());
    assert_eq!(value(OrderState::Server, six()), msg::library_order_value_server());
    assert_eq!(value(OrderState::Recorded { in_effect: false }, six()), msg::library_order_value_server(), "saved but not in effect");
    assert_eq!(value(RECORDED, rows()), msg::library_order_value_first("Anime"));
    assert_eq!(root_value(DirectorySnapshot::default().view()), msg::library_order_value_server(), "no libraries yet");
}

#[test]
fn the_root_lists_the_row_between_favorite_libraries_and_under_posters_and_opens_the_page() {
    let _g = plx_base::testlock::serial();
    let mut form = FormTable::<RootId, Action, SettingsPage>::page(super::super::super::registry::BAND);
    form.set(root_form(&super::super::text_fit_tests::base_root_inputs()), None);
    let at = |id: RootId| form.index_of(&id).expect("on the signed-in root");
    assert_eq!(at(RootId::LibraryOrder), at(RootId::Favourites) + 1);
    assert_eq!(at(RootId::UnderPosters), at(RootId::LibraryOrder) + 1);
    assert_eq!(form.activate(at(RootId::LibraryOrder)).map(|a| matches!(a, plx_ui::form::Activation::Push(SettingsPage::LibraryOrder))), Some(true));
    assert_eq!(sentence_for(Some(&RootId::LibraryOrder)), plx_os::i18n::msg::library_order_row_detail());
}

// ---- every string fits ------------------------------------------------------------------------

#[test]
fn every_library_order_string_fits_in_every_language() {
    let mut out = Vec::new();
    let long = || {
        let mut rows = six();
        rows[0].title = "Documentaries and concerts".into();
        rows
    };
    for language in SHIPPED {
        let _guard = language_on_this_thread_for_test(language);
        let tag = language.tag();
        for (state, name) in [(OrderState::Server, "server"), (RECORDED, "recorded")] {
            let s = snap(long(), state, WriteState::Saved);
            let page = page_of(&s);
            super::super::text_fit_tests::overflowing(&format!("{tag} library order ({name})"), &page.form.table, &mut out);
        }
        let value = root_value(snap(long(), RECORDED, WriteState::Saved).view());
        assert!(!value.is_empty(), "{tag}");
        let unnamed = plx_os::i18n::msg::library_order_state_unnamed("Documentaries and concerts");
        for text in [
            plx_os::i18n::msg::library_order_state_confirm(), plx_os::i18n::msg::library_order_state_reset_done(),
            plx_os::i18n::msg::library_order_preview_more(), plx_os::i18n::msg::library_order_state_unsaved(),
            unnamed.as_str(),
        ] {
            assert!(!text.contains('\u{22ef}'), "{tag}: the ⋯ glyph is not in the shipped faces: {text}");
        }
    }
    assert!(out.is_empty(), "{out:#?}");
}
