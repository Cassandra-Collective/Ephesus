import { spawnSync } from 'node:child_process'
import { createRequire } from 'node:module'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { afterEach, describe, expect, it } from 'vitest'
import { removeTempDir } from '../tmpdir'

/**
 * The README's landed list against the register (M8.4 audit; the register split
 * into `docs/PROGRESS.md` and `docs/progress/` on 2026-10-11).
 *
 * The script's job is which files it reads and in what order, so every case
 * runs the real `check()` over a fixture repository on disk. Each fixture is
 * built so that one wrong reading — the archive ignored, `M10` sorted before
 * `M9`, PROGRESS read before the archive, a stray file skipped — gives a
 * different verdict from the right one.
 */

const require_ = createRequire(import.meta.url)
const SCRIPT = fileURLToPath(new URL('../../scripts/check-readme-current.cjs', import.meta.url))
const REPO_ROOT = fileURLToPath(new URL('../..', import.meta.url))

interface Result {
  readonly code: number
  readonly out: readonly string[]
  readonly err: readonly string[]
}

const script = require_(SCRIPT) as {
  check: (root: string) => Result
  archiveFiles: (root: string) => { files: string[]; strays: string[] }
}

const temps: string[] = []

afterEach(() => {
  for (const dir of temps.splice(0)) removeTempDir(dir)
})

/** A repository with a README marker, a PROGRESS.md and an optional archive. */
function repo(options: {
  readonly marker: string | null
  readonly progress: readonly string[]
  readonly archive?: Readonly<Record<string, readonly string[]>>
}): string {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'eph-readme-'))
  temps.push(root)
  fs.mkdirSync(path.join(root, 'docs'), { recursive: true })
  const readme = options.marker === null ? '# x\n' : `# x\n\n<!-- landed: ${options.marker}\n-->\n`
  fs.writeFileSync(path.join(root, 'README.md'), readme)
  fs.writeFileSync(path.join(root, 'docs', 'PROGRESS.md'), options.progress.join('\n') + '\n')
  if (options.archive !== undefined) {
    fs.mkdirSync(path.join(root, 'docs', 'progress'))
    for (const [name, lines] of Object.entries(options.archive)) {
      fs.writeFileSync(path.join(root, 'docs', 'progress', name), lines.join('\n') + '\n')
    }
  }
  return root
}

const ticked = (id: string): string => `- [x] **${id} Something** — evidence.`
const open = (id: string): string => `- [ ] **${id} Something** — owed.`

describe('the register is PROGRESS.md plus the archive', () => {
  it('counts the ticks of an archived milestone', () => {
    // M8 moved out; M8b stayed. Ignoring the archive would call M8.0 and M8.1
    // claimed-but-not-ticked.
    const root = repo({
      marker: 'M8.0 M8.1 M8b',
      progress: ['## M8b', ticked('M8b.1'), open('M8b exit')],
      archive: { 'M8.md': ['## M8', ticked('M8.0'), ticked('M8.1')] }
    })
    const result = script.check(root)
    expect(result.err).toEqual([])
    expect(result.out).toEqual(['README landed list is current for M8 (3 package(s)).'])
    expect(result.code).toBe(0)
  })

  it('reads archived milestones by number, so M10 comes after M9', () => {
    // A string sort puts M10.md first, which would make M9.1 the last tick.
    const root = repo({
      marker: 'M10.1',
      progress: ['## M11', open('M11.0')],
      archive: { 'M9.md': [ticked('M9.1')], 'M10.md': [ticked('M10.1')] }
    })
    expect(script.check(root)).toEqual({
      code: 0,
      out: ['README landed list is current for M10 (1 package(s)).'],
      err: []
    })
  })

  it('puts a lettered milestone after its number, and PROGRESS.md after the archive', () => {
    const root = repo({
      marker: null,
      progress: [],
      archive: { 'M5b.md': [], 'M5.md': [], 'M6.md': [] }
    })
    expect(script.archiveFiles(root).files.map((file) => path.basename(file))).toEqual([
      'M5.md',
      'M5b.md',
      'M6.md'
    ])
    // PROGRESS's tick is the latest; read first, the archive's M9.1 would be.
    const later = repo({
      marker: 'M10.1',
      progress: [ticked('M10.1')],
      archive: { 'M9.md': [ticked('M9.1')] }
    })
    expect(script.check(later).code).toBe(0)
  })

  it('refuses a file in the archive that is not named for a milestone', () => {
    const root = repo({
      marker: 'M8.0',
      progress: [],
      archive: { 'M8.md': [ticked('M8.0')], 'notes.md': [ticked('M8.1')] }
    })
    const result = script.check(root)
    expect(result.code).toBe(1)
    expect(result.err.join('\n')).toContain('notes.md')
    expect(result.out).toEqual([])
  })

  it('works with no archive at all', () => {
    const root = repo({
      marker: 'M3.1 M3.2',
      progress: [ticked('M3.1'), ticked('M3.2'), open('M3 exit')]
    })
    expect(script.check(root).code).toBe(0)
  })
})

describe('the verdict', () => {
  it('names a ticked package the README does not claim', () => {
    const root = repo({ marker: 'M4.1', progress: [ticked('M4.1'), ticked('M4.2')] })
    const result = script.check(root)
    expect(result.code).toBe(1)
    expect(result.err.join('\n')).toContain('ticked in the register, absent from the README: M4.2')
  })

  it('names a claimed package the register has not ticked', () => {
    const root = repo({ marker: 'M4.1 M4.2', progress: [ticked('M4.1'), open('M4.2')] })
    const result = script.check(root)
    expect(result.code).toBe(1)
    expect(result.err.join('\n')).toContain(
      'claimed by the README, not ticked in the register: M4.2'
    )
  })

  it('refuses a README with no marker', () => {
    const result = script.check(repo({ marker: null, progress: [ticked('M1.1')] }))
    expect(result.code).toBe(1)
    expect(result.err.join('\n')).toContain('no `<!-- landed: ... -->` marker')
  })

  it('passes, and says so, when nothing is ticked yet', () => {
    const result = script.check(repo({ marker: 'M0.1', progress: [open('M0.1')] }))
    expect(result.code).toBe(0)
    expect(result.out).toEqual([
      'no milestone has a ticked package yet — nothing for the README to claim.'
    ])
  })
})

/**
 * The Rust rewrite's phases tick `R` packages (ADR-0041; R0.3). Each case is
 * built so one wrong reading gives a different verdict: the `R` track unseen,
 * a milestone keyed by its number alone (so `R0` reads as `M0`), or a phase's
 * own summary row counted as a package.
 */
describe('the R track (the Rust rewrite)', () => {
  it('makes an R phase the milestone when its package is ticked last', () => {
    const progress = [ticked('M8.1'), ticked('M8b'), ticked('R0.1'), ticked('R0.2')]
    const current = script.check(repo({ marker: 'R0.1 R0.2', progress }))
    expect(current.code).toBe(0)
    expect(current.out).toEqual(['README landed list is current for R0 (2 package(s)).'])
    const stale = script.check(repo({ marker: 'M8.1 M8b', progress }))
    expect(stale.code).toBe(1)
    const err = stale.err.join('\n')
    expect(err).toContain('ticked in the register, absent from the README: R0.1, R0.2')
    expect(err).toContain('claimed by the README, not ticked in the register: M8.1, M8b')
  })

  it('keys a milestone by its letter, so R0 is not M0', () => {
    const progress = [ticked('M0.1'), ticked('M0.2'), ticked('R0.1')]
    const result = script.check(repo({ marker: 'R0.1', progress }))
    expect(result.code).toBe(0)
    expect(result.out).toEqual(['README landed list is current for R0 (1 package(s)).'])
    const merged = script.check(repo({ marker: 'M0.1 M0.2 R0.1', progress }))
    expect(merged.code).toBe(1)
    expect(merged.err.join('\n')).toContain(
      'claimed by the README, not ticked in the register: M0.1, M0.2'
    )
  })

  it("does not count an R phase's own summary row as a package", () => {
    const progress = [ticked('R0.1'), ticked('R1.1'), ticked('R1')]
    const result = script.check(repo({ marker: 'R1.1', progress }))
    expect(result.code).toBe(0)
    expect(result.out).toEqual(['README landed list is current for R1 (1 package(s)).'])
  })

  it('still reads an M milestone exactly as before, its summary row included', () => {
    const progress = [ticked('M8.1'), ticked('M8b'), ticked('M8c'), ticked('M8')]
    const result = script.check(repo({ marker: 'M8.1 M8b M8c', progress }))
    expect(result.code).toBe(0)
    expect(result.out).toEqual(['README landed list is current for M8 (3 package(s)).'])
  })
})

describe('the command CI runs', () => {
  /**
   * CI runs the file, not `check()`, and only its exit status fails the job. The
   * script finds its repository from its own location, so each case copies the
   * real script into a fixture repository's `scripts/` and runs it there.
   */
  function run(root: string): { status: number | null; stdout: string; stderr: string } {
    fs.mkdirSync(path.join(root, 'scripts'))
    const copy = path.join(root, 'scripts', 'check-readme-current.cjs')
    fs.copyFileSync(SCRIPT, copy)
    const result = spawnSync(process.execPath, [copy], { encoding: 'utf8' })
    return { status: result.status, stdout: result.stdout, stderr: result.stderr }
  }

  it('exits 1 and says why when the README and the register disagree', () => {
    const result = run(
      repo({
        marker: 'M8.0',
        progress: [ticked('M8b.1')],
        archive: { 'M8.md': [ticked('M8.0')] }
      })
    )
    expect(result.status).toBe(1)
    expect(result.stderr).toContain('ticked in the register, absent from the README: M8b')
    expect(result.stdout).toBe('')
  })

  it('exits 0 and says so when they agree', () => {
    const result = run(
      repo({
        marker: 'M8.0 M8b',
        progress: [ticked('M8b.1')],
        archive: { 'M8.md': [ticked('M8.0')] }
      })
    )
    expect(result.status).toBe(0)
    expect(result.stdout).toBe('README landed list is current for M8 (2 package(s)).\n')
    expect(result.stderr).toBe('')
  })
})

describe('this repository', () => {
  it('has a README current with the register as it is split today', () => {
    // The register is the archive plus PROGRESS.md, and since R0.3 the README
    // describes the Rust rewrite's current phase (an R milestone) whenever its
    // package is the most recent tick; M8's names became history then.
    const result = script.check(REPO_ROOT)
    expect(result.err).toEqual([])
    expect(result.code).toBe(0)
    expect(result.out[0]).toMatch(
      /^README landed list is current for [MR]\d+ \(\d+ package\(s\)\)\.$/
    )
    expect(script.archiveFiles(REPO_ROOT).strays).toEqual([])
  })
})
