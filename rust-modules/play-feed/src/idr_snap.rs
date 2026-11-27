//! idr_snap — keep Dolby Vision across a seek by landing on an IDR (#1166's other half).
//!
//! The firmware's Dolby Vision splitter takes only an HEVC IDR as the first picture after a flush
//! ([`crate::dv_start`]). A seek's landing is the keyframe `av_seek_frame(.., BACKWARD)` finds, which a
//! CRA often is, and then the guard withdraws the declaration and the play drops to the HDR10 base
//! layer. Before giving the picture up, look backward for an IDR: seek a little earlier, read the first
//! video keyframe, and repeat while it is a CRA or BLA, within a bounded distance and number of probes.
//! A file with no IDR in reach (its only IDR is frame 0) is [`Snap::Prime`]: the demuxer feeds that IDR
//! ahead of the landing so the splitter latches, and drops the landing's leading pictures.
//!
//! Pure: the demux thread implements [`Probe`] over its format context, so every rule is a unit test.

/// What a probe read at a position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Found {
    /// The keyframe's pts, in the stream's time base.
    pub pts: i64,
    /// It is an HEVC IDR (NAL 19 or 20), which the splitter takes first.
    pub idr: bool,
}

/// Seek to a stream timestamp and read the first video keyframe from there.
pub trait Probe {
    fn land(&mut self, ts: i64) -> Option<Found>;
}

/// The most probes a seek makes after its first landing.
pub const PROBE_TRIES: u32 = 6;

/// How far before the seek's target an IDR may sit, in milliseconds: the viewer lands that much
/// earlier than asked, which beats losing Dolby Vision, and no further.
pub const BACK_REACH_MS: i64 = 20_000;

/// The answer for one seek.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Snap {
    /// Seek to the original target: the landing is already an IDR, or nothing could be read.
    Stay,
    /// Seek to the original target, which lands on a non-IDR with no IDR in reach: the caller primes
    /// the splitter with the file's first IDR ahead of the landing.
    Prime,
    /// Seek to this timestamp: it is an IDR's pts.
    To(i64),
}

/// Find an IDR at or before `target`, not earlier than `floor`. `unit` is the smallest step back (a
/// tenth of a second, in the stream's time base); a probe that lands where the last one did doubles it.
pub fn snap<P: Probe>(probe: &mut P, target: i64, floor: i64, unit: i64) -> Snap {
    let unit = unit.max(1);
    let Some(first) = probe.land(target) else { return Snap::Stay };
    if first.idr {
        return Snap::Stay;
    }
    let (mut cur, mut at, mut step) = (first.pts, target, unit);
    for _ in 0..PROBE_TRIES {
        at = at.min(cur).saturating_sub(step);
        if at < floor {
            return Snap::Prime;
        }
        let Some(found) = probe.land(at) else { return Snap::Prime };
        if found.idr {
            return if found.pts >= floor { Snap::To(found.pts) } else { Snap::Prime };
        }
        if found.pts < cur {
            cur = found.pts;
            step = unit;
        } else {
            step = step.saturating_mul(2);
        }
    }
    Snap::Prime
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file's keyframes as (pts, idr); `land(ts)` is the last keyframe at or before `ts`.
    struct File {
        keys: Vec<(i64, bool)>,
        probes: Vec<i64>,
        /// How far before its pts a keyframe's index entry sits (an MP4's dts).
        lag: i64,
    }

    impl File {
        fn new(keys: &[(i64, bool)]) -> Self {
            File { keys: keys.to_vec(), probes: Vec::new(), lag: 0 }
        }
    }

    impl Probe for File {
        fn land(&mut self, ts: i64) -> Option<Found> {
            self.probes.push(ts);
            self.keys.iter().rev().find(|k| k.0 - self.lag <= ts).map(|&(pts, idr)| Found { pts, idr })
        }
    }

    const S: i64 = 1000;

    #[test]
    fn an_idr_landing_stays() {
        let mut f = File::new(&[(0, true), (50 * S, true), (60 * S, false)]);
        assert_eq!(snap(&mut f, 55 * S, 35 * S, 100), Snap::Stay);
        assert_eq!(f.probes, [55 * S], "one probe, nothing further");
    }

    #[test]
    fn a_cra_landing_walks_back_to_the_idr() {
        let mut f = File::new(&[(0, true), (40 * S, true), (48 * S, false), (52 * S, false), (56 * S, false)]);
        assert_eq!(snap(&mut f, 57 * S, 37 * S, 100), Snap::To(40 * S));
    }

    #[test]
    fn a_file_whose_only_idr_is_frame_zero_stays() {
        let mut f = File::new(&[(0, true), (10 * S, false), (20 * S, false), (30 * S, false), (56 * S, false)]);
        assert_eq!(snap(&mut f, 57 * S, 37 * S, 100), Snap::Prime);
        assert!(f.probes.iter().all(|&p| p >= 37 * S), "no probe below the floor: {:?}", f.probes);
    }

    #[test]
    fn an_idr_below_the_floor_is_not_taken() {
        let mut f = File::new(&[(0, true), (10 * S, true), (56 * S, false)]);
        assert_eq!(snap(&mut f, 57 * S, 37 * S, 100), Snap::Prime);
    }

    #[test]
    fn a_probe_that_lands_where_the_last_did_steps_back_further() {
        // The index entry sits 200 ticks before the pts, so a probe 100 ticks back finds the same CRA.
        let mut f = File::new(&[(0, true), (30 * S, true), (50 * S, false), (56 * S, false)]);
        f.lag = 200;
        assert_eq!(snap(&mut f, 56 * S + 500, 26 * S, 100), Snap::To(30 * S));
        assert_eq!(f.probes.iter().filter(|&&p| p == 56 * S - 100).count(), 1, "no probe repeated: {:?}", f.probes);
    }

    #[test]
    fn the_number_of_probes_is_bounded() {
        let keys: Vec<(i64, bool)> = (1..=40).map(|i| (i * S / 2, false)).collect();
        let mut f = File::new(&keys);
        assert_eq!(snap(&mut f, 20 * S, 0, 100), Snap::Prime);
        assert!(f.probes.len() as u32 <= 1 + PROBE_TRIES);
    }

    #[test]
    fn a_probe_that_reads_nothing_stays() {
        let mut f = File::new(&[]);
        assert_eq!(snap(&mut f, 5 * S, 0, 100), Snap::Stay);
    }
}
