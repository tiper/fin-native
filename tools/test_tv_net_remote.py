#!/usr/bin/env python3
"""Grade tools/tv-net-remote.py without a television: its frames are valid masked WebSocket text
frames, and the key list refuses anything that could start a playback (OK, PLAY, ENTER)."""
import importlib.util
from pathlib import Path
import struct
import unittest

spec = importlib.util.spec_from_file_location("tv_net_remote", Path(__file__).with_name("tv-net-remote.py"))
remote = importlib.util.module_from_spec(spec)
spec.loader.exec_module(remote)


def unmask(data):
    n, at = data[1] & 0x7F, 2
    if n == 126:
        n, at = struct.unpack(">H", data[2:4])[0], 4
    mask, body = data[at:at + 4], data[at + 4:]
    assert len(body) == n
    return bytes(b ^ mask[i % 4] for i, b in enumerate(body)).decode()


class Frames(unittest.TestCase):
    def test_a_frame_is_a_final_masked_text_frame(self):
        data = remote.frame("type:button\nname:UP\n\n", mask=b"\x01\x02\x03\x04")
        self.assertEqual(data[0], 0x81)
        self.assertTrue(data[1] & 0x80)
        self.assertEqual(unmask(data), "type:button\nname:UP\n\n")

    def test_a_long_message_uses_the_16_bit_length(self):
        text = "x" * 300
        data = remote.frame(text)
        self.assertEqual(data[1] & 0x7F, 126)
        self.assertEqual(unmask(data), text)


class Keys(unittest.TestCase):
    def test_arrows_and_back_pass(self):
        self.assertEqual(remote.checked_keys("right, left,Back"), ["RIGHT", "LEFT", "BACK"])

    def test_anything_that_could_play_is_refused(self):
        for keys in ("ENTER", "UP,OK", "PLAY", ""):
            with self.assertRaises(SystemExit, msg=keys):
                remote.checked_keys(keys)


if __name__ == "__main__":
    unittest.main()
