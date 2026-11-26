//! Number entry's panel, graded through its events the way the dispatcher drives it (Live TV PR5; handoff SPEC-5): what it
//! lists, what it seats, what it reports to its host and when, and the window of five rows.
use super::*;
use plx_data::livetv::{LiveTvStatus, LiveTvView};
use plx_machine::machine::{
    Chrome, FocusRead, Host, InputEvent, InputOwner, Measure, PressRead, ScreenId, Source, Stamped, Tick,
};
use plx_ui::fixture::FixtureMeasure;
use plx_ui::screen::ScreenArg;

#[derive(Clone, PartialEq, Eq)]
struct TestArg;

impl LogicalState for TestArg {
    fn write(&self, c: &mut Canon) {
        c.u32(0);
    }
    fn probe(&self, _: &mut String) {}
}

impl ScreenArg for TestArg {
    fn chrome(&self) -> Chrome {
        Chrome::None
    }
    fn id(&self) -> ScreenId {
        ScreenId(803)
    }
    fn title(&self) -> Option<&str> {
        None
    }
    fn same_instance(&self, other: &Self) -> bool {
        self == other
    }
}

struct TestHost;

impl Host for TestHost {
    type Arg = TestArg;
    type Fx = crate::registry::AppFx;
    type Msg = AppMsg;
    type Elem = u32;
    type Views<'a> = LiveTvView<'a>;
    type Init = crate::registry::NoInit;
    type Memory = PageMemory;
}

impl LiveTvLike for TestHost {
    fn livetv<'a>(cx: &Cx<'a, Self>) -> LiveTvView<'a> {
        cx.views
    }
}

const ENTRY: EntryId = EntryId(3);
const HOST: InstanceId = InstanceId(9);

fn channel(vcn: &str) -> Channel {
    Channel {
        sid: ServerId::UNSET,
        dvr: "7".into(),
        channel: format!("C{vcn}"),
        vcn: vcn.into(),
        name: format!("Station {vcn}"),
        thumb: String::new(),
        now: String::new(),
        now_thumb: String::new(),
        now_summary: String::new(),
        begins_at: 0,
        ends_at: 0,
    }
}

/// The handoff board's lineup, with 2010 on screen: `201` is a channel AND the prefix of six more.
fn board() -> Vec<Channel> {
    ["201", "2010", "2011", "2012", "2013", "2014", "2015", "3001", "3002", "44"].map(channel).to_vec()
}

fn on_screen(vcn: &str) -> Option<PlayingKey> {
    Some((ServerId::UNSET, "7".into(), format!("C{vcn}")))
}

fn panel_over(lineup: &[Channel], first: u8, playing: Option<PlayingKey>) -> ChannelEntryScreen {
    ChannelEntryScreen::new(ENTRY, ChannelEntryArg::opened_by(HOST, first, playing), lineup)
}

fn panel(first: u8) -> ChannelEntryScreen {
    panel_over(&board(), first, on_screen("2010"))
}

fn cx_at(ms: u32, focus: Option<u32>) -> Cx<'static, TestHost> {
    Cx {
        views: LiveTvView::for_test(LiveTvStatus::Ready, &[], &[], 1),
        tick: Tick { ms, dt_us: 16_000 },
        measure: &FixtureMeasure,
        press: PressRead::default(),
        modal: Default::default(),
        page_image: Default::default(),
        hold_hint: Default::default(),
        focus: FocusRead { current: focus.map(|elem| FocusKey { entry: ENTRY, elem }), ..Default::default() },
        owner: InputOwner::Entry(ENTRY),
    }
}

fn drive(s: &mut ChannelEntryScreen, ms: u32, focus: Option<u32>, ev: ScreenEvent<TestHost>) -> Vec<Stamped<TestHost>> {
    let mut out = Vec::new();
    let mut present = plx_machine::present::Present::new();
    let mut fx = Effects::new(&mut out, MachineId::Instance(InstanceId(4)), &mut present);
    Machine::<TestHost>::step(s, &ev, &cx_at(ms, focus), &mut fx);
    out
}

fn tick(s: &mut ChannelEntryScreen, ms: u32) -> Vec<Stamped<TestHost>> {
    drive(s, ms, None, ScreenEvent::Tick(Tick { ms, dt_us: 16_000 }))
}

fn press(s: &mut ChannelEntryScreen, ms: u32, key: Key, sym: u32) -> Vec<Stamped<TestHost>> {
    let kind = InputKind::Key { key, sym, wcode: 0, edge: Edge::Down, at_edge: false };
    drive(s, ms, None, ScreenEvent::Input(InputEvent { kind, at: Tick { ms, dt_us: 0 }, source: Source::Sdl }))
}

/// A digit key at `ms`.
fn digit(s: &mut ChannelEntryScreen, ms: u32, d: u8) -> Vec<Stamped<TestHost>> {
    press(s, ms, Key::Other, 48 + u32::from(d))
}

/// The channel keys the panel reported to its host.
fn picked(out: &[Stamped<TestHost>]) -> Vec<String> {
    out.iter()
        .filter_map(|e| match &e.fx {
            Fx::Deliver(
                MachineId::Instance(HOST),
                Delivery::Screen(ScreenEvent::App(AppMsg::LiveChannelPicked { channel, .. })),
            ) => Some(channel.clone()),
            _ => None,
        })
        .collect()
}

fn dismissed(out: &[Stamped<TestHost>]) -> bool {
    out.iter().any(|e| matches!(e.fx, Fx::Nav(NavOp::Dismiss(ENTRY))))
}

/// The lineup indices the panel asked the engine to put the cursor on.
fn seated(out: &[Stamped<TestHost>]) -> Vec<u32> {
    out.iter()
        .filter_map(|e| match &e.fx {
            Fx::Deliver(_, Delivery::Screen(ScreenEvent::Enter(Enter::Fresh { focus: FocusTarget::Elem(key) }))) => Some(key.elem),
            _ => None,
        })
        .collect()
}

/// The panel's root, run through the id conformance (`AppArg::ChannelEntry`; the coverage table calls this).
pub(crate) fn channel_entry_conforms() -> plx_ui::a11y::Namespaces {
    vec![plx_ui::a11y::conform::<TestHost, _>(&panel(2), &cx_at(0, None))]
}

/// Typing "2" lists every channel whose number starts with 2 in lineup order, the digit is on screen before any
/// event, and the first frame seats the cursor on the first match that is not the channel on screen.
#[test]
fn the_first_digit_lists_its_matches_and_the_first_frame_seats_the_highlight() {
    let mut p = panel(2);
    assert_eq!((p.machine().typed(), p.shown()), ("2", vec![0, 1, 2, 3, 4, 5, 6]));
    assert_eq!(seated(&tick(&mut p, 1_000)), vec![0], "201 is not on screen; 2010 is");
}

/// A number that matches one channel tunes at once: one pick to the host and the panel closes.
#[test]
fn a_prefix_that_fits_one_channel_is_reported_at_once_and_closes() {
    let mut p = panel(4);
    let out = tick(&mut p, 1_000);
    assert_eq!(picked(&out), vec!["C44".to_string()]);
    assert!(dismissed(&out));
}

/// Several matches never tune on a digit; 2.5 s after the last key the highlight tunes.
#[test]
fn an_ambiguous_prefix_waits_for_the_timeout_then_reports_the_highlight() {
    let mut p = panel(3);
    let out = tick(&mut p, 1_000);
    assert_eq!((seated(&out), picked(&out)), (vec![7], Vec::new()));
    assert!(tick(&mut p, 3_499).is_empty(), "not before 2.5 s");
    let out = tick(&mut p, 3_500);
    assert_eq!(picked(&out), vec!["C3001".to_string()]);
    assert!(dismissed(&out));
}

/// BACK closes at once and reports nothing, before or after digits, and the timeout then tunes nothing.
#[test]
fn back_closes_without_a_pick() {
    let mut p = panel(3);
    tick(&mut p, 1_000);
    let out = press(&mut p, 1_200, Key::Back, 0);
    assert_eq!((picked(&out), dismissed(&out)), (Vec::new(), true));
    assert!(picked(&tick(&mut p, 9_000)).is_empty());
}

/// Digits typed on the panel append, narrow the list and re-seat on the exact match; two in the same frame as the
/// opening one are not lost.
#[test]
fn typed_digits_narrow_the_list_and_seat_the_exact_match() {
    let mut p = panel(2);
    tick(&mut p, 1_000);
    assert_eq!(seated(&digit(&mut p, 1_100, 0)), vec![0], "20: nothing is 20; the first not on screen");
    let out = digit(&mut p, 1_200, 1);
    assert_eq!((p.machine().typed(), seated(&out)), ("201", vec![0]), "201 is a channel and the prefix of six more");
    assert_eq!(p.shown(), vec![0, 1, 2, 3, 4, 5, 6]);
    assert!(picked(&out).is_empty(), "an exact match that is also a prefix waits");
    // OK on the row (a click or Select): that channel
    let out = drive(&mut p, 1_300, None, ScreenEvent::Activate(0));
    assert_eq!(picked(&out), vec!["C201".to_string()]);
    // two digits in one frame, the second arriving with the first event
    let mut q = panel(3);
    let out = digit(&mut q, 5_000, 0);
    assert_eq!((q.machine().typed(), seated(&out)), ("30", vec![7]));
}

/// Digits that match nothing say so and tune nothing, not on OK and not at the timeout, which closes the panel.
#[test]
fn digits_that_match_nothing_read_no_channel_and_tune_nothing() {
    let _language = plx_os::i18n::language_on_this_thread_for_test(plx_os::i18n::Preference::En);
    let mut p = panel(2);
    tick(&mut p, 1_000);
    for (ms, d) in [(1_100, 0), (1_200, 9), (1_300, 9)] {
        digit(&mut p, ms, d);
    }
    assert_eq!((p.machine().typed(), p.shown()), ("2099", Vec::new()));
    assert_eq!(plx_os::i18n::msg::widgets_live_entry_none(p.machine().typed()), "No channel 2099");
    assert!(picked(&drive(&mut p, 1_400, None, ScreenEvent::Activate(0))).is_empty());
    let out = tick(&mut p, 1_300 + entry::IDLE_MS);
    assert_eq!((picked(&out), dismissed(&out)), (Vec::new(), true));
}

/// A number that is only the channel on screen closes without a pick: the stream is not restarted.
#[test]
fn the_channel_on_screen_is_never_retuned() {
    let mut p = panel(2);
    tick(&mut p, 1_000);
    for (ms, d) in [(1_100, 0), (1_200, 1), (1_300, 0)] {
        let out = digit(&mut p, ms, d);
        if d == 0 && ms == 1_300 {
            assert_eq!((picked(&out), dismissed(&out)), (Vec::new(), true));
        }
    }
}

/// **The panel shows what the machine typed, to the last digit**: the digit that finishes the entry (a tune, or the
/// channel on screen) is on screen while the panel fades, with the one row it matched; there is no copy to go stale.
#[test]
fn the_final_digit_is_shown_while_the_panel_fades() {
    let mut p = panel(2);
    tick(&mut p, 1_000);
    for (ms, d) in [(1_100, 0), (1_200, 1), (1_300, 0)] {
        digit(&mut p, ms, d);
    }
    assert!(p.machine().is_done());
    assert_eq!((p.machine().typed(), p.shown()), ("2010", vec![1]), "2010 is the channel on screen: it closes");
    assert_eq!(p.form.focusable_len(), 1, "the list was rebuilt for the final digits");
    // a tune: the same
    let mut q = panel(4);
    let out = tick(&mut q, 1_000);
    assert_eq!((picked(&out), q.machine().typed(), q.shown()), (vec!["C44".to_string()], "4", vec![9]));
}

/// **A surface dismissed from outside stops acting**: a press that misses the panel dismisses it through the
/// container, which delivers `Closing`; the 2.5 s timer must not then tune a channel for a panel nobody sees.
#[test]
fn a_panel_dismissed_from_outside_never_tunes_at_its_timeout() {
    let mut p = panel(3);
    tick(&mut p, 1_000);
    let out = drive(&mut p, 1_200, None, ScreenEvent::Closing);
    assert!(picked(&out).is_empty() && !dismissed(&out), "the container already dismissed it: no second dismissal");
    assert!(p.machine().is_done());
    let out = tick(&mut p, 1_000 + entry::IDLE_MS + 500);
    assert!(picked(&out).is_empty(), "the closing surface ticks, and tunes nothing");
    assert_eq!(p.machine().typed(), "3", "the digits stay on screen while it fades");
}

/// **Keys that arrived before the panel mounted are its opening digits**: two requests folded into one argument
/// open one panel typed "30", not two panels.
#[test]
fn keys_folded_into_the_parked_argument_open_one_panel_with_all_of_them() {
    let mut arg = ChannelEntryArg::opened_by(HOST, 3, on_screen("2010"));
    arg.absorb(&ChannelEntryArg::opened_by(HOST, 0, None));
    assert_eq!(arg.digits, "30");
    assert_eq!(arg.playing, on_screen("2010"), "the earlier request's playing channel stands");
    let mut p = ChannelEntryScreen::new(ENTRY, arg, &board());
    assert_eq!((p.machine().typed(), p.shown()), ("30", vec![7, 8]), "both digits on screen before any event");
    assert_eq!(seated(&tick(&mut p, 1_000)), vec![7]);
}

/// **An ATSC lineup**: typing 6 then 1 reaches `6.1`, and the row reads `6.1`.
#[test]
fn a_dotted_channel_is_reached_by_its_digits_and_listed_as_it_is() {
    let lineup: Vec<Channel> = ["6.1", "6.2", "6.3", "7"].map(channel).to_vec();
    let mut p = panel_over(&lineup, 6, None);
    assert_eq!(p.shown(), vec![0, 1, 2]);
    assert_eq!((p.cands[0].number.as_str(), p.cands[0].key.as_str()), ("6.1", "61"), "matched on 61, shown as 6.1");
    assert_eq!(p.lineup[0].vcn, "6.1", "the row is built from the channel's own number");
    tick(&mut p, 1_000);
    let out = digit(&mut p, 1_100, 1);
    assert_eq!(picked(&out), vec!["C6.1".to_string()]);
}

fn twelve() -> Vec<Channel> {
    (500..512).map(|n| channel(&n.to_string())).collect()
}

/// **The window is five rows, and the rest sit under it as "+N more"**: more or fewer matches change the panel's height
/// by the rows shown, never beyond five, and only a list longer than five carries the line.
#[test]
fn the_list_is_a_window_of_five_rows_with_the_rest_counted_under_it() {
    let m = &FixtureMeasure;
    let long = panel_over(&twelve(), 5, None);
    assert_eq!((long.shown().len(), long.more()), (12, 7));
    let few = panel_over(&twelve()[..5], 5, None);
    assert_eq!((few.shown().len(), few.more()), (5, 0));
    let three = panel_over(&twelve()[..3], 5, None);
    let more_line = plx_ui::theme::space::SM + m.cap_h(plx_ui::theme::size::MICRO);
    assert_eq!(long.rect(m).h, few.rect(m).h + more_line, "the same five rows, plus the line under them");
    assert_eq!(few.rect(m).h - three.rect(m).h, 2.0 * ROW_H, "two rows fewer");
    // the panel stays on the safe frame's top-right corner at every height
    for p in [&long, &few, &three] {
        let r = p.rect(m);
        assert_eq!((r.x + r.w, r.y), (plx_ui::consts::SAFE.x + plx_ui::consts::SAFE.w, plx_ui::consts::SAFE.y));
        assert_eq!(r.w, 560.0);
    }
    // the count follows the digits: 5 (twelve), 50 (ten: 500-509), 505 (one) narrow it
    let mut p = panel_over(&twelve(), 5, None);
    tick(&mut p, 1_000);
    digit(&mut p, 1_100, 0);
    assert_eq!((p.shown().len(), p.more()), (10, 5));
    let out = digit(&mut p, 1_200, 5);
    assert_eq!(picked(&out), vec!["C505".to_string()], "505 fits one channel: tuned at once");
}

/// Down moves the cursor through the matches and the window scrolls with it: the last of twelve rows is inside the
/// table's frame once the scroll has settled, and the neighbour walk stops at the ends.
#[test]
fn down_scrolls_the_window_to_the_cursor() {
    let mut p = panel_over(&twelve(), 5, None);
    tick(&mut p, 1_000);
    let frame = p.frame(&FixtureMeasure);
    let mut at = FocusKey { entry: ENTRY, elem: 0 };
    for step in 1..12u32 {
        match Focusable::<TestHost>::neighbour(&p, at, Dir::Down, &cx_at(0, None)) {
            Step::Move(next) => {
                assert_eq!(next.elem, step);
                at = next;
            }
            other => panic!("step {step}: {other:?}"),
        }
    }
    assert!(matches!(Focusable::<TestHost>::neighbour(&p, at, Dir::Down, &cx_at(0, None)), Step::Edge));
    for ms in (1_016..3_000).step_by(16) {
        drive(&mut p, ms, Some(11), ScreenEvent::Tick(Tick { ms, dt_us: 16_000 }));
    }
    let placed = Focusable::<TestHost>::place(&p, &11, &cx_at(0, None), At::Drawn).expect("row 11 is placed");
    assert!(
        placed.rect.y >= frame.y && placed.rect.y + placed.rect.h <= frame.y + frame.h + 0.5,
        "{:?} is not inside the window {frame:?}",
        placed.rect
    );
}

/// The cursor moving keeps the timer from running out under the viewer's hand: 2.5 s after the LAST key, on the row
/// the cursor is on.
#[test]
fn the_cursor_moving_restarts_the_timeout_and_the_pick_is_where_it_stands() {
    let mut p = panel(3);
    tick(&mut p, 1_000);
    drive(&mut p, 2_000, None, ScreenEvent::FocusMoved {
        from: Some(FocusKey { entry: ENTRY, elem: 7 }),
        to: FocusKey { entry: ENTRY, elem: 8 },
        by: plx_machine::machine::By::Dir,
    });
    assert!(tick(&mut p, 2_000 + entry::IDLE_MS - 1).is_empty());
    let out = tick(&mut p, 2_000 + entry::IDLE_MS);
    assert_eq!(picked(&out), vec!["C3002".to_string()]);
}
