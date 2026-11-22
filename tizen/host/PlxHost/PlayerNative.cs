// The device's media player library, as the host calls it: the C API of libcapi-media-player (Tizen Native `player.h`).
//
// WHY P/INVOKE AND NOT Tizen.Multimedia.Player. The window the library draws into is an Ecore_Wl2_Window* that libplx.so made itself
// (rust-modules/platform-tizen/src/ecore.rs, `ecore_wl2_window_new`), and at TFM tizen80 (TizenFX API8) the managed `Display` class has only
// three constructors, none of which takes a raw window: `Display(MediaView)`, `Display(ElmSharp.Window)` and `Display(NUI.Window)`. The NUI one
// is the one that reaches an Ecore window, and it does so by calling `player_set_ecore_wl_display(handle, OVERLAY, window, x, y, w, h)`. This
// file makes that same call with our window. TizenFX is a thin wrapper over these entry points, so it is also the reference for the call order
// and the error codes (https://github.com/Samsung/TizenFX, branch API8: src/Tizen.Multimedia.MediaPlayer/Interop/Interop.Player.cs,
// Interop.Display.cs, Player/Player.Properties.cs `ApplyEcoreWindow`). The same library is what the field project nuvio's tizen-tpk/Video.cs
// P/Invokes (`player_set_display_mode`). MEASURED on the emulator 2026-11-22 (TV 10.0 x86_64, a probe on a scratch branch, a local clip): this
// whole chain returns PLAYER_ERROR_NONE on every call; prepare_async completes in 250-520 ms; the play position advances in real time; the
// completed callback fires at the end with the native state still PLAYING until player_stop (so the host does not read the state to validate
// a completion); a player with NO display does not advance, so the display is bound before prepare. UNVERIFIED on a Samsung set
// (docs/tizen.md, "The media player").
//
// The library is the device's own, opened by name at run time. It is not in the package (tizen/package.py ALLOWED_BIN/ALLOWED_LIB are
// unchanged) and THIRD-PARTY-NOTICES.md section 2.8 lists it with the other libraries opened by name.
//
// Every int return is a player_error_e: 0 is PLAYER_ERROR_NONE. `player_h` is a pointer (IntPtr). A C99 `bool` argument is passed as an int,
// which is what the register holds on both x86_64 and ARM.

using System;
using System.Runtime.InteropServices;

namespace PlxHost
{
    internal static class MediaPlayerNative
    {
        internal const string MediaLib = "libcapi-media-player.so.0";

        // The Ecore side, to find the window libplx.so made (the library is the one libplx.so itself opens by name; THIRD-PARTY-NOTICES.md 2.8).
        internal const string EcoreLib = "libecore_wl2.so.1";

        // Ecore_Wl2_Display *ecore_wl2_connected_display_get(const char *name); NULL name = the default display (the one libplx connected).
        [DllImport(EcoreLib, CallingConvention = CallingConvention.Cdecl)]
        internal static extern IntPtr ecore_wl2_connected_display_get(IntPtr name);

        // Ecore_Wl2_Window *ecore_wl2_display_window_find(Ecore_Wl2_Display *display, unsigned int id).
        [DllImport(EcoreLib, CallingConvention = CallingConvention.Cdecl)]
        internal static extern IntPtr ecore_wl2_display_window_find(IntPtr display, uint id);

        // player_display_type_e: PLAYER_DISPLAY_TYPE_OVERLAY.
        internal const int DISPLAY_TYPE_OVERLAY = 0;

        // player_display_mode_e: PLAYER_DISPLAY_MODE_LETTER_BOX.
        internal const int DISPLAY_MODE_LETTER_BOX = 0;

        // void (*)(void *user_data): prepared, completed and seek-completed.
        [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
        internal delegate void VoidCallback(IntPtr userData);

        // void (*)(int value, void *user_data): error (error code), buffering (percent) and interrupted (interrupted code).
        [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
        internal delegate void IntCallback(int value, IntPtr userData);

        [DllImport(MediaLib, CallingConvention = CallingConvention.Cdecl)]
        internal static extern int player_create(out IntPtr player);

        [DllImport(MediaLib, CallingConvention = CallingConvention.Cdecl)]
        internal static extern int player_destroy(IntPtr player);

        [DllImport(MediaLib, CallingConvention = CallingConvention.Cdecl)]
        internal static extern int player_set_uri(IntPtr player, [MarshalAs(UnmanagedType.LPUTF8Str)] string uri);

        // player_set_ecore_wl_display(player, type, Ecore_Wl2_Window *window, x, y, w, h). The window is the one libplx.so made, found with the two calls above.
        [DllImport(MediaLib, CallingConvention = CallingConvention.Cdecl)]
        internal static extern int player_set_ecore_wl_display(IntPtr player, int type, IntPtr ecoreWindow, int x, int y, int width, int height);

        [DllImport(MediaLib, CallingConvention = CallingConvention.Cdecl)]
        internal static extern int player_set_display_mode(IntPtr player, int mode);

        [DllImport(MediaLib, CallingConvention = CallingConvention.Cdecl)]
        internal static extern int player_prepare_async(IntPtr player, VoidCallback prepared, IntPtr userData);

        [DllImport(MediaLib, CallingConvention = CallingConvention.Cdecl)]
        internal static extern int player_unprepare(IntPtr player);

        [DllImport(MediaLib, CallingConvention = CallingConvention.Cdecl)]
        internal static extern int player_start(IntPtr player);

        [DllImport(MediaLib, CallingConvention = CallingConvention.Cdecl)]
        internal static extern int player_pause(IntPtr player);

        [DllImport(MediaLib, CallingConvention = CallingConvention.Cdecl)]
        internal static extern int player_stop(IntPtr player);

        // player_set_play_position(player, int millisecond, bool accurate, player_seek_completed_cb callback, void *user_data)
        [DllImport(MediaLib, CallingConvention = CallingConvention.Cdecl)]
        internal static extern int player_set_play_position(IntPtr player, int millisecond, int accurate, VoidCallback seekCompleted, IntPtr userData);

        [DllImport(MediaLib, CallingConvention = CallingConvention.Cdecl)]
        internal static extern int player_get_play_position(IntPtr player, out int millisecond);

        [DllImport(MediaLib, CallingConvention = CallingConvention.Cdecl)]
        internal static extern int player_get_duration(IntPtr player, out int duration);

        [DllImport(MediaLib, CallingConvention = CallingConvention.Cdecl)]
        internal static extern int player_get_video_size(IntPtr player, out int width, out int height);

        [DllImport(MediaLib, CallingConvention = CallingConvention.Cdecl)]
        internal static extern int player_set_completed_cb(IntPtr player, VoidCallback completed, IntPtr userData);

        [DllImport(MediaLib, CallingConvention = CallingConvention.Cdecl)]
        internal static extern int player_set_error_cb(IntPtr player, IntCallback error, IntPtr userData);

        [DllImport(MediaLib, CallingConvention = CallingConvention.Cdecl)]
        internal static extern int player_set_buffering_cb(IntPtr player, IntCallback buffering, IntPtr userData);

        [DllImport(MediaLib, CallingConvention = CallingConvention.Cdecl)]
        internal static extern int player_set_interrupted_cb(IntPtr player, IntCallback interrupted, IntPtr userData);
    }
}
