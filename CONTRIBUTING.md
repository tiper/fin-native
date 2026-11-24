# Contributing

## Issues and discussions

Use the [bug report form](https://github.com/plxnative/plx-native/issues/new?template=bug_report.yml)
when something breaks. Share feature ideas in [Discussions](https://github.com/plxnative/plx-native/discussions/categories/ideas)
and setup questions in [Q&A](https://github.com/plxnative/plx-native/discussions/categories/q-a).

When filing an issue as a maintainer, set its GitHub Type in the sidebar or with `gh issue create --type`.
Use **Bug** for broken behavior, **Feature** for an accepted addition or improvement, and **Task**
for maintenance, verification, investigations, or tracking work. Leave unclear reports untyped
until triage.

Use GitHub's issue Status, where available, to track progress. New issues start in **Backlog**.
Move work to **Todo** when it is ready to pick up. GitHub can move an issue to **In progress** when
an open pull request is linked, and sets the final status when the issue is closed.

Use labels for areas such as `playback` and `subtitles`, or signals such as `user-report` and
`needs-info`. The [issue-label workflow](.github/workflows/issue-labels.yml) adds approved area
labels and `user-report`. Use `is:issue type:Bug` or `is:issue type:Feature` in new saved searches.

If the PlxNative support assistant asks for details, reply in the same issue. Share the requested
diagnostic only, with tokens, addresses and private identifiers removed. The
[troubleshooting guide](docs/troubleshooting.md#collecting-logs) explains which log to collect.

### Maintainer support pilot

The Issue labels workflow calls Pullfrog once per eligible turn on GitHub-hosted runners, with
one retry on failure. The same run can classify the issue and select a diagnostic. The pinned
action gets a read-only GitHub token, no provider secrets and no shell. Issue text and comments
reach the model as GitHub tool results, which can include media hosted by GitHub. A separate
Python step applies labels and sends public replies as `pullfrog[bot]` with a short-lived
Pullfrog App token. If that token is unavailable, pending writes are withheld and the run
reports the failure in Actions.

- In the [Pullfrog console](https://pullfrog.com/console), turn off **Enrich issues** and
  **Label issues** so the workflow owns intake. Keep **Review PRs**, **Address reviews** and
  **Fix CI failures** off for the support pilot. Prefer **No code pushes** and keep mentions
  from non-collaborators off. Discussions stay with maintainers.
- Set the GitHub repository variable `PLXNATIVE_ISSUE_SUPPORT` to `shadow` for **shadow** mode.
  This is also the workflow default.
  Area labels apply normally. Support decisions, related issues and proposed replies appear in
  the Actions summary. That summary is visible on the public repository.
- After reviewing the shadow results, set the variable to `public` to allow replies, or
  `off` for labels alone. Manually dispatch **Issue labels** with issue numbers and dry run
  selected to preview a case, including a closed one. Manual backfills never send support
  replies. The supported topics and their exact public wording live in
  [`ci/issue_support.py`](ci/issue_support.py).
- Public replies are limited to two per issue and use checked-in diagnostic text. The original
  reporter's new replies can advance the thread. A maintainer reply, assignment, lock, close,
  Pullfrog comment outside this support workflow, or a maintainer's `ready` or `needs-info`
  label hands off to a human. Maintainers own Type, Status, duplicate handling, and closure.

To evaluate the pilot, record the number of **maintainer-account requests for missing diagnostics
before investigation** per report. Compare the next ten eligible reports with ten recent reports
that had a maintainer response. Also count repeated questions, corrections a maintainer had to
post, and cases that reached a usable handoff. Check the Actions decision against the issue
thread. Keep public replies enabled only if the maintainer requests fall without more repetitions
or corrections.

## Code and translations

New contributions to PlxNative's own code are offered under GPL-3.0-or-later, without copyright
assignment; to `rust-modules/owp/` (`plx_owp`), under Apache-2.0, as `LICENSING.md` says. Retain existing third-party notices and identify external code, its source revision,
license and modifications. Do not submit code whose redistribution rights you cannot establish.

Please include a `Signed-off-by: Your Name <email>` trailer certifying the
[Developer Certificate of Origin 1.1](https://developercertificate.org/). This records your right
to contribute; it is not a transfer of copyright or a requirement to re-sign historical MIT work.
Use the project verification workflow in `AGENTS.md`, and preserve unrelated changes.

Translations: see `docs/localization.md`. A new or changed language is merged once a native speaker
has reviewed it; the pull request should say who, and screenshots of the UI help them judge it.
