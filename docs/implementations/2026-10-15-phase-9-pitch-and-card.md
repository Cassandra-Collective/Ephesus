# The Phase 9 pitch reaches the public surfaces, and the card is rebuilt

**Date:** 2026-10-15 · **Branch:** `docs/phase-9-pitch` · **Issues:** #96, #80

---

Ephesus's front door still described the project as it was before PHASE-9-PLAN:
"An agent company you govern as its architect". The plan's direction of 2026-10-03 keeps
the brand core and moves Ephesus toward a real agentic harness, whose difference is that
its agents are governed and its harness is measured. On 2026-10-15 the Architect chose a
pitch that says so, and where it goes. The decisions and the rejected alternatives are in
`docs/DECISIONS-LOG.md`.

## What changed

| Surface | Change |
|---|---|
| `README.md` | Headline "An agentic harness you govern and measure"; an opening paragraph that keeps today (a governed company of the coding CLIs you already pay for) apart from Phase 9 (planned, not built); one Status line saying Phase 9 is approved for planning only |
| `site/src/pages/index.astro` | Title, meta description and H1 carry the pitch; the lede keeps today's text and adds the gate and the planned engine and bench |
| `site/src/layouts/Base.astro` | The preview image's alt text matches the new card |
| `package.json` | The description carries the pitch |
| `docs/brand/ephesus-social-card.svg`, `site/public/img/social-card.svg`, `site/public/img/social-card.png` | The rebuilt card (below) |
| `docs/brand/README.md` | The card is now built, not hand-edited, and the PNG comes with it |
| `docs/DECISIONS-LOG.md` | The Architect's decisions of 2026-10-15, appended |

The repository's description on GitHub is set to the approved text with `gh repo edit`,
outside the tree.

## The card

It is built in VectorCraft by `Cassandra-Collective/.github`'s brand script, and drawn
entirely in this repository's identity:
- this folder's pixel column (the 64-unit master at an integer 6x) and pixel wordmark,
  vendored there rect for rect from the previous card;
- seven §2 tokens: ink-900, ink-700, marble-100, marble-200, marble-300, terracotta and
  terracotta-light;
- IBM Plex Mono for the kicker "A CASSANDRA COLLECTIVE PROJECT", the headline, the pitch,
  and the status and addresses (`pre-alpha · MIT · ephesushq.com`,
  `github.com/Cassandra-Collective/Ephesus`).

The layout is the one the Collective's other project cards share. The SVG keeps live
text, as the previous card did.

## Evidence

- `grep -ohE '#[0-9A-Fa-f]{6}' docs/brand/*.svg | sort -u` gives `#221A14 #4A3B2C
  #C4552D #D3C4A5 #E7DCC6 #E8A987 #F4EDE0`, each defined in `docs/design/UI-DESIGN.md`.
  The fractional-coordinate grep prints nothing.
- Widths measured by the build, against a 533 px text column:

  | Line | Width (px) |
  |---|---|
  | Kicker | 345.6 |
  | Headline | 516.6 |
  | Pitch, first line | 453.6 |
  | Pitch, second line | 491.4 |
  | Status | 334.8 |
  | Address | 421.2 |

- Contrast: ink-900 on marble-100 is 14.71:1, ink-700 9.23:1, and the terracotta rule
  3.85:1.
- The old line no longer appears outside dated records:
  `grep -rn "agent company you govern"` finds it only in the decisions log, progress
  archives and implementation records, which are history and stay as written.

## Not covered

- The site changes reach ephesushq.com only when it is next built and deployed.
- The social preview uploaded in the repository settings changes only when a maintainer
  uploads `site/public/img/social-card.png`.
- No application source changed.
