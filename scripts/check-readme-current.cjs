#!/usr/bin/env node
'use strict'

/**
 * The README's landed list keeps up with the register (M8.4 audit, 2026-09-07).
 *
 * ## Why this exists
 *
 * M8.4's own package line named the defect: "the README has no setup section and
 * its status is two milestones stale". It fixed both. By M8.9 the same section
 * was three packages stale again — M8.6, M8.7a/b and M8.8 had landed and the
 * front door still described a company without worktree isolation, without
 * engine isolation, and without a survivable restart. Nothing was watching, so
 * it came straight back.
 *
 * ## What it checks, and what it deliberately does not
 *
 * The README carries an HTML comment naming the packages its narrative covers.
 * This compares that set against the packages ticked in the register:
 * `docs/PROGRESS.md` and the closed milestones archived in `docs/progress/`.
 *
 * It does NOT read the prose. Deciding whether a paragraph "describes" a package
 * is not something a script can do, and a check that guessed would either block
 * good writing or pass bad writing — both worse than no check.
 *
 * So the marker is a CLAIM, and this verifies the claim is complete. It catches
 * the oversight — a package landing while nobody touched the README — which is
 * the failure that actually happened, twice. It cannot catch someone editing the
 * marker without writing the sentence; that is a deliberate lie rather than an
 * oversight, and a different problem.
 *
 * Exit 0 when the sets agree, 1 with the difference named otherwise.
 */

const fs = require('node:fs')
const path = require('node:path')

const REPO_ROOT = path.resolve(__dirname, '..')

/** An archived milestone's file name: `M8.md`, `M5b.md`. */
const ARCHIVED = /^M(\d+)([a-z]?)\.md$/

/**
 * The register is `docs/PROGRESS.md` plus the closed milestones moved verbatim
 * into `docs/progress/` (2026-10-11). Moving M8 out of the one file this check
 * read would have left it finding no ticked M8 package, and so calling the
 * README's sixteen M8 names claimed-but-not-ticked — or, once nothing was ticked
 * at all, passing with nothing checked.
 *
 * Read in build order: every archived milestone by number, then suffix (M5
 * before M5b before M6), then PROGRESS.md. That is the order the single file
 * had, so "the most recently ticked package" still means what it meant. A file
 * in the archive that is not named for a milestone is REFUSED rather than
 * skipped: its ticks would otherwise vanish from the register unseen.
 */
function archiveFiles(root) {
  const dir = path.join(root, 'docs', 'progress')
  if (!fs.existsSync(dir)) return { files: [], strays: [] }
  const files = []
  const strays = []
  for (const name of fs.readdirSync(dir)) {
    const match = ARCHIVED.exec(name)
    if (match && match[1] !== undefined) {
      files.push({ name, number: Number(match[1]), suffix: match[2] ?? '' })
    } else {
      strays.push(name)
    }
  }
  files.sort((a, b) => a.number - b.number || a.suffix.localeCompare(b.suffix))
  return { files: files.map((file) => path.join(dir, file.name)), strays }
}

function readRegister(root, files) {
  const parts = files.map((file) => fs.readFileSync(file, 'utf8'))
  parts.push(fs.readFileSync(path.join(root, 'docs', 'PROGRESS.md'), 'utf8'))
  return parts.join('\n')
}

/**
 * The packages ticked in the register for the milestone the README is about.
 *
 * Scoped to ONE milestone deliberately: "Landed so far" is the current
 * milestone's narrative, and everything before it lives under *the previous
 * milestone's story* or in `docs/PROGRESS.md`. Requiring the README to name
 * every package since M0 would be asking the front door to be the register.
 *
 * The milestone is the one holding the most recently ticked package — which is
 * what the README should be describing — rather than the first with unticked
 * work, so a milestone whose first package has not landed yet does not make
 * this demand a paragraph about nothing.
 *
 * Two tracks tick packages: `M` milestones and, since the Rust rewrite
 * (ADR-0041), `R` phases (`R0.1`, `R1.3`). A milestone is keyed by its letter
 * AND number, so `R0` is never `M0`. "Most recent" is register order, so the
 * rewrite's section sits in PROGRESS.md where it falls in the build order:
 * after M8c, before M9 (R0.3).
 */
function landedPackages(progress) {
  const ticked = []
  for (const line of progress.split('\n')) {
    const match = /^- \[x\] \*\*(([MR])(\d+)(?:\.\d+)?[a-z]?)\b/.exec(line.trim())
    if (match && match[1] && match[2] && match[3]) {
      ticked.push({ id: match[1], milestone: `${match[2]}${match[3]}` })
    }
  }
  const last = ticked[ticked.length - 1]
  if (last === undefined) return { milestone: null, ids: [] }
  const ids = []
  for (const entry of ticked) {
    // The milestone's own summary row (`M8`, `R1`) is not a package; the
    // packages are the numbered ones under it.
    if (entry.milestone === last.milestone && entry.id !== entry.milestone) ids.push(entry.id)
  }
  return { milestone: last.milestone, ids }
}

/** The README's own claim about what its narrative covers. */
function claimedPackages(readme) {
  const marker = /<!--\s*landed:([^]*?)-->/.exec(readme)
  if (!marker || marker[1] === undefined) return null
  // Only the first line of the comment is the list; the rest explains it.
  const firstLine = marker[1].split('\n')[0] ?? ''
  return firstLine.trim().split(/\s+/).filter(Boolean)
}

/**
 * The whole check over a repository root, returned rather than printed so the
 * tests can run it against a fixture register. `out` is what passes, `err` what
 * fails, and `code` the exit status.
 */
function check(root) {
  const { files, strays } = archiveFiles(root)
  if (strays.length > 0) {
    return {
      code: 1,
      out: [],
      err: [
        `docs/progress/ holds a file not named for a milestone: ${strays.join(', ')}\n` +
          '  — an archived milestone is `M<number>[letter].md` (M8.md, M5b.md). Anything else\n' +
          '    is not read as the register, so its ticks would drop out of this check unseen.'
      ]
    }
  }
  const readme = fs.readFileSync(path.join(root, 'README.md'), 'utf8')
  const register = readRegister(root, files)

  const claimed = claimedPackages(readme)
  if (claimed === null) {
    return {
      code: 1,
      out: [],
      err: [
        'README.md has no `<!-- landed: ... -->` marker, so nothing relates it to the register.\n' +
          'Add one naming the packages the "Landed so far" narrative covers.'
      ]
    }
  }

  const { milestone, ids: landed } = landedPackages(register)
  if (milestone === null || landed.length === 0) {
    return {
      code: 0,
      out: ['no milestone has a ticked package yet — nothing for the README to claim.'],
      err: []
    }
  }

  const claimedSet = new Set(claimed)
  const landedSet = new Set(landed)
  const missing = landed.filter((id) => !claimedSet.has(id))
  const surplus = claimed.filter((id) => !landedSet.has(id))

  if (missing.length === 0 && surplus.length === 0) {
    return {
      code: 0,
      out: [
        `README landed list is current for ${milestone} (${String(landed.length)} package(s)).`
      ],
      err: []
    }
  }

  const err = [
    "README.md's landed list disagrees with the register (docs/PROGRESS.md and docs/progress/):\n"
  ]
  if (missing.length > 0) {
    err.push(
      `  ticked in the register, absent from the README: ${missing.join(', ')}\n` +
        '    — write what it did into "Landed so far" and add it to the marker. The front\n' +
        '      door describing a company older than the tree is how somebody decides not to\n' +
        '      install this over a problem that was fixed two packages ago.'
    )
  }
  if (surplus.length > 0) {
    err.push(
      `  claimed by the README, not ticked in the register: ${surplus.join(', ')}\n` +
        '    — either the package is not done, or the marker names it too early.'
    )
  }
  return { code: 1, out: [], err }
}

function main() {
  const result = check(REPO_ROOT)
  for (const line of result.out) console.log(line)
  for (const line of result.err) console.error(line)
  return result.code
}

if (require.main === module) process.exit(main())

module.exports = { archiveFiles, readRegister, landedPackages, claimedPackages, check, main }
