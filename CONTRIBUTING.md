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
