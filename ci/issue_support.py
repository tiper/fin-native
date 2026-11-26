"""Eligibility, reply history, and fixed public diagnostics for PlxNative issue support."""
from __future__ import annotations

import hashlib
import json
import re
from dataclasses import dataclass

# A diagnosis is chosen from these checked-in questions and safe, documented checks.
GUIDE = "https://plxnative.com/troubleshooting/"
TOPICS = {
    "tv_model": "What is the exact TV model on the sticker or in **Settings → About This TV**?",
    "platform": "Which TV platform and version are you running? On LG, look in **Settings → About This TV**.",
    "app_version": "Which PlxNative version are you running, including the build number if it is a nightly?",
    "install_method": "Did you install PlxNative with Dev Manager, Homebrew Channel, or as a nightly?",
    "reproduction": "What is the shortest sequence of steps from opening PlxNative to seeing this happen?",
    "home_row": "Which Home row does this happen on, and does it move while you use the remote or after you let go?",
    "library_startup": "During a slow launch, is Home still loading, or do the rows appear with blank posters?",
    "playback_error": f"Could you copy the single line at the bottom of the playback error screen? It starts with **PlxNative**. [Where to find it]({GUIDE}#what-to-include-in-the-report)",
    "playback_quality": "Does the same item fail on Direct Play and on a selected quality? If you only tried one, which was it?",
    "playback_scope": "Does this happen with one item or several, and does it fail before playback begins or after video has started?",
    "seek_or_resume": "Does the same item play from the beginning, and does the failure start only after seeking or resuming?",
    "playback_events": f"Could you share the relevant redacted lines from the event log, around when playback failed? [Collect the log before reopening the app]({GUIDE}#collecting-logs). Leave out tokens, addresses and private identifiers.",
    "crash_logs": f"If PlxNative exits on its own, could you share the relevant redacted lines from the crash and event logs? [Collect the logs before reopening the app]({GUIDE}#collecting-logs). Leave out tokens, addresses and private identifiers.",
    "sign_in_error": "What exact error does the sign-in screen show? Please leave out any PIN, sign-in code, account details or token.",
    "sign_in_report": f"On the sign-in failure screen, could you open **Details** and share the **Report ID** if one is shown? [Finding the Report ID]({GUIDE}#cant-sign-in-or-no-servers-are-found)",
    "tv_clock": f"If the sign-in screen says the TV could not make a secure connection, could you check the date and time in the TV settings and say whether correcting them helps? [Sign-in troubleshooting]({GUIDE}#cant-sign-in-or-no-servers-are-found)",
    "server_scope": "Is the Plex server on the same network as the TV or remote, and can another Plex client reach it? Please leave out server addresses and account details.",
    "subtitle_track": "Which subtitle format and language are selected, and does the same problem occur with a different track?",
    "sound_output": "Does this also happen with the TV speakers, or only through a soundbar or receiver? If you use HDMI (e)ARC, is Digital Sound Out set to PCM, Auto, or Pass Through?",
}
FORM_FIELDS = {
    "tv_model": "TV model",
    "platform": "webOS version",
    "app_version": "PlxNative version",
    "install_method": "How you installed PlxNative",
}
OPERATOR_ASSOCIATIONS = {"OWNER", "MEMBER", "COLLABORATOR"}
# The support state of a thread is two labels. A maintainer removes HANDOFF_LABEL to give it back.
WAITING_LABEL = "support:waiting-reporter"
HANDOFF_LABEL = "support:handoff"
HUMAN_HANDOFF_LABELS = {"ready", "needs-info", "duplicate", "invalid", "wontfix", HANDOFF_LABEL}
MAX_SUPPORT_REPLIES = 2
MAX_REPORTER_UPDATES = 2
MARKER = re.compile(r"^<!-- plxnative-support:v1 topic=([a-z_]+) -->\n")
ACKNOWLEDGEMENTS = {"thanks", "thank you", "ok", "okay", "great", "same", "me too", "+1"}
EXPOSED_CREDENTIAL = re.compile(
    r"(?i)\b(?:x-plex-token|authorization|access_token|api_key)\s*[:=]\s*"
    r"(?:(?:bearer|basic)\s+)?[a-z0-9_./+-]{12,}"
)


@dataclass(frozen=True)
class Turn:
    phase: str
    available: tuple[str, ...]
    replies: int
    reporter_comment: int = 0


def actor(comment):
    return comment.get("user") or {}


def bot_topic(comment):
    user = actor(comment)
    if user.get("type") != "Bot" or user.get("login") not in {"github-actions[bot]", "pullfrog[bot]"}:
        return None
    match = MARKER.match(comment.get("body") or "")
    # A topic renamed or retired since the reply was posted is still that bot's support reply.
    return match[1] if match else None


def provided_fields(body):
    sections = re.split(r"^### (.+?)\s*$", body or "", flags=re.M)
    values = dict(zip(sections[1::2], sections[2::2]))
    placeholders = {"", "no response", "not sure", "other or not sure", "unknown", "n/a", "?"}
    return {topic for topic, heading in FORM_FIELDS.items()
            if values.get(heading, "").strip().strip("_*").lower() not in placeholders}


def meaningful_reply(comment, reporter, seen):
    if not reporter or actor(comment).get("id") != reporter or actor(comment).get("type") != "User":
        return False
    lines = (line.strip() for line in (comment.get("body") or "").splitlines())
    body = " ".join(line for line in lines if line and not line.startswith(">"))
    body = " ".join(body.casefold().split()).strip(" .!?")
    if not body or body in ACKNOWLEDGEMENTS or body in seen:
        return False
    seen.add(body)
    return True


def label_names(issue):
    return {label["name"] for label in issue.get("labels", [])}


def hands_off(comment, reporter):
    commenter = actor(comment)
    return ((commenter.get("type") == "Bot" and commenter.get("login") == "pullfrog[bot]"
             and bot_topic(comment) is None)
            or (commenter.get("type") == "User" and commenter.get("id") != reporter
                and comment.get("author_association") in OPERATOR_ASSOCIATIONS))


def human_owned(issue, comments):
    """A maintainer, assignment, lock or human label has taken the thread."""
    reporter = (issue.get("user") or {}).get("id")
    return bool(issue.get("locked") or issue.get("assignees")
                or HUMAN_HANDOFF_LABELS.intersection(label_names(issue))
                or any(hands_off(comment, reporter) for comment in comments))


def support_turn(issue, comments, mode, comment_id):
    """Return the eligible turn of this thread, independent of the model's answer.

    An `exhausted` turn means a reporter comment arrived after the reply budget ran out. It
    needs no model: the caller hands the thread to a human instead of going silent.
    """
    if mode == "off":
        return None
    history = [topic for comment in comments if (topic := bot_topic(comment))]
    already_known = provided_fields(issue.get("body")).union(history)
    available = tuple(topic for topic in TOPICS if topic not in already_known)
    if mode == "preview":
        return Turn("preview", available, len(history))
    user = issue.get("user") or {}
    issue_type = (issue.get("type") or {}).get("name")
    if (issue.get("state") != "open" or user.get("type") != "User"
            or issue.get("author_association") in OPERATOR_ASSOCIATIONS
            or issue_type in {"Feature", "Task"} or human_owned(issue, comments)):
        return None
    reporter = user.get("id")
    if not comment_id:
        return Turn("intake", available, 0) if not history else None
    if mode not in {"shadow", "public"}:
        return None

    seen = set()
    replies = []
    for comment in comments:
        if bot_topic(comment):
            # An answer is only a repeat within one question: "No" may answer the next one too.
            replies.clear()
            seen.clear()
        elif meaningful_reply(comment, reporter, seen):
            replies.append(comment["id"])
    if not replies or replies[-1] != comment_id:
        return None
    if len(replies) > MAX_REPORTER_UPDATES:
        return Turn("exhausted", (), len(history), comment_id)
    if len(history) >= MAX_SUPPORT_REPLIES:
        # One final update can produce a handoff summary after the last public question.
        if len(replies) != 1:
            return Turn("exhausted", (), len(history), comment_id)
        available = ()
    return Turn("followup" if history else "intake", available, len(history), comment_id)


def thread_revision(issue, comments):
    reporter = (issue.get("user") or {}).get("id")
    seen = set()
    data = [(comment["id"], comment.get("body")) for comment in comments
            if bot_topic(comment) or hands_off(comment, reporter) or meaningful_reply(comment, reporter, seen)]
    return hashlib.sha256(json.dumps(data).encode()).hexdigest()


def has_exposed_credential(issue, comments):
    bodies = [issue.get("title"), issue.get("body"), *(comment.get("body") for comment in comments)]
    return any(EXPOSED_CREDENTIAL.search(body or "") for body in bodies)


def support_state_change(issue, comments, turn, decision, reply):
    """The label transition this thread's support state calls for, or None.

    `reply` is the public text this turn would post. Only public mode writes the transition;
    shadow and preview report it.
    """
    labels = label_names(issue)
    if HANDOFF_LABEL in labels:
        return None
    waiting = WAITING_LABEL in labels

    def handoff(reason):
        return {"add": [HANDOFF_LABEL], "remove": [WAITING_LABEL], "reason": reason}

    # A thread the bot has worked keeps a human signal: after a handoff removed `waiting`, a maintainer
    # who removes HANDOFF_LABEL while their comment still stands gets it back. Threads the bot never
    # touched (no label, no marker) are left to their maintainers.
    worked = waiting or any(bot_topic(comment) for comment in comments)
    if worked and human_owned(issue, comments):
        return handoff("human_owned")
    if turn is None:
        return None
    if turn.phase == "exhausted":
        return handoff("exhausted")
    if decision.get("outcome") == "handoff":
        return handoff("handoff")
    if reply and not waiting:
        return {"add": [WAITING_LABEL], "remove": [], "reason": "reply"}
    return None


def support_preview(turn, decision, related):
    if turn is None:
        return None
    topic = decision.get("topic")
    reply = ""
    if (decision.get("outcome") in {"ask", "check"} and topic in turn.available
            and decision.get("category") != "feature"):
        reply = f"<!-- plxnative-support:v1 topic={topic} -->\n{TOPICS[topic]}"
        if related:
            match = related[0]
            reply += f"\n\nPossibly related [#{match['number']}]({match['url']})."
        reply += "\n\n_PlxNative support assistant_"
    return {"phase": turn.phase, "outcome": decision.get("outcome", "quiet"),
            "category": decision.get("category", "unclear"), "topic": topic,
            "prior_replies": turn.replies, "handoff": decision.get("handoff"),
            "reply": reply}


def support_instructions(turn):
    if turn is None:
        return 'This run only handles issue labels. Return related: [] and support: null.'
    topics = "\n".join(f"- {topic}: {TOPICS[topic]}" for topic in turn.available) or "(none)"
    return f"""First-line support phase: {turn.phase}. A public reply, if allowed, is rendered from a checked-in topic.
Read the report and conversation before deciding. The relevant reporter comment ID is {turn.reporter_comment or 'not specified'}.
Check docs/troubleshooting.md, docs/compatibility.md and relevant implementation or the reported release when needed.
A simulator or another TV does not establish what this firmware does. Use docs/playback-diagnostics.md for
specialized playback evidence. A playback failure screen cannot send a sign-in Report ID.
Classify the case as app_bug, platform_limit, configuration, feature, or unclear. Treat causal guesses as hypotheses.
For one missing fact choose ask. For one concrete, safe diagnostic check choose check. Pick one topic
from the list and only if the report and conversation do not already answer it. If the reporter
cannot supply it, choose an easier alternative or hand off. If the evidence is
sufficient for a maintainer to investigate, choose handoff. Choose quiet for thanks, weak matches,
feature requests, sensitive reports or whenever replying would add no value. If the symptoms
have already been split into dedicated issues, leave those diagnostics to the dedicated threads.
No public message is sent for handoff or quiet. The handoff field can contain one short observation
and the most useful next engineering check, with a GitHub issue/PR number or repo path as evidence.
If the source does not record a test result, describe the gap in evidence rather than asserting
the test was never run. Use null when there is no useful handoff.
Treat an already attached screenshot or log as supplied evidence. If you cannot interpret it, hand off
instead of asking for the same attachment again. Keep raw reports and identifiers out of the summary.
Never suggest rooting, changing security controls, deleting data, posting credentials or dumping unredacted logs.
Do not tell a reporter which implementation or feature will ship, assign blame, or close an issue.
Return support with outcome, category, topic (null for handoff or quiet), and handoff.
Available diagnostic topics:
{topics}
"""
