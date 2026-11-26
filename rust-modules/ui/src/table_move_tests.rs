//! Unit tests for the shared table's reorder surface: the inert row (`Row::inert`,
//! `FormSection::inert`), the per-row move state (`RowMove`) and the row glide
//! (`TableView::glide_from`, seeded by `FormTable` on a reorder).

use super::form::*;
use crate::table::{Row, RowMove, Section, TableView, ROW_H};
use crate::{draw_census, Painter, Rect};
use crate::widgets::LegacyMeasure;

#[derive(Clone, Debug, PartialEq)]
enum Id {
    A,
    B,
    C,
    D,
}
impl FormId for Id {
    fn key(&self) -> RowKey {
        RowKey(match self {
            Id::A => 1,
            Id::B => 2,
            Id::C => 3,
            Id::D => 4,
        })
    }
}
type T = FormTable<Id, (), ()>;
type S = FormSection<Id, (), ()>;
type F = Form<Id, (), ()>;

const FRAME: Rect = Rect { x: 0.0, y: 0.0, w: 700.0, h: 600.0 };
const DT: f32 = 1.0 / 60.0;

fn row(label: &str) -> Row {
    Row::new(label)
}

/// A, B, C, D in the given order, with `inert_b` making B an inert slot.
fn form(order: &[Id], inert_b: bool) -> F {
    let mut s = S::new("Libraries");
    for id in order {
        let label = format!("{id:?}");
        s = s.item(id.clone(), RowKind::Button, (), row(&label)).inert(inert_b && *id == Id::B);
    }
    F::new().section(s)
}

fn tops(t: &T) -> Vec<f32> {
    t.table.rest_tops()
}

// ---- the inert row ----

#[test]
fn focus_skips_an_inert_row_in_both_directions_and_from_the_edges() {
    let mut t = T::new(100);
    t.open(form(&[Id::A, Id::B, Id::C], true), Some(&Id::A));
    assert_eq!(t.focusable_len(), 2, "the inert slot is no stop");
    assert_eq!(t.step_key(RowKey(1), 1), Some(RowKey(3)), "down steps over it");
    assert_eq!(t.step_key(RowKey(3), -1), Some(RowKey(1)), "and up");
    assert_eq!(t.table.next_selectable(1, 0), None, "it is not selectable itself");
    // the edges: an inert FIRST row is never where focus opens, nor LAST where it ends
    let mut e = T::new(100);
    e.open(form(&[Id::B, Id::A, Id::C], true), None);
    assert_eq!(e.selected_id(), Some(&Id::A), "a menu opens past an inert first row");
    let mut e = T::new(100);
    e.open(form(&[Id::A, Id::C, Id::B], true), Some(&Id::C));
    assert_eq!(e.table.last_row(), Some(1), "the last SELECTABLE row skips an inert tail");
    assert_eq!(e.step_key(RowKey(3), 1), None, "no stop past the end");
    // the selection settles off it, whoever set it
    e.table.sel = 2;
    e.table.move_sel(-1);
    assert_eq!(e.table.sel, 0, "settled onto C, then one up");
    assert_eq!(e.table.settle(2), 1);
    // a pointer cannot land on it either
    let b = e.table.row_frame(FRAME, 0).expect("A has a frame");
    assert!(e.table.hit_row(FRAME, b.x + 5.0, b.y + ROW_H * 2.0 + 5.0).is_none_or(|i| i != 2));
}

#[test]
fn an_inert_row_is_still_drawn_and_drawn_dim() {
    let mut t = T::new(100);
    t.open(form(&[Id::A, Id::B, Id::C], true), Some(&Id::A));
    let runs = draw_census::capture_runs(|| t.table.draw(Painter::recording(), FRAME, &LegacyMeasure));
    let alpha = |label: &str| runs.iter().find(|r| r.text == label).map(|r| r.ink[3]).unwrap_or_else(|| panic!("{label} not drawn"));
    assert!(alpha("B") > 0.0, "it is on screen");
    assert!(alpha("B") < alpha("C"), "but dimmer than a live row: {} vs {}", alpha("B"), alpha("C"));
    assert!((alpha("B") / alpha("C") - 0.45).abs() < 0.01, "at the inert weight");
}

#[test]
fn a_disabled_row_is_still_focusable_as_before() {
    let mut t = T::new(100);
    t.open(
        F::new().section(
            S::new("S")
                .item(Id::A, RowKind::Button, (), row("A"))
                .item(Id::B, RowKind::Button, (), row("B"))
                .disabled(true)
                .item(Id::C, RowKind::Button, (), row("C")),
        ),
        Some(&Id::A),
    );
    assert_eq!(t.step_key(RowKey(1), 1), Some(RowKey(2)), "focus still lands on a disabled row");
    assert!(t.activate(1).is_none(), "and it still does nothing");
}

// ---- the move states ----

fn reorder_table(states: [RowMove; 3]) -> TableView {
    let mut t = TableView::new();
    let mut s = Section::new("Libraries");
    for (i, m) in states.into_iter().enumerate() {
        s = s.row(row(["A", "B", "C"][i]).moving(m));
    }
    t.set_sections(vec![s], 1, false);
    t
}

/// Step `t` until its lift has landed (a lifted selection at 1, an unlifted one at 0).
fn settle_lift(t: &mut TableView) {
    for _ in 0..600 {
        t.update(DT, 600.0);
    }
}

/// B's text size and y as the table draws them now.
fn b_face(t: &TableView) -> (f32, i32) {
    let runs = draw_census::capture_runs(|| t.draw(Painter::recording(), FRAME, &LegacyMeasure));
    runs.iter().find(|r| r.text == "B").map(|r| (r.at.y, r.size)).unwrap()
}

#[test]
fn chevrons_follow_the_state_alone() {
    let pick = RowMove::Pickable { can_up: true, can_down: false };
    assert_eq!(pick.chevrons(false), None, "an unfocused pickable row shows nothing");
    assert_eq!(pick.chevrons(true), Some((0.45, 0.45 * 0.25)), "focused: the dim pair, the wall end quieter");
    let lifted = RowMove::Lifted { can_up: false, can_down: true };
    assert_eq!(lifted.chevrons(true), Some((0.25, 1.0)), "lifted: full chevrons, the end one at .25");
    assert_eq!(lifted.chevrons(false), lifted.chevrons(true), "a lifted row needs no focus to say so");
    assert_eq!(RowMove::Off.chevrons(true), None);
    assert_eq!(RowMove::Waiting.chevrons(true), None);
}

#[test]
fn a_focused_pickable_row_draws_the_chevron_pair_in_the_check_column() {
    let t = reorder_table([RowMove::Pickable { can_up: false, can_down: true }; 3]);
    let icons = draw_census::capture_icons(|| t.draw(Painter::recording(), FRAME, &LegacyMeasure));
    let icon = crate::icons::Icon::ChevronUp;
    assert_eq!(icons.iter().filter(|(i, _)| *i == icon).count(), 1, "only the focused row: {icons:?}");
    let (_, up) = icons.iter().find(|(i, _)| *i == icon).unwrap();
    let (_, down) = icons.iter().find(|(i, _)| *i == crate::icons::Icon::ChevronDown).unwrap();
    assert!(up.y + up.h <= down.y + 0.5, "up above down, one box each");
    assert!(up.x < crate::table::CONTENT_X + 32.0, "in the check column");
}

#[test]
fn a_lifted_row_grows_rises_casts_a_shadow_and_is_drawn_last() {
    let off = reorder_table([RowMove::Off; 3]);
    let mut lifted = reorder_table([RowMove::Waiting, RowMove::Lifted { can_up: true, can_down: true }, RowMove::Waiting]);
    settle_lift(&mut lifted);
    let still = draw_census::capture_runs(|| off.draw(Painter::recording(), FRAME, &LegacyMeasure));
    let runs = draw_census::capture_runs(|| lifted.draw(Painter::recording(), FRAME, &LegacyMeasure));
    let at = |runs: &[draw_census::Run], l: &str| runs.iter().find(|r| r.text == l).map(|r| (r.at.y, r.size)).unwrap();
    assert!(at(&runs, "B").0 < at(&still, "B").0, "it rises");
    assert_eq!(at(&runs, "A"), at(&still, "A"), "the others stay put");
    assert!(runs.iter().position(|r| r.text == "B") > runs.iter().position(|r| r.text == "C"), "and it is drawn after the row below it");
    let inks = draw_census::capture_inks(|| lifted.draw(Painter::recording(), FRAME, &LegacyMeasure));
    assert!(inks.iter().any(|(tag, _, a)| *tag == 5 && *a > 0.0), "the focus shadow is cast: {inks:?}");
    let pills: Vec<_> = inks.iter().filter(|(tag, ..)| *tag == 2).collect();
    assert_eq!(pills.len(), 1, "the lifted row wears its own pill, not the shared one");
    assert!(pills[0].1.w > FRAME.w - 24.0, "grown by the lift");
}

#[test]
fn a_waiting_row_reads_in_secondary_ink() {
    let t = reorder_table([RowMove::Waiting, RowMove::Lifted { can_up: true, can_down: true }, RowMove::Waiting]);
    let live = reorder_table([RowMove::Off; 3]);
    let ink = |t: &TableView| draw_census::capture_runs(|| t.draw(Painter::recording(), FRAME, &LegacyMeasure)).into_iter().find(|r| r.text == "A").unwrap().ink;
    assert_eq!(ink(&t), crate::theme::TEXT_SECONDARY);
    assert_ne!(ink(&live), crate::theme::TEXT_SECONDARY);
}

// ---- glide ----

#[test]
fn a_reorder_glides_each_moved_row_from_where_it_was_and_settles_to_zero() {
    let mut t = T::new(100);
    t.open(form(&[Id::A, Id::B, Id::C, Id::D], false), Some(&Id::B));
    let before = tops(&t);
    // B moves down one place: A, C, B, D
    t.refresh(form(&[Id::A, Id::C, Id::B, Id::D], false));
    let after = tops(&t);
    assert_eq!(after, before, "the layout is the same four slots");
    let drawn = t.table.drawn_tops();
    assert_eq!(drawn[0], after[0], "A did not move");
    assert_eq!(drawn[1], before[2], "C is drawn where B was (it was at slot 2 and now stands in 1)");
    assert_eq!(drawn[2], before[1], "B is drawn where C was");
    // it settles
    let mut frames = 0;
    while t.table.drawn_tops() != after && frames < 600 {
        t.table.update(DT, 600.0);
        frames += 1;
    }
    assert!(frames > 1 && frames < 600, "it moves over frames, then lands ({frames})");
    assert_eq!(t.table.drawn_tops(), after);
    assert!(!t.table.gliding(), "a settled table holds no glide, so its motion reads as one that never glided");
}

#[test]
fn a_reinstall_that_keeps_the_order_cuts_as_before() {
    let mut t = T::new(100);
    t.open(form(&[Id::A, Id::B, Id::C], false), Some(&Id::A));
    t.refresh(form(&[Id::A, Id::C], false)); // a row left; no row changed place relative to another
    t.set(form(&[Id::A, Id::D, Id::C], false), Some(&Id::A)); // and one arrived
    assert_eq!(t.table.drawn_tops(), t.table.rest_tops(), "no glide without a reorder");
}

#[test]
fn glide_state_is_part_of_the_canonical_motion() {
    let hash = |t: &TableView| {
        let mut c = plx_machine::machine::Canon::new();
        t.write_motion(&mut c);
        c.finish()
    };
    let mut t = TableView::new();
    t.set_sections(vec![Section::new("S").row(row("A")).row(row("B"))], 0, false);
    let base = hash(&t);
    t.glide_from(&[Some(ROW_H), Some(0.0)]);
    let seeded = hash(&t);
    assert_ne!(seeded, base, "a seeded glide is canonical state");
    assert_eq!(seeded, hash(&t), "reading it does not advance it");
    t.update(DT, 600.0);
    assert_ne!(hash(&t), seeded, "stepping moves it");
}

// ---- the lift spring ----

#[test]
fn the_lift_eases_in_on_pickup_and_out_on_drop_on_one_spring() {
    let rest = b_face(&reorder_table([RowMove::Off; 3]));
    let mut t = reorder_table([RowMove::Waiting, RowMove::Lifted { can_up: true, can_down: true }, RowMove::Waiting]);
    assert_eq!(b_face(&t), rest, "the frame the page lifts the row it has not grown yet: the spring starts at 0");
    t.update(DT * 4.0, 600.0);
    let mid = b_face(&t);
    assert!(mid.0 < rest.0, "a few frames in it has begun to rise and grow: {mid:?} vs {rest:?}");
    settle_lift(&mut t);
    let up = b_face(&t);
    assert!(up.0 < mid.0 && up.1 >= mid.1, "and it lands higher and larger: {up:?}");
    // the drop: the page hands the row back as an ordinary one; it settles down instead of cutting
    t.set_sections(vec![Section::new("Libraries")
        .row(row("A").moving(RowMove::Pickable { can_up: true, can_down: true }))
        .row(row("B").moving(RowMove::Pickable { can_up: true, can_down: true }))
        .row(row("C").moving(RowMove::Pickable { can_up: true, can_down: true }))], 1, false);
    assert_eq!(b_face(&t), up, "the frame after the drop still shows the lifted row: it has not cut");
    t.update(DT * 4.0, 600.0);
    let down = b_face(&t);
    assert!(down.0 > up.0 && down.1 <= up.1, "it is coming back down: {down:?}");
    settle_lift(&mut t);
    assert_eq!(b_face(&t), rest, "and lands exactly at rest");
}

#[test]
fn a_landed_drop_leaves_the_motion_as_a_table_that_never_lifted() {
    let hash = |t: &TableView| {
        let mut c = plx_machine::machine::Canon::new();
        t.write_motion(&mut c);
        c.finish()
    };
    let mut t = reorder_table([RowMove::Off; 3]);
    settle_lift(&mut t);
    let base = hash(&t);
    t.set_sections(vec![Section::new("Libraries").row(row("A")).row(row("B").moving(RowMove::Lifted { can_up: true, can_down: true })).row(row("C"))], 1, false);
    t.update(DT * 4.0, 600.0);
    assert_ne!(hash(&t), base, "a lift in flight is canonical state");
    t.set_sections(vec![Section::new("Libraries").row(row("A")).row(row("B")).row(row("C"))], 1, false);
    settle_lift(&mut t);
    assert_eq!(hash(&t), base, "once landed it is gone");
}

// ---- the lift belongs to a row, not to a slot ----

/// A, B, C, D in `order`: every row Pickable with nothing held, or with `held` lifted and the rest Waiting.
fn move_form(order: &[Id], held: Option<Id>) -> F {
    let mut s = S::new("Libraries");
    for id in order {
        let state = match &held {
            Some(h) if h == id => RowMove::Lifted { can_up: true, can_down: true },
            Some(_) => RowMove::Waiting,
            None => RowMove::Pickable { can_up: true, can_down: true },
        };
        s = s.item(id.clone(), RowKind::Button, (), row(&format!("{id:?}")).moving(state));
    }
    F::new().section(s)
}

fn at_of(t: &T, id: &Id) -> i32 {
    t.index_of(id).expect("present") as i32
}

#[test]
fn a_lift_that_is_landing_stays_with_the_row_that_was_held_through_a_reorder() {
    // pick B up, carry it down twice, then put it back (Back): the order returns to A B C D
    let (abcd, acdb) = ([Id::A, Id::B, Id::C, Id::D], [Id::A, Id::C, Id::D, Id::B]);
    let mut t = T::new(100);
    t.open(move_form(&abcd, None), Some(&Id::B));
    t.set_sliding(move_form(&abcd, Some(Id::B)), Some(&Id::B));
    settle_lift(&mut t.table);
    t.set_sliding(move_form(&acdb, Some(Id::B)), Some(&Id::B));
    t.table.update(DT, 600.0);
    assert_eq!(t.table.drawn_lifted(), [at_of(&t, &Id::B)], "B is the one lifted, wherever it stands");
    t.set_sliding(move_form(&abcd, None), Some(&Id::B));
    t.table.update(DT, 600.0);
    assert_eq!(t.table.drawn_lifted(), [at_of(&t, &Id::B)], "the landing lift is B's, not whoever slid into B's old slot (D)");
    settle_lift(&mut t.table);
    assert!(t.table.drawn_lifted().is_empty(), "and it lands");
}

#[test]
fn a_lift_whose_row_left_the_form_is_dropped() {
    let mut t = T::new(100);
    t.open(move_form(&[Id::A, Id::B, Id::C], None), Some(&Id::B));
    t.set_sliding(move_form(&[Id::A, Id::B, Id::C], Some(Id::B)), Some(&Id::B));
    settle_lift(&mut t.table);
    t.set_sliding(move_form(&[Id::A, Id::C], None), Some(&Id::A));
    t.table.update(DT, 600.0);
    assert!(t.table.drawn_lifted().is_empty(), "the held row is gone: nothing else is drawn lifted");
}

// ---- the rows that wait are no stop ----

#[test]
fn while_one_row_is_held_the_others_are_no_stop_for_a_key_a_pointer_or_an_element_walk() {
    let mut t = T::new(100);
    t.open(move_form(&[Id::A, Id::B, Id::C, Id::D], None), Some(&Id::B));
    assert_eq!(crate::geom::table_elems(&t.table, Some(&t)).count(), 4, "idle: every row is an element");
    t.set_sliding(move_form(&[Id::A, Id::B, Id::C, Id::D], Some(Id::B)), Some(&Id::B));
    assert_eq!(crate::geom::table_elems(&t.table, Some(&t)).collect::<Vec<_>>(), [1], "only the held row is");
    assert_eq!(t.table.next_selectable(1, 1), None, "a key has nowhere to go");
    assert_eq!(t.table.next_selectable(1, -1), None);
    let hits: std::collections::BTreeSet<i32> = (0..FRAME.h as i32).step_by(3).filter_map(|y| t.table.hit_row(FRAME, FRAME.x + 30.0, y as f32)).collect();
    assert_eq!(hits, [1].into(), "a pointer anywhere on the table lands on the held row or nothing");
    // drawn at its own weight, not dimmed as an inert row is
    let runs = draw_census::capture_runs(|| t.table.draw(Painter::recording(), FRAME, &LegacyMeasure));
    let alpha = |label: &str| runs.iter().find(|r| r.text == label).map(|r| r.ink[3]).unwrap();
    assert!(alpha("A") > 0.5 * alpha("B"), "a waiting row is not drawn at the inert weight: {} vs {}", alpha("A"), alpha("B"));
    t.set_sliding(move_form(&[Id::A, Id::B, Id::C, Id::D], None), Some(&Id::B));
    assert_eq!(crate::geom::table_elems(&t.table, Some(&t)).count(), 4, "put down: every row is a stop again");
}
