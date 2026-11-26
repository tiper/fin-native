use crate::timeline::{Phase, Timeline};

pub const DEADBAND_MS: i64 = 350;
pub const NUDGE_EXIT_MS: i64 = 150;
pub const SEEK_MS: i64 = 2000;
pub const SEEK_ONLY_MS: i64 = 1000;
pub const HELD_SEEK_MS: i64 = 500;
pub const SEEK_LEAD_MS: i64 = 250;
pub const SEEK_COOLDOWN_MS: i64 = 2000;
pub const END_MARGIN_MS: i64 = 1000;
pub const RATE_EPSILON: f64 = 0.001;
pub const SLOWER: f64 = 0.96;
pub const FASTER: f64 = 1.04;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Player {
    pub position_ms: i64,
    pub paused: bool,
    pub buffering: bool,
    pub duration_ms: Option<i64>,
    pub can_set_rate: bool,
    pub applied_rate: f64,
    pub seek_accuracy_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StepInput {
    pub room: Timeline,
    pub duration_ms: i64,
    pub relay_now_ms: i64,
    pub offset_ms: i64,
    pub player: Player,
    pub nudging: bool,
    pub since_seek_ms: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Action {
    Pause,
    SetRate { rate: f64 },
    Seek { position_ms: i64 },
    Play,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Step {
    Idle,
    Held { pause: bool, set_rate: Option<f64>, seek_to_ms: Option<i64>, play_in_ms: Option<i64> },
    AtEnd { set_rate: Option<f64> },
    Buffering { nudging: bool },
    Following { set_rate: Option<f64>, seek_to_ms: Option<i64>, play: bool, nudging: bool },
}

impl Step {
    pub fn actions(&self) -> Vec<Action> {
        let (pause, set_rate, seek_to_ms, play) = match *self {
            Step::Idle | Step::Buffering { .. } => (false, None, None, false),
            Step::Held { pause, set_rate, seek_to_ms, .. } => (pause, set_rate, seek_to_ms, false),
            Step::AtEnd { set_rate } => (false, set_rate, None, false),
            Step::Following { set_rate, seek_to_ms, play, .. } => (false, set_rate, seek_to_ms, play),
        };
        let mut actions = Vec::new();
        if pause {
            actions.push(Action::Pause);
        }
        actions.extend(set_rate.map(|rate| Action::SetRate { rate }));
        actions.extend(seek_to_ms.map(|position_ms| Action::Seek { position_ms }));
        if play {
            actions.push(Action::Play);
        }
        actions
    }

    pub fn nudging(&self) -> bool {
        match *self {
            Step::Buffering { nudging } | Step::Following { nudging, .. } => nudging,
            Step::Idle | Step::Held { .. } | Step::AtEnd { .. } => false,
        }
    }

    pub fn play_in_ms(&self) -> Option<i64> {
        match *self {
            Step::Held { play_in_ms, .. } => play_in_ms,
            Step::Idle | Step::AtEnd { .. } | Step::Buffering { .. } | Step::Following { .. } => None,
        }
    }
}

struct Hold {
    at_ms: i64,
    play_in_ms: Option<i64>,
}

fn end_limit(duration_ms: Option<i64>) -> Option<i64> {
    duration_ms.filter(|ms| *ms > 0).map(|ms| (ms - END_MARGIN_MS).max(0))
}

fn clamp_local(position_ms: i64, duration_ms: Option<i64>) -> i64 {
    let floor = position_ms.max(0);
    end_limit(duration_ms).map_or(floor, |end| floor.min(end))
}

fn rate_change(player: &Player, want: f64) -> Option<f64> {
    (player.can_set_rate && (player.applied_rate - want).abs() > RATE_EPSILON).then(|| (want * 1e6).round() / 1e6)
}

fn hold(room: &Timeline, offset_ms: i64, relay_now_ms: i64, player_duration_ms: Option<i64>) -> Option<Hold> {
    let anchor = room.position_ms + offset_ms;
    let play_in_ms = match room.phase {
        Phase::Idle | Phase::Paused | Phase::Waiting => None,
        Phase::Playing => {
            let lead = if anchor < 0 { -(anchor as f64) / room.rate } else { 0.0 };
            let start_at = room.at_ms as f64 + lead;
            let now = relay_now_ms as f64;
            if now >= start_at {
                return None;
            }
            Some((start_at - now).ceil() as i64)
        }
    };
    Some(Hold { at_ms: clamp_local(anchor, player_duration_ms), play_in_ms })
}

pub fn step(input: &StepInput) -> Step {
    let (room, player) = (&input.room, &input.player);
    if room.phase == Phase::Idle {
        return Step::Idle;
    }
    let accuracy = player.seek_accuracy_ms.max(0);
    let cooled = input.since_seek_ms.is_none_or(|ms| ms >= SEEK_COOLDOWN_MS);
    if let Some(hold) = hold(room, input.offset_ms, input.relay_now_ms, player.duration_ms) {
        let off_by = player.position_ms.saturating_sub(hold.at_ms).saturating_abs();
        return Step::Held {
            pause: !player.paused,
            set_rate: rate_change(player, room.rate),
            seek_to_ms: (off_by > HELD_SEEK_MS.max(accuracy) && cooled).then_some(hold.at_ms),
            play_in_ms: hold.play_in_ms,
        };
    }
    let target = (room.at(input.duration_ms, input.relay_now_ms).position_ms + input.offset_ms).max(0);
    let nudging = input.nudging && player.can_set_rate;
    if end_limit(player.duration_ms).is_some_and(|end| target >= end && player.position_ms >= end) {
        return Step::AtEnd { set_rate: rate_change(player, room.rate) };
    }
    if player.buffering {
        return Step::Buffering { nudging };
    }
    let drift = player.position_ms.saturating_sub(target);
    let size = drift.saturating_abs();
    let (nudging, seek) = match (player.can_set_rate, nudging) {
        (true, _) if size > SEEK_MS.max(accuracy) => (false, cooled),
        (true, true) => (size >= NUDGE_EXIT_MS, false),
        (true, false) => (size > DEADBAND_MS, false),
        (false, _) => (false, size > SEEK_ONLY_MS.max(accuracy) && cooled),
    };
    let want = if nudging { room.rate * if drift > 0 { SLOWER } else { FASTER } } else { room.rate };
    Step::Following {
        set_rate: rate_change(player, want),
        seek_to_ms: seek.then(|| clamp_local(target + SEEK_LEAD_MS, player.duration_ms)),
        play: player.paused,
        nudging,
    }
}
