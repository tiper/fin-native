#!/usr/bin/env python3
"""Make the packet-player probe's bundled clips (`tizen/esprobe/*.plxes`; docs/tizen.md, "The packet player").

    python3 tools/make-esprobe-clips.py [--ffmpeg PATH] [--ffprobe PATH] [--out DIR]

The Tizen packet-player probe (`rust-modules/platform-tizen/src/esprobe/`) feeds the television's elementary-stream player
from files it ships, never from the viewer's media. Each file is one elementary stream, already split into access units with
their timestamps, in the form the Feed engine hands its sink on webOS (`plx_play_feed::ff`): H.264 and HEVC as Annex-B with
the parameter sets in-band on every keyframe (an MPEG-TS source gives exactly that), AAC as ADTS frames, AC-3 and E-AC-3 as
their own sync frames, AV1 and VP9 as the frames Matroska carries (with AV1's `av1C` as the codec data). The H.264 clip has
B-frames, so its access units arrive in decode order with presentation times that are not monotonic, as a real file's do.

The test patterns are synthetic (`testsrc2`, `sine`): no third-party content. The output is deterministic for one FFmpeg build
(`-bitexact`, fixed seeds are not needed: the sources are synthetic); a different FFmpeg may encode other bytes, which is fine,
because the probe reads only the format below.

THE FORMAT (`PLXES1`, little-endian; the reader is `esprobe::clip`):

    magic   8 bytes  b"PLXES1\\n\\0"
    kind    u8       1 video, 2 audio
    codec   u8       1 H.264, 2 HEVC, 3 AV1, 4 VP9, 16 AAC, 17 AC-3, 18 E-AC-3
    flags   u16      bit 0: HEVC 10-bit with HDR10 (PQ, BT.2020) colour
    a, b    u32 u32  video: width, height        audio: channels, sample rate
    c, d    u32 u32  video: frame rate num, den  audio: bit rate, 0
    cdlen   u32      then cdlen bytes of codec data (may be 0)
    count   u32      then count packets, each:
        pts_us   i64   presentation time in microseconds, the stream's first presented time being 0
        dur_us   u32   duration in microseconds
        key      u8    1 for a keyframe (random access point)
        pad      3 bytes, zero
        len      u32   then len bytes of the access unit

Standard library only, plus the `ffmpeg` and `ffprobe` executables; Python 3.10 or newer.
"""
from __future__ import annotations

import argparse
import json
import struct
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
MAGIC = b"PLXES1\n\0"

VIDEO, AUDIO = 1, 2
H264, HEVC, AV1, VP9, AAC, AC3, EAC3 = 1, 2, 3, 4, 16, 17, 18
FLAG_HDR10 = 1

W, H, FPS = 640, 360, 25


@dataclass(frozen=True)
class Clip:
    name: str
    kind: int
    codec: int
    seconds: float
    container: str
    args: tuple[str, ...]
    flags: int = 0


def video_src(seconds: float) -> tuple[str, ...]:
    return ("-f", "lavfi", "-i", f"testsrc2=size={W}x{H}:rate={FPS}:duration={seconds}")


def audio_src(seconds: float) -> tuple[str, ...]:
    return ("-f", "lavfi", "-i", f"sine=frequency=440:sample_rate=48000:duration={seconds}", "-ac", "2")


CLIPS = (
    # The probe's main run: 8 s, so position, pause, a seek to the middle and the end all fit.
    Clip("h264", VIDEO, H264, 8, "mpegts", ("-c:v", "libx264", "-profile:v", "high", "-pix_fmt", "yuv420p", "-preset", "medium",
                                            "-crf", "34", "-g", "25", "-bf", "2", "-x264-params", "aud=1")),
    Clip("aac", AUDIO, AAC, 8, "mpegts", ("-c:a", "aac", "-b:a", "64k")),
    # One codec each: is it accepted, does it decode.
    Clip("hevc", VIDEO, HEVC, 2, "mpegts", ("-c:v", "libx265", "-pix_fmt", "yuv420p", "-crf", "34", "-g", "25",
                                            "-x265-params", "log-level=error:aud=1:repeat-headers=1")),
    Clip("hevc10", VIDEO, HEVC, 2, "mpegts", ("-c:v", "libx265", "-pix_fmt", "yuv420p10le", "-crf", "34", "-g", "25",
                                              "-color_primaries", "bt2020", "-color_trc", "smpte2084", "-colorspace", "bt2020nc",
                                              "-x265-params", "log-level=error:aud=1:repeat-headers=1:hdr10=1:"
                                              "master-display=G(13250,34500)B(7500,3000)R(34000,16000)WP(15635,16450)L(10000000,1):"
                                              "max-cll=1000,400"), FLAG_HDR10),
    Clip("av1", VIDEO, AV1, 2, "matroska", ("-c:v", "libaom-av1", "-pix_fmt", "yuv420p", "-crf", "45", "-b:v", "0", "-g", "25",
                                            "-cpu-used", "8")),
    Clip("vp9", VIDEO, VP9, 2, "matroska", ("-c:v", "libvpx-vp9", "-pix_fmt", "yuv420p", "-crf", "45", "-b:v", "0", "-g", "25",
                                            "-deadline", "realtime", "-cpu-used", "8")),
    Clip("ac3", AUDIO, AC3, 2, "mpegts", ("-c:a", "ac3", "-b:a", "192k")),
    Clip("eac3", AUDIO, EAC3, 2, "mpegts", ("-c:a", "eac3", "-b:a", "192k")),
)


def hexdump_bytes(dump: str) -> bytes:
    """The bytes of an FFmpeg `av_hex_dump` text: `%08x: ` then eight groups of four hex digits, then the ASCII column."""
    out = bytearray()
    for line in dump.splitlines():
        if len(line) < 10 or line[8:10] != ": ":
            continue
        out += bytes.fromhex(line[10:50].replace(" ", ""))
    return bytes(out)


def time_us(value: str | None, base: str) -> int | None:
    if value in (None, "N/A"):
        return None
    num, den = (int(x) for x in base.split("/"))
    return int(value) * 1_000_000 * num // den


def probe(ffprobe: str, path: Path) -> tuple[dict, list[dict]]:
    text = subprocess.run([ffprobe, "-v", "error", "-show_streams", "-show_packets", "-show_data", "-of", "json", str(path)],
                          check=True, capture_output=True, text=True).stdout
    doc = json.loads(text)
    if len(doc["streams"]) != 1:
        raise SystemExit(f"{path}: expected one stream, found {len(doc['streams'])}")
    return doc["streams"][0], doc["packets"]


def encode(ffmpeg: str, clip: Clip, out: Path) -> None:
    src = video_src(clip.seconds) if clip.kind == VIDEO else audio_src(clip.seconds)
    cmd = [ffmpeg, "-v", "error", "-y", *src, *clip.args, "-bitexact", "-map_metadata", "-1", "-f", clip.container, str(out)]
    subprocess.run(cmd, check=True)


def pack(clip: Clip, stream: dict, packets: list[dict]) -> bytes:
    base = stream["time_base"]
    rows = []
    for p in packets:
        pts = time_us(p.get("pts"), base)
        if pts is None:
            pts = time_us(p.get("dts"), base)
        dur = time_us(p.get("duration"), base) or 0
        data = hexdump_bytes(p["data"])
        if len(data) != int(p["size"]):
            raise SystemExit(f"{clip.name}: packet at pts {p.get('pts')} read {len(data)} bytes, ffprobe says {p['size']}")
        rows.append((pts, dur, 1 if "K" in p.get("flags", "") else 0, data))
    first = min(r[0] for r in rows)
    extradata = hexdump_bytes(stream.get("extradata", "")) if clip.codec == AV1 else b""
    if clip.kind == VIDEO:
        num, den = (int(x) for x in stream["avg_frame_rate"].split("/"))
        head = (int(stream["width"]), int(stream["height"]), num, den)
    else:
        head = (int(stream["channels"]), int(stream["sample_rate"]), int(stream.get("bit_rate") or 0), 0)
    out = bytearray(MAGIC)
    out += struct.pack("<BBH4I", clip.kind, clip.codec, clip.flags, *head)
    out += struct.pack("<I", len(extradata)) + extradata
    out += struct.pack("<I", len(rows))
    for pts, dur, key, data in rows:
        out += struct.pack("<qIB3xI", pts - first, dur, key, len(data)) + data
    return bytes(out)


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--ffmpeg", default="ffmpeg")
    ap.add_argument("--ffprobe", default="ffprobe")
    ap.add_argument("--out", type=Path, default=ROOT / "tizen" / "esprobe")
    args = ap.parse_args(argv)
    args.out.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory() as tmp:
        for clip in CLIPS:
            media = Path(tmp) / f"{clip.name}.{'ts' if clip.container == 'mpegts' else 'mkv'}"
            encode(args.ffmpeg, clip, media)
            stream, packets = probe(args.ffprobe, media)
            blob = pack(clip, stream, packets)
            (args.out / f"{clip.name}.plxes").write_bytes(blob)
            print(f"{clip.name}.plxes: {len(packets)} packets, {len(blob)} bytes")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
