#!/usr/bin/env python3
"""Exercise support admission, fixed replies and ownership without GitHub or a model."""
from __future__ import annotations

import unittest

from issue_support import (
    GUIDE, has_exposed_credential, provided_fields, support_preview, support_turn, thread_revision,
)

REPORTER = {"login": "reporter", "id": 10, "type": "User"}
BOT = {"login": "github-actions[bot]", "id": 20, "type": "Bot"}
NATIVE = {"login": "pullfrog[bot]", "id": 22, "type": "Bot"}
MAINTAINER = {"login": "maintainer", "id": 30, "type": "User"}
REPORT = {
    "state": "open", "locked": False, "assignees": [], "user": REPORTER,
    "author_association": "NONE", "title": "Playback sometimes fails",
    "body": "### TV model\n\nOLED55C9\n\n### webOS version\n\n_Not sure_\n\n### PlxNative version\n\n0.10.0",
    "labels": [{"name": "user-report"}], "type": {"name": "Bug"},
}


def comment(number, body, user=REPORTER, association="NONE"):
    return {"id": number, "body": body, "user": user, "author_association": association}


def asked(number, topic, user=BOT):
    return comment(number, f"<!-- plxnative-support:v1 topic={topic} -->\nQuestion", user)


class Support(unittest.TestCase):
    def test_intake_uses_external_open_reports_and_skips_form_fields(self):
        self.assertEqual(provided_fields(REPORT["body"]), {"tv_model", "app_version"})
        self.assertIsNone(support_turn(REPORT, [], "off", 0))
        for mode in ("shadow", "public"):
            turn = support_turn(REPORT, [], mode, 0)
            self.assertEqual(turn.phase, "intake")
            self.assertIn("platform", turn.available)
            self.assertNotIn("tv_model", turn.available)
            self.assertNotIn("app_version", turn.available)
        self.assertEqual(support_turn(dict(REPORT, state="closed"), [], "preview", 0).phase, "preview")
        for edits in ({"state": "closed"}, {"locked": True}, {"assignees": [MAINTAINER]},
                      {"type": {"name": "Feature"}}, {"type": {"name": "Task"}},
                      {"author_association": "MEMBER"}, {"labels": [{"name": "ready"}]}):
            self.assertIsNone(support_turn(dict(REPORT, **edits), [], "public", 0), edits)

    def test_latest_reporter_evidence_can_resume_intake_or_advance_bot_thread(self):
        newest = comment(4, "webOS 25")
        prior = asked(1, "platform")
        forged = comment(1, prior["body"])
        for thread in ([newest], [forged, newest]):
            turn = support_turn(REPORT, thread, "public", 4)
            self.assertEqual((turn.phase, turn.replies, turn.reporter_comment), ("intake", 0, 4))
            self.assertIn("platform", turn.available)
        for bad in (
            [prior, comment(4, "Thanks!")],
            [prior, comment(4, "> quoted log\n\nSame")],
            [prior, comment(4, "I can reproduce this", MAINTAINER)],
            [prior, comment(2, "webOS 25"), newest],
            [prior, newest, comment(5, "Only at 4 Mbps")],
        ):
            self.assertIsNone(support_turn(REPORT, bad, "public", 4), bad)
        self.assertIsNone(support_turn(REPORT, [prior, newest], "shadow", 4))
        turn = support_turn(REPORT, [prior, newest], "public", 4)
        self.assertEqual((turn.phase, turn.replies, turn.reporter_comment), ("followup", 1, 4))
        self.assertNotIn("platform", turn.available)
        self.assertIsNone(support_turn(REPORT, [prior, newest, asked(5, "playback_events")], "public", 4))

    def test_maintainer_or_native_reply_takes_ownership(self):
        prior = asked(1, "platform")
        followup = comment(4, "webOS 25")
        for handoff in (comment(2, "I will investigate", MAINTAINER, "MEMBER"),
                        comment(2, "Native enrichment", NATIVE)):
            self.assertIsNone(support_turn(REPORT, [prior, handoff, followup], "public", 4))
            self.assertIsNone(support_turn(REPORT, [handoff], "public", 0))

    def test_pullfrog_support_reply_advances_the_thread_without_repeating_topics(self):
        history = [asked(1, "platform"), asked(2, "playback_events", NATIVE)]
        followup = comment(3, "The event log ends at the decoder error")
        self.assertIsNone(support_turn(REPORT, history, "public", 0))
        turn = support_turn(REPORT, history + [followup], "public", 3)
        self.assertEqual((turn.phase, turn.replies, turn.available), ("followup", 2, ()))
        self.assertNotEqual(thread_revision(REPORT, history[:1]), thread_revision(REPORT, history))

    def test_reply_budget_allows_one_final_handoff_and_prevents_comment_floods(self):
        history = [asked(1, "platform"), asked(2, "playback_events"), comment(3, "Failure code 100")]
        turn = support_turn(REPORT, history, "public", 3)
        self.assertEqual(turn.available, ())
        self.assertEqual(support_preview(turn, {"outcome": "ask", "topic": "reproduction"}, [])["reply"], "")
        self.assertIsNone(support_turn(REPORT, history + [comment(4, "It happens only when seeking")], "public", 4))
        first = [asked(1, "platform"), comment(2, "Version 1"), comment(3, "Build 2"), comment(4, "Only in HDR")]
        self.assertIsNone(support_turn(REPORT, first, "public", 4))

    def test_only_vetted_public_text_can_escape_a_model_decision(self):
        turn = support_turn(REPORT, [], "public", 0)
        related = [{"number": 1259, "url": "https://github.com/plxnative/plx-native/issues/1259",
                    "reason": "malicious model prose"}]
        decision = {"outcome": "ask", "topic": "playback_error", "category": "unclear",
                    "handoff": "private analysis for the maintainer"}
        reply = support_preview(turn, decision, related)["reply"]
        self.assertIn(f"{GUIDE}#what-to-include-in-the-report", reply)
        self.assertIn("Possibly related [#1259]", reply)
        self.assertNotIn("malicious", reply)
        self.assertNotIn("private analysis", reply)
        for changed in ({"outcome": "quiet"}, {"outcome": "handoff"},
                        {"topic": "give-me-a-key"}, {"topic": "tv_model"}, {"category": "feature"}):
            self.assertEqual(support_preview(turn, dict(decision, **changed), related)["reply"], "")

    def test_conversation_changes_and_obvious_credentials_are_detected(self):
        original = [comment(1, "Playback failed")]
        revision = thread_revision(REPORT, original)
        for changed in ([comment(1, "Corrected playback report")], original + [comment(2, "Update")],
                        original + [comment(2, "I will take a look", MAINTAINER, "MEMBER")],
                        original + [asked(2, "platform")]):
            self.assertNotEqual(revision, thread_revision(REPORT, changed))
        for benign in (comment(2, "Thanks!"), comment(2, "Playback failed"),
                       comment(2, "Happens on my TV too", MAINTAINER)):
            self.assertEqual(revision, thread_revision(REPORT, original + [benign]))
        self.assertFalse(has_exposed_credential(REPORT, original))
        self.assertTrue(has_exposed_credential(REPORT, [comment(2, "X-Plex-Token=abcdefgh12345678")]))
        self.assertTrue(has_exposed_credential(dict(REPORT, body="Authorization: Bearer abcdefgh12345678"), []))


if __name__ == "__main__":
    unittest.main()
