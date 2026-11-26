use crate::json::Json;
use crate::media::MAX_DURATION_MS;
use crate::text::equal_ignoring_ascii_case;

pub const MATCH_TOLERANCE_MS: i64 = 1500;
pub const MAX_PROBES: usize = 4;
pub const PROBE_BUDGET_MS: i64 = 60_000;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Candidate {
    pub id: String,
    pub duration_ms: Option<i64>,
    pub size_bytes: Option<i64>,
    pub oshash: Option<String>,
    pub name: Option<String>,
    pub probe: bool,
}

pub fn counting(duration_ms: Option<i64>) -> Option<i64> {
    duration_ms.filter(|ms| (1..=MAX_DURATION_MS).contains(ms))
}

fn media_duration_ms(media: &Json) -> i64 {
    media.get("durationMs").and_then(Json::as_int).unwrap_or(0)
}

fn file_text<'a>(file: Option<&'a Json>, name: &str) -> Option<&'a str> {
    file?.get(name).and_then(Json::as_text).filter(|text| !text.unpaired).map(|text| text.value.as_str())
}

fn fold_equal(hint: Option<&str>, file_value: Option<&str>) -> bool {
    matches!((hint, file_value), (Some(a), Some(b)) if equal_ignoring_ascii_case(a, b))
}

pub fn rank(media: &Json, candidates: Vec<Candidate>) -> Vec<Candidate> {
    let file = media.get("file");
    let size_bytes = file.and_then(|file| file.get("sizeBytes")).and_then(Json::as_int);
    let duration_ms = media_duration_ms(media);
    let mut ranked = candidates;
    ranked.sort_by_key(|candidate| {
        let known = counting(candidate.duration_ms);
        (
            !fold_equal(candidate.oshash.as_deref(), file_text(file, "oshash")),
            !(candidate.size_bytes.is_some() && candidate.size_bytes == size_bytes),
            !fold_equal(candidate.name.as_deref(), file_text(file, "name")),
            known.is_none(),
            known.map_or(0, |ms| (ms - duration_ms).abs()),
        )
    });
    ranked
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Matched { index: usize, delta_ms: i64 },
    Mismatch { index: usize, delta_ms: i64 },
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProbeRun {
    pub duration_ms: Option<i64>,
    pub took_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    Probing { index: usize, deadline_ms: i64 },
    Done(Outcome),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lead {
    Duration(i64),
    Probe,
    Nothing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Probes {
    Left { counted: usize, spent_ms: i64 },
    Over,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Walk {
    media_duration_ms: i64,
    leads: Vec<Lead>,
    deltas: Vec<Option<i64>>,
    probed: Vec<usize>,
    probes: Probes,
    closest: Option<(usize, i64)>,
    progress: Progress,
}

impl Walk {
    pub fn new(media: &Json, ranked: &[Candidate]) -> Walk {
        let media_duration_ms = media_duration_ms(media);
        let leads: Vec<Lead> = ranked
            .iter()
            .map(|candidate| match (counting(candidate.duration_ms), candidate.probe) {
                (Some(ms), _) => Lead::Duration(ms),
                (None, true) => Lead::Probe,
                (None, false) => Lead::Nothing,
            })
            .collect();
        let deltas = leads
            .iter()
            .map(|lead| match lead {
                Lead::Duration(ms) => Some(ms - media_duration_ms),
                Lead::Probe | Lead::Nothing => None,
            })
            .collect();
        let start = Walk {
            media_duration_ms,
            leads,
            deltas,
            probed: Vec::new(),
            probes: Probes::Left { counted: 0, spent_ms: 0 },
            closest: None,
            progress: Progress::Done(Outcome::Unavailable),
        };
        start.continue_at(0)
    }

    pub fn progress(&self) -> Progress {
        self.progress
    }

    pub fn deltas(&self) -> &[Option<i64>] {
        &self.deltas
    }

    pub fn probed(&self) -> &[usize] {
        &self.probed
    }

    pub fn answered(mut self, run: ProbeRun) -> Walk {
        let Progress::Probing { index, deadline_ms } = self.progress else { return self };
        let took_ms = run.took_ms.max(0);
        if took_ms > deadline_ms {
            self.probes = Probes::Over;
            return self.continue_at(index + 1);
        }
        self.probes = match self.probes {
            Probes::Left { counted, spent_ms } if counted + 1 < MAX_PROBES => Probes::Left { counted: counted + 1, spent_ms: spent_ms + took_ms },
            Probes::Left { .. } | Probes::Over => Probes::Over,
        };
        self.probed.push(index);
        if let Some(ms) = counting(run.duration_ms) {
            self.deltas[index] = Some(ms - self.media_duration_ms);
            if let Some(outcome) = self.weigh(index, ms) {
                self.progress = Progress::Done(outcome);
                return self;
            }
        }
        self.continue_at(index + 1)
    }

    fn weigh(&mut self, index: usize, duration_ms: i64) -> Option<Outcome> {
        let delta_ms = duration_ms - self.media_duration_ms;
        if delta_ms.abs() <= MATCH_TOLERANCE_MS {
            return Some(Outcome::Matched { index, delta_ms });
        }
        if self.closest.is_none_or(|(_, best)| delta_ms.abs() < best.abs()) {
            self.closest = Some((index, delta_ms));
        }
        None
    }

    fn continue_at(mut self, start: usize) -> Walk {
        for index in start..self.leads.len() {
            match (self.leads[index], self.probes) {
                (Lead::Duration(ms), _) => {
                    if let Some(outcome) = self.weigh(index, ms) {
                        self.progress = Progress::Done(outcome);
                        return self;
                    }
                }
                (Lead::Probe, Probes::Left { spent_ms, .. }) => {
                    self.progress = Progress::Probing { index, deadline_ms: PROBE_BUDGET_MS - spent_ms };
                    return self;
                }
                (Lead::Probe, Probes::Over) | (Lead::Nothing, _) => {}
            }
        }
        self.progress = Progress::Done(self.closest.map_or(Outcome::Unavailable, |(index, delta_ms)| Outcome::Mismatch { index, delta_ms }));
        self
    }
}
