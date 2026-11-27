//! The television's elementary-stream player, `libesplusplayer.so`, bound at run time (`plx_base::dynlib!`), and the
//! window lookup its display call needs.
//!
//! **Where every declaration comes from.** No header for this library ships in the Tizen SDK this repository uses, so
//! nothing here is written from a header. Each function, struct and constant is the firmware's own managed binding,
//! `Tizen.TV.Multimedia.ESPlayer.dll` (csapi-tv-multimedia 1.1.18, read from the TV 10.0 emulator image's
//! `/usr/share/dotnet.tizen/framework/`): its `Interop.NativeESPlusPlayer` P/Invokes name the symbol and the argument
//! types, its `Native*` structs give the field order, and its enums give the values the native side receives
//! unchanged. The library itself (esplusplayer 1.11.3 on that image, SONAME `libesplusplayer.so`) exports
//! `get_size_of_esplusplayer_{es_packet,video_stream_info,audio_stream_info}`, which return 64, 40 and 32 there; the
//! probe compares them with this file's layouts before it submits anything ([`layout_matches`]), so a set (armv7
//! included) whose packet or stream-info struct sizes differ from this build's is a logged refusal, never a misread
//! buffer. The library offers no
//! such size for [`MatroskaColor`] (136 bytes here), which the HDR10 packet points at: its layout is the binding's
//! `ESMatroskaColor` and is not checked against the set. Every call but one is one the binding makes: the first-frame
//! setter, which the binding does not use, sits in a table of its own (`esplus_first_frame`, whose doc says where
//! its signature comes from); `esplusplayer_flush` exists but no primary source here states its signature, so it is
//! not used.
//!
//! **The privilege.** The managed binding checks `http://developer.samsung.com/privilege/esplay` (its
//! `PrivilegeChecker`), which Samsung's TV API reference lists at the Partner level. The native library contains no
//! privilege check that a string search of it, of `libespplayer-core.so` or of `libtrackrenderer.so` finds. This
//! package does not declare the privilege; whether the native calls work without it on a retail set is exactly what
//! the probe measures.

use std::ffi::{c_char, c_int, c_void};

/// `esplusplayer_es_packet` (the binding's `NativePacket`).
#[repr(C)]
pub struct EsPacket {
    pub stream_type: c_int,
    pub buffer: *mut c_char,
    pub buffer_size: u32,
    pub pts: u64,
    pub duration: u64,
    pub matroska_color_info: *mut c_void,
    pub hdr10p_metadata_size: u32,
    pub hdr10p_metadata: *mut c_char,
}

/// `esplusplayer_video_stream_info` (the binding's `NativeVideoStreamInfo`).
#[repr(C)]
pub struct VideoStreamInfo {
    pub codec_data: *mut c_char,
    pub codec_data_length: u32,
    pub mime_type: c_int,
    pub width: u32,
    pub height: u32,
    pub max_width: u32,
    pub max_height: u32,
    pub framerate_num: u32,
    pub framerate_den: u32,
}

/// `esplusplayer_audio_stream_info` (the binding's `NativeAudioStreamInfo`).
#[repr(C)]
pub struct AudioStreamInfo {
    pub codec_data: *mut c_char,
    pub codec_data_length: u32,
    pub mime_type: c_int,
    pub bitrate: u32,
    pub channels: u32,
    pub sample_rate: u32,
}

/// The mastering display metadata inside [`MatroskaColor`] (the binding's `ESMatroskaMasteringMetadata`).
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct MasteringMetadata {
    pub primary_r_x: f64,
    pub primary_r_y: f64,
    pub primary_g_x: f64,
    pub primary_g_y: f64,
    pub primary_b_x: f64,
    pub primary_b_y: f64,
    pub white_x: f64,
    pub white_y: f64,
    pub luminance_max: f64,
    pub luminance_min: f64,
}

/// The colour description a video packet may carry (the binding's `ESMatroskaColor`, Matroska's `Colour` element:
/// the values are Matroska's, which are ISO/IEC 23091-2's code points).
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct MatroskaColor {
    pub matrix_coefficients: u32,
    pub bits_per_channel: u32,
    pub chroma_subsampling_horz: u32,
    pub chroma_subsampling_vert: u32,
    pub cb_subsampling_horz: u32,
    pub cb_subsampling_vert: u32,
    pub chroma_siting_horz: u32,
    pub chroma_siting_vert: u32,
    pub range: u32,
    pub transfer_characteristics: u32,
    pub primaries: u32,
    pub max_cll: u32,
    pub max_fall: u32,
    pub metadata: MasteringMetadata,
}

/// The HDR10 colour of the bundled `hevc10` clip: BT.2020 non-constant matrix (9), PQ transfer (16), BT.2020 primaries
/// (9), limited range (1), 10 bits, and the clip's own mastering display and light levels (P3-D65 primaries, 1000 /
/// 0.0001 cd/m², MaxCLL 1000, MaxFALL 400), as `tools/make-esprobe-clips.py` encodes them.
pub fn hdr10_colour() -> MatroskaColor {
    MatroskaColor {
        matrix_coefficients: 9,
        bits_per_channel: 10,
        range: 1,
        transfer_characteristics: 16,
        primaries: 9,
        max_cll: 1000,
        max_fall: 400,
        metadata: MasteringMetadata {
            primary_r_x: 0.680,
            primary_r_y: 0.320,
            primary_g_x: 0.265,
            primary_g_y: 0.690,
            primary_b_x: 0.150,
            primary_b_y: 0.060,
            white_x: 0.3127,
            white_y: 0.3290,
            luminance_max: 1000.0,
            luminance_min: 0.0001,
        },
        ..MatroskaColor::default()
    }
}

/// `esplusplayer_stream_type`.
pub const STREAM_AUDIO: c_int = 0;
pub const STREAM_VIDEO: c_int = 1;
/// `esplusplayer_display_type`: OVERLAY is 1 here (the capi player's OVERLAY is 0).
pub const DISPLAY_OVERLAY: c_int = 1;
/// `esplusplayer_display_mode`.
pub const DISPLAY_MODE_LETTER_BOX: c_int = 0;

/// `esplusplayer_submit_status`.
pub const SUBMIT_NOT_PREPARED: c_int = 0;
pub const SUBMIT_INVALID_PACKET: c_int = 1;
pub const SUBMIT_OUT_OF_MEMORY: c_int = 2;
pub const SUBMIT_FULL: c_int = 3;
pub const SUBMIT_SUCCESS: c_int = 4;

/// `esplusplayer_buffer_status`.
pub const BUFFER_UNDERRUN: c_int = 0;

/// `esplusplayer_video_mime_type` and `esplusplayer_audio_mime_type`, for the codecs the probe carries. The binding's
/// enums stop at the PCM formats; there is no DTS or TrueHD member in either (the library's GStreamer layer knows
/// `audio/x-dts` and `audio/x-true-hd` caps, which the public enum does not reach).
pub fn mime(codec: super::clip::Codec) -> c_int {
    use super::clip::Codec;
    match codec {
        Codec::H264 => 2,
        Codec::Hevc => 3,
        Codec::Vp9 => 8,
        Codec::Av1 => 10,
        Codec::Aac => 1,
        Codec::Ac3 => 4,
        Codec::Eac3 => 5,
        Codec::PcmS16le => 8,
    }
}

/// `esplusplayer_error_type` names for the values the binding lists (`ErrorType`), for the log.
pub fn error_name(code: c_int) -> &'static str {
    match code {
        0 => "none",
        -12 => "out_of_memory",
        -105 => "buffer_space",
        -13 => "permission_denied",
        -22 => "invalid_parameter",
        -38 => "invalid_operation",
        -26476511 => "seek_failed",
        -26476510 => "invalid_state",
        -26476509 => "not_supported_file",
        -26476506 => "connection_failed",
        -26476500 => "resource_limit",
        -26476498 => "not_supported_audio_codec",
        -26476497 => "not_supported_video_codec",
        -26472440 => "not_supported_format",
        -26472439 => "unknown",
        _ => "?",
    }
}

pub fn submit_name(status: c_int) -> &'static str {
    match status {
        SUBMIT_NOT_PREPARED => "not_prepared",
        SUBMIT_INVALID_PACKET => "invalid_packet",
        SUBMIT_OUT_OF_MEMORY => "out_of_memory",
        SUBMIT_FULL => "full",
        SUBMIT_SUCCESS => "success",
        _ => "?",
    }
}

pub type VoidCb = unsafe extern "C" fn(*mut c_void);
pub type ErrorCb = unsafe extern "C" fn(c_int, *mut c_void);
pub type BufferCb = unsafe extern "C" fn(c_int, c_int, *mut c_void);
pub type TypeCb = unsafe extern "C" fn(c_int, *mut c_void);
pub type BoolCb = unsafe extern "C" fn(bool, *mut c_void);
pub type SeekCb = unsafe extern "C" fn(c_int, u64, *mut c_void);

plx_base::dynlib! {
    /// The player: every call the probe makes, as the firmware's binding declares it.
    pub esplus: ["libesplusplayer.so"] {
        fn esplusplayer_create() -> *mut c_void;
        fn esplusplayer_destroy(h: *mut c_void) -> c_int;
        fn esplusplayer_open(h: *mut c_void) -> c_int;
        fn esplusplayer_close(h: *mut c_void) -> c_int;
        fn esplusplayer_prepare_async(h: *mut c_void) -> c_int;
        fn esplusplayer_start(h: *mut c_void) -> c_int;
        fn esplusplayer_stop(h: *mut c_void) -> c_int;
        fn esplusplayer_pause(h: *mut c_void) -> c_int;
        fn esplusplayer_resume(h: *mut c_void) -> c_int;
        fn esplusplayer_seek(h: *mut c_void, ms: u64) -> c_int;
        fn esplusplayer_set_ecore_display(h: *mut c_void, kind: c_int, window: *mut c_void, x: c_int, y: c_int, w: c_int, hgt: c_int) -> c_int;
        fn esplusplayer_set_display_mode(h: *mut c_void, mode: c_int) -> c_int;
        fn esplusplayer_get_state(h: *mut c_void) -> c_int;
        fn esplusplayer_submit_packet(h: *mut c_void, packet: *mut EsPacket) -> c_int;
        fn esplusplayer_submit_eos_packet(h: *mut c_void, stream_type: c_int) -> c_int;
        fn esplusplayer_set_audio_stream_info(h: *mut c_void, info: *mut AudioStreamInfo) -> c_int;
        fn esplusplayer_set_video_stream_info(h: *mut c_void, info: *mut VideoStreamInfo) -> c_int;
        fn esplusplayer_get_playing_time(h: *mut c_void, ms: *mut u64) -> c_int;
        fn esplusplayer_set_error_cb(h: *mut c_void, cb: ErrorCb, user: *mut c_void) -> c_int;
        fn esplusplayer_set_buffer_status_cb(h: *mut c_void, cb: BufferCb, user: *mut c_void) -> c_int;
        fn esplusplayer_set_resource_conflicted_cb(h: *mut c_void, cb: VoidCb, user: *mut c_void) -> c_int;
        fn esplusplayer_set_eos_cb(h: *mut c_void, cb: VoidCb, user: *mut c_void) -> c_int;
        fn esplusplayer_set_ready_to_prepare_cb(h: *mut c_void, cb: TypeCb, user: *mut c_void) -> c_int;
        fn esplusplayer_set_prepare_async_done_cb(h: *mut c_void, cb: BoolCb, user: *mut c_void) -> c_int;
        fn esplusplayer_set_seek_done_cb(h: *mut c_void, cb: VoidCb, user: *mut c_void) -> c_int;
        fn esplusplayer_set_ready_to_seek_cb(h: *mut c_void, cb: SeekCb, user: *mut c_void) -> c_int;
    }
}

plx_base::dynlib! {
    /// The library's own sizes of the three structs the probe hands it. A table of its own: a firmware without them is
    /// a refusal of the probe, not of the player.
    pub esplus_sizes: ["libesplusplayer.so"] {
        fn get_size_of_esplusplayer_es_packet() -> c_int;
        fn get_size_of_esplusplayer_video_stream_info() -> c_int;
        fn get_size_of_esplusplayer_audio_stream_info() -> c_int;
    }
}

plx_base::dynlib! {
    /// The first-decoded-picture callback. The managed binding does not declare it, so its own table: a firmware without
    /// it loses the first-frame step, not the player. The signature is read from the TV 10.0 emulator's library itself:
    /// the setter's prologue keeps `rdi`, `rsi` and `rdx` exactly as `esplusplayer_set_eos_cb`'s does (handle, callback,
    /// user data), and the library's own trampoline is `listener_bridge::OnFirstDecodingDone(void*)`, the shape of
    /// `listener_bridge::OnEos(void*)` behind the binding's `void (*)(void*)` end-of-stream callback.
    pub esplus_first_frame: ["libesplusplayer.so"] {
        fn esplusplayer_set_first_video_decoding_done_cb(h: *mut c_void, cb: VoidCb, user: *mut c_void) -> c_int;
    }
}

plx_base::dynlib! {
    /// The library's window, found the way the host finds it for the URI player (`PlayerNative.cs`): the connected
    /// display, then window id 1 (the library makes exactly one window). HOST THREAD.
    pub ecore_find: ["libecore_wl2.so.1"] {
        fn ecore_wl2_connected_display_get(name: *const c_char) -> *mut c_void;
        fn ecore_wl2_display_window_find(display: *mut c_void, id: u32) -> *mut c_void;
    }
}

/// This build's sizes of the three structs, in the order the library's `get_size_of_*` report them.
pub fn compiled_sizes() -> [usize; 3] {
    [std::mem::size_of::<EsPacket>(), std::mem::size_of::<VideoStreamInfo>(), std::mem::size_of::<AudioStreamInfo>()]
}

/// Whether the library's sizes are this build's, exactly. A larger struct on the library's side could be read past
/// the end of ours, a smaller one means fields moved: both refuse.
pub fn layout_matches(native: [i64; 3]) -> bool {
    native.iter().zip(compiled_sizes()).all(|(n, c)| *n >= 0 && *n as usize == c)
}
