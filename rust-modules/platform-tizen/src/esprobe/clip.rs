//! The probe's bundled clips: one elementary stream per file, already split into access units (`PLXES1`, written by
//! `tools/make-esprobe-clips.py`, whose docstring is the format's definition), and the PCM clip the probe makes itself.
//!
//! A clip is read whole into memory (the largest is about 270 KB) and checked here, before anything reaches the native
//! player: a truncated or foreign file is a refusal with a reason, never a packet handed over with a length that runs past
//! the buffer.

/// What the stream is, in the probe's own words; [`super::api`] maps it to the native player's MIME enum.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec {
    H264,
    Hevc,
    Av1,
    Vp9,
    Aac,
    Ac3,
    Eac3,
    PcmS16le,
}

impl Codec {
    fn from_wire(byte: u8) -> Option<Codec> {
        Some(match byte {
            1 => Codec::H264,
            2 => Codec::Hevc,
            3 => Codec::Av1,
            4 => Codec::Vp9,
            16 => Codec::Aac,
            17 => Codec::Ac3,
            18 => Codec::Eac3,
            _ => return None,
        })
    }

    pub fn is_video(self) -> bool {
        matches!(self, Codec::H264 | Codec::Hevc | Codec::Av1 | Codec::Vp9)
    }

    pub fn name(self) -> &'static str {
        match self {
            Codec::H264 => "h264",
            Codec::Hevc => "hevc",
            Codec::Av1 => "av1",
            Codec::Vp9 => "vp9",
            Codec::Aac => "aac",
            Codec::Ac3 => "ac3",
            Codec::Eac3 => "eac3",
            Codec::PcmS16le => "pcm_s16le",
        }
    }
}

/// One access unit. `pts_ms` is the presentation time in the native player's default unit, milliseconds; the file keeps
/// microseconds, so the rounding is here and nowhere else.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Packet {
    pub pts_ms: u64,
    pub duration_ms: u64,
    pub key: bool,
    pub data: Vec<u8>,
}

/// One stream. Video: `a`/`b` are width and height, `c`/`d` the frame rate. Audio: `a`/`b` are channels and sample
/// rate, `c` the bit rate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Clip {
    pub codec: Codec,
    /// HEVC 10-bit carrying HDR10 colour (PQ, BT.2020): the probe attaches the colour description to its packets.
    pub hdr10: bool,
    pub a: u32,
    pub b: u32,
    pub c: u32,
    pub d: u32,
    pub codec_data: Vec<u8>,
    pub packets: Vec<Packet>,
}

impl Clip {
    /// The last presented time plus its duration: where the stream ends.
    pub fn end_ms(&self) -> u64 {
        self.packets.iter().map(|p| p.pts_ms + p.duration_ms).max().unwrap_or(0)
    }

    /// The index to resume feeding at after a seek to `target_ms`: for video the last keyframe, in decode order, whose
    /// presentation time is at or before the target (the decoder needs it, and the player drops what precedes the
    /// target); for audio the first packet that ends after the target. `None` when the target is past the end.
    pub fn resume_index(&self, target_ms: u64) -> Option<usize> {
        if target_ms >= self.end_ms() {
            return None;
        }
        if self.codec.is_video() {
            let at = self.packets.iter().rposition(|p| p.key && p.pts_ms <= target_ms);
            return Some(at.unwrap_or(0));
        }
        self.packets.iter().position(|p| p.pts_ms + p.duration_ms > target_ms)
    }
}

pub const MAGIC: &[u8; 8] = b"PLXES1\n\0";
/// More than any bundled clip holds; a count above it is a corrupt file, not a reason to allocate.
const MAX_PACKETS: usize = 4096;
/// The largest access unit a bundled clip may carry.
const MAX_PACKET_BYTES: usize = 1 << 20;

/// Why a clip file was refused. Every variant names what was wrong; the probe logs it as the step's FAIL.
#[derive(Debug, PartialEq, Eq)]
pub enum ClipError {
    Magic,
    Truncated(&'static str),
    Kind(u8),
    Codec(u8),
    KindMismatch,
    TooMany(usize),
    Empty,
    PacketTooLarge(usize),
    Trailing(usize),
}

struct Reader<'a> {
    rest: &'a [u8],
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize, what: &'static str) -> Result<&'a [u8], ClipError> {
        if self.rest.len() < n {
            return Err(ClipError::Truncated(what));
        }
        let (head, tail) = self.rest.split_at(n);
        self.rest = tail;
        Ok(head)
    }
    fn u8(&mut self, what: &'static str) -> Result<u8, ClipError> {
        Ok(self.take(1, what)?[0])
    }
    fn u16(&mut self, what: &'static str) -> Result<u16, ClipError> {
        let b = self.take(2, what)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    fn u32(&mut self, what: &'static str) -> Result<u32, ClipError> {
        let b = self.take(4, what)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn i64(&mut self, what: &'static str) -> Result<i64, ClipError> {
        let b = self.take(8, what)?;
        let mut a = [0u8; 8];
        a.copy_from_slice(b);
        Ok(i64::from_le_bytes(a))
    }
}

fn us_to_ms(us: u64) -> u64 {
    (us + 500) / 1000
}

/// Read a `PLXES1` file.
pub fn parse(bytes: &[u8]) -> Result<Clip, ClipError> {
    let mut r = Reader { rest: bytes };
    if r.take(8, "magic").map_err(|_| ClipError::Magic)? != MAGIC {
        return Err(ClipError::Magic);
    }
    let kind = r.u8("kind")?;
    if kind != 1 && kind != 2 {
        return Err(ClipError::Kind(kind));
    }
    let wire = r.u8("codec")?;
    let codec = Codec::from_wire(wire).ok_or(ClipError::Codec(wire))?;
    if codec.is_video() != (kind == 1) {
        return Err(ClipError::KindMismatch);
    }
    let flags = r.u16("flags")?;
    let (a, b, c, d) = (r.u32("a")?, r.u32("b")?, r.u32("c")?, r.u32("d")?);
    let cd_len = r.u32("codec data length")? as usize;
    let codec_data = r.take(cd_len, "codec data")?.to_vec();
    let count = r.u32("packet count")? as usize;
    if count > MAX_PACKETS {
        return Err(ClipError::TooMany(count));
    }
    if count == 0 {
        return Err(ClipError::Empty);
    }
    let mut packets = Vec::with_capacity(count);
    for _ in 0..count {
        let pts_us = r.i64("pts")?.max(0) as u64;
        let dur_us = u64::from(r.u32("duration")?);
        let key = r.u8("key")? != 0;
        r.take(3, "pad")?;
        let len = r.u32("packet length")? as usize;
        if len > MAX_PACKET_BYTES {
            return Err(ClipError::PacketTooLarge(len));
        }
        let data = r.take(len, "packet data")?.to_vec();
        packets.push(Packet { pts_ms: us_to_ms(pts_us), duration_ms: us_to_ms(dur_us), key, data });
    }
    if !r.rest.is_empty() {
        return Err(ClipError::Trailing(r.rest.len()));
    }
    Ok(Clip { codec, hdr10: flags & 1 != 0, a, b, c, d, codec_data, packets })
}

/// The PCM clip, made here rather than shipped: `seconds` of a quiet 400 Hz triangle tone (integer arithmetic: 120
/// samples a period), 48 kHz stereo signed 16-bit little-endian, in 20 ms packets.
pub fn pcm_tone(seconds: u32) -> Clip {
    const RATE: u32 = 48_000;
    const PACKET_MS: u64 = 20;
    const PERIOD: i64 = 120;
    const PEAK: i64 = 6000;
    let per_packet = (RATE as u64 * PACKET_MS / 1000) as usize;
    let total = (u64::from(seconds) * 1000 / PACKET_MS) as usize;
    let mut packets = Vec::with_capacity(total);
    let mut n: i64 = 0;
    for i in 0..total {
        let mut data = Vec::with_capacity(per_packet * 4);
        for _ in 0..per_packet {
            let t = n % PERIOD;
            let half = PERIOD / 2;
            let s = (if t < half { -PEAK + 4 * PEAK * t / PERIOD } else { 3 * PEAK - 4 * PEAK * t / PERIOD }) as i16;
            data.extend_from_slice(&s.to_le_bytes());
            data.extend_from_slice(&s.to_le_bytes());
            n += 1;
        }
        packets.push(Packet { pts_ms: i as u64 * PACKET_MS, duration_ms: PACKET_MS, key: true, data });
    }
    Clip { codec: Codec::PcmS16le, hdr10: false, a: 2, b: RATE, c: RATE * 2 * 16, d: 0, codec_data: Vec::new(), packets }
}
