# Questions go to Discussions, and the Question form stays for what should be tracked

**Date:** 2026-10-15 · **Branch:** `docs/discussions-qa` · **Issue:** #98

---

On 2026-10-15 the owner turned on GitHub Discussions for every repository in the
Cassandra Collective organisation. Ephesus got the default categories, among them
Q&A — the one category where the answer that settles a thread can be marked as
accepted. Nothing pointed there. The new-issue page had no questions link at all, and
CONTRIBUTING §9 sent every question to the Question form, so a question that only
needed an answer still became an issue for a maintainer to triage and close, and the
answer was buried in the tracker.

Questions now go to Discussions Q&A first. The Question form stays, for the question
that should be tracked: the one whose answer looks like a bug, or like something the
docs ought to say.

## What changed

| File | Change |
|---|---|
| `.github/ISSUE_TEMPLATE/config.yml` | A fourth contact link, "Ask a question", pointing at `https://github.com/Cassandra-Collective/Ephesus/discussions/categories/q-a`. The three existing links and `blank_issues_enabled: false` are unchanged |
| `CONTRIBUTING.md` §9 | Points at Discussions Q&A first, and at the Question form for a question that should be tracked as an issue. The closing stance stays: a question the docs should have answered is a finding about the docs |

The new link goes last, after "Read the design first" and "Where the build currently
stands". Those two are still the right first stops — most "why is it like this"
questions are answered in the docs — so asking comes after them on the page, as it
does in §9's own advice.

## What deliberately did not change

- **The Question form.** `.github/ISSUE_TEMPLATE/question.yml` keeps its fields and
  its `question` and `needs-triage` labels. Some questions are bug reports or
  documentation gaps in disguise, and with blank issues off the form is the only way
  to put a question in the tracker.
- **No code of conduct link.** The chooser never had one; Ephesus keeps its own
  `CODE_OF_CONDUCT.md`, which CONTRIBUTING links in its first lines. Adding one is a
  separate question from where questions go.
- **No entry in `docs/DECISIONS-LOG.md`.** The log records mechanical choices made
  when the docs do not answer (BUILD-PROMPT §8.2) and the Architect's decisions on
  Ephesus's own work. Turning Discussions on was the owner's decision for the whole
  organisation, taken outside this repository, and the closest precedent — turning
  blank issues off for every repository, #94 and #95 — added no entry either. This
  record is where the change is written down.
- **No repository setting** is changed by the pull request; Discussions was already on.

## Evidence

- `config.yml` parses with PyYAML 6.0.3 (Python 3.13.14): its keys are
  `blank_issues_enabled` and `contact_links`, `blank_issues_enabled` is `False`, and
  the four links (`Security vulnerability`, `Read the design first`,
  `Where the build currently stands`, `Ask a question`) each carry exactly `name`,
  `url` and `about`.
- `curl -s -o /dev/null -w "%{http_code}"` on the Q&A URL prints `200`, signed out.
- `node scripts/check-attribution.cjs --pending` on the commit message passes.
- No application source or test changed, so typecheck, lint and tests are
  unaffected; CI runs them on the pull request.

## Not covered

How the new-issue page renders the fourth link. GitHub reads `config.yml` only from
`main`, and the page needs a signed-in account. After merging, "New issue" should show
the four forms and the four links, with "Ask a question" last.
