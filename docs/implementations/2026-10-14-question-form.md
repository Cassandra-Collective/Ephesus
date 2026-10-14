# Blank issues off, and questions get a form of their own

**Date:** 2026-10-14 · **Branch:** `docs/question-form` · **Issue:** #94

---

The owner turned blank issues off for every repository in the Cassandra Collective
organisation, so that every issue arrives through a form with its labels. Here the
chooser still allowed blank issues, and CONTRIBUTING §9 told people to "open an
issue with the `question` label", which needs a blank issue once none of the forms
fits a question.

## What changed

| File | Change |
|---|---|
| `.github/ISSUE_TEMPLATE/config.yml` | `blank_issues_enabled: false`; the contact links are unchanged |
| `.github/ISSUE_TEMPLATE/question.yml` | A new Question form: the question, what was read or tried, the commit and platform. Labels `question` and `needs-triage`, like the other forms |
| `CONTRIBUTING.md` §9 | Points at the Question form instead of a blank issue |

The form's opening note keeps §9's stance: a question the docs should have answered
is a finding about the docs.

## Evidence

- Both YAML files parse with PyYAML 6.0.3 (Python 3.13.14): `labels` is
  `['question', 'needs-triage']`, and `blank_issues_enabled` is `False`.
- Both labels exist in the repository (`gh label list`).
- No application source or test changed.

## Not covered

How the new-issue page renders the form. GitHub reads forms only from `main`, and
the page needs a signed-in account.
