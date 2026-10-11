import { describe, expect, it } from 'vitest'
import {
  INTAKE_LICENSES,
  STOA_SCHEMA_VERSION,
  checkIntake,
  checkRegistrar,
  checkStudiable,
  licenseStanding,
  nextBriefId,
  parseWatchlist,
  parseWatchlistMarkdown,
  registerDraftSchema,
  sourceIdFor,
  uniqueSourceId,
  watchlistEntrySchema
} from '../../src/shared/stoa'

/**
 * The Stoa's validators (ADR-0017, FR-13, SDD §4.7).
 *
 * The entry shape IS the provenance chain — id, pin and license are what make
 * a brief's claims checkable years later — so most of these tests are about
 * what the schema refuses rather than what it accepts.
 */

function entry(over: Record<string, unknown> = {}): unknown {
  return {
    id: 'src-hermes-agent',
    url: 'https://github.com/NousResearch/hermes-agent',
    kind: 'git',
    tags: ['agent-loop'],
    license: 'unverified',
    pin: null,
    registeredBy: 'architect',
    registeredAt: '2026-08-28T09:00:00.000Z',
    notes: 'what to learn',
    ...over
  }
}

describe('the watchlist entry schema (SDD §4.7)', () => {
  it('accepts the documented shape', () => {
    expect(watchlistEntrySchema.safeParse(entry()).success).toBe(true)
  })

  it.each([
    ['an id that is not slug-shaped', { id: 'hermes agent' }],
    ['an id without the src- prefix', { id: 'hermes-agent' }],
    ['a url that is not a url', { url: 'not-a-url' }],
    ['a non-git kind while v1 studies git only', { kind: 'web' }],
    ['no tags at all — tags scope every study', { tags: [] }],
    ['an empty license rather than "unverified"', { license: '' }],
    ['an empty pin instead of null', { pin: '' }],
    ['a registrar who is not the Architect', { registeredBy: 'artemis' }],
    ['an unknown extra field', { escalate: true }]
  ])('refuses %s', (_why, over) => {
    expect(watchlistEntrySchema.safeParse(entry(over)).success).toBe(false)
  })

  it('accepts "unverified" as a license — recording "not checked" is honest', () => {
    const parsed = watchlistEntrySchema.safeParse(entry({ license: 'unverified' }))
    expect(parsed.success).toBe(true)
  })

  it('refuses a registrar smuggled in as an extra key, not just a wrong value', () => {
    // `.strict()` is what makes the authority claim un-forgeable at the
    // boundary: there is no field to set, so there is nothing to ignore.
    const parsed = registerDraftSchema.safeParse({
      url: 'https://github.com/o/r',
      tags: ['x'],
      license: 'MIT',
      pin: null,
      notes: '',
      registeredBy: 'artemis'
    })
    expect(parsed.success).toBe(false)
  })
})

describe('the watchlist file', () => {
  it('round-trips a valid file', () => {
    const body = JSON.stringify({
      schemaVersion: STOA_SCHEMA_VERSION,
      sources: [entry()],
      retired: []
    })
    const parsed = parseWatchlist(body)
    expect(parsed.ok).toBe(true)
    if (parsed.ok) expect(parsed.watchlist.sources[0]?.id).toBe('src-hermes-agent')
  })

  it('reports every reason at once rather than the first', () => {
    const parsed = parseWatchlist(
      JSON.stringify({
        schemaVersion: STOA_SCHEMA_VERSION,
        sources: [entry({ id: 'bad id', url: 'nope' })],
        retired: []
      })
    )
    expect(parsed.ok).toBe(false)
    if (!parsed.ok) expect(parsed.reasons.length).toBeGreaterThan(1)
  })

  it('names the file, not a stack trace, when it is not JSON', () => {
    const parsed = parseWatchlist('{ nope')
    expect(parsed.ok).toBe(false)
    if (!parsed.ok) expect(parsed.reasons[0]).toContain('not JSON')
  })

  it('refuses a schemaVersion it does not know', () => {
    const parsed = parseWatchlist(JSON.stringify({ schemaVersion: 2, sources: [], retired: [] }))
    expect(parsed.ok).toBe(false)
  })
})

describe('curation authority (ADR-0017 R1, FR-13.1)', () => {
  it('lets the Architect curate', () => {
    expect(checkRegistrar('architect').allowed).toBe(true)
  })

  it.each(['artemis', 'agent.artemis', 'researcher', 'orchestrator', ''])(
    'refuses "%s" — the Stoa may never widen its own reading list',
    (who) => {
      const check = checkRegistrar(who)
      expect(check.allowed).toBe(false)
      expect(check.because).toContain('R1')
    }
  )
})

describe('what a source is allowed to do (FR-13.2, FR-13.5)', () => {
  const pinned = watchlistEntrySchema.parse(entry({ pin: 'b91a49f', license: 'MIT' }))
  const unpinned = watchlistEntrySchema.parse(entry())

  it('refuses to study an unpinned source — a study runs on a pinned snapshot', () => {
    const check = checkStudiable(unpinned)
    expect(check.allowed).toBe(false)
    expect(check.because).toContain('FR-13.2')
  })

  it('studies a pinned source', () => {
    expect(checkStudiable(pinned).allowed).toBe(true)
  })

  it('permits study but refuses intake on an unverified license', () => {
    // The asymmetry IS FR-13.5: "we did not check" is not a licence to copy,
    // but it is no reason to refuse to read in public.
    expect(checkStudiable(watchlistEntrySchema.parse(entry({ pin: 'abc1234' }))).allowed).toBe(true)
    expect(checkIntake(unpinned).allowed).toBe(false)
  })

  it.each(['unverified', 'UNVERIFIED', '  Unverified  '])(
    'treats %s as unverified however it is cased',
    (license) => {
      const row = watchlistEntrySchema.parse(entry({ license }))
      expect(checkIntake(row).allowed).toBe(false)
    }
  )

  it('permits intake on a verified license', () => {
    expect(checkIntake(pinned).allowed).toBe(true)
  })
})

describe('intake fails closed (FR-13.5, DECISIONS-LOG 2026-10-11)', () => {
  // The rule this replaced refused only the exact string `unverified`, so a
  // License cell holding a READING — `unverified (read as AGPL-3.0 …)` — was
  // taken for a verified license and permitted intake from the AGPL row it was
  // written to refuse. Only a listed, MIT-compatible license permits it now.
  const withLicense = (license: string): ReturnType<typeof watchlistEntrySchema.parse> =>
    watchlistEntrySchema.parse(entry({ license, pin: 'b91a49f' }))

  it('is the list the Architect decided, and no other', () => {
    // Pinned literally: a test that only iterated the list would pass with an
    // id removed from it, or AGPL-3.0 added to it.
    expect([...INTAKE_LICENSES]).toEqual([
      'MIT',
      'MIT-0',
      'Apache-2.0',
      'BSD-2-Clause',
      'BSD-3-Clause',
      'ISC',
      '0BSD',
      'Unlicense',
      'CC0-1.0',
      'Zlib'
    ])
  })

  it.each([...INTAKE_LICENSES])('permits intake from %s', (license) => {
    expect(checkIntake(withLicense(license)).allowed).toBe(true)
  })

  it.each(['mit', 'APACHE-2.0', '  BSD-3-Clause  ', 'unlicense'])(
    'matches %s ignoring case and surrounding space, as SPDX identifiers are',
    (license) => {
      expect(checkIntake(withLicense(license)).allowed).toBe(true)
    }
  )

  it.each([
    ['a reading kept in the license cell', 'unverified (read as MIT)'],
    ['a capitalised note', 'Unverified — pending'],
    ['a copyleft license somebody did verify', 'AGPL-3.0'],
    ['another copyleft license', 'GPL-3.0-only'],
    ['a listed license with a note', 'MIT (read 2026-10-03)'],
    ['a placeholder', 'TBD'],
    ['a guess', 'MIT?']
  ])('refuses intake for %s ("%s")', (_why, license) => {
    const check = checkIntake(withLicense(license))
    expect(check.allowed).toBe(false)
    // A refusal must teach the rule: it reaches the reading desk as
    // `intakeBlocked`, in front of the person who has to fix the entry.
    expect(check.because).toContain('FR-13.5')
    expect(check.because).toContain(INTAKE_LICENSES.join(', '))
    expect(check.because).toContain(license.trim())
  })

  it('still says "unverified" for the honest unverified value', () => {
    const check = checkIntake(withLicense('unverified'))
    expect(check.allowed).toBe(false)
    expect(check.because).toContain('license is unverified')
  })

  it('names three standings, so the researcher is never told "nobody checked" of a read license', () => {
    expect(licenseStanding('MIT')).toBe('permitted')
    expect(licenseStanding(' Unverified ')).toBe('unverified')
    expect(licenseStanding('unverified (read as AGPL-3.0)')).toBe('incompatible')
    expect(licenseStanding('AGPL-3.0')).toBe('incompatible')
  })
})

describe('id derivation', () => {
  it.each([
    ['https://github.com/NousResearch/hermes-agent', 'src-hermes-agent'],
    ['https://github.com/chaitanyagiri/munder-difflin', 'src-munder-difflin'],
    ['https://github.com/anomalyco/opencode.git', 'src-opencode'],
    ['https://github.com/owner/Repo_Name/', 'src-repo-name']
  ])('derives %s → %s', (url, id) => {
    expect(sourceIdFor(url)).toBe(id)
  })

  it('never returns the bare prefix', () => {
    expect(sourceIdFor('https://example.com')).toBe('src-example-com')
    expect(sourceIdFor('///')).toBe('src-source')
  })

  it('mints a free id rather than colliding with a registered one', () => {
    expect(uniqueSourceId('src-opencode', ['src-opencode'])).toBe('src-opencode-2')
    expect(uniqueSourceId('src-opencode', ['src-opencode', 'src-opencode-2'])).toBe(
      'src-opencode-3'
    )
  })

  it('mints brief ids that never reuse one', () => {
    expect(nextBriefId([])).toBe('RB-001')
    expect(nextBriefId(['RB-001'])).toBe('RB-002')
    // A gap does not let the next mint fall back into it — RB-002 is cited by
    // whatever referenced it, whether or not the file still exists.
    expect(nextBriefId(['RB-001', 'RB-009'])).toBe('RB-010')
  })
})

describe('the build-phase seed reader (FR-13.7)', () => {
  const table = [
    '# Stoa watchlist',
    '',
    'Prose the table grows around, with a | pipe | in it.',
    '',
    '| ID | Source | Tags | License | Pin | Notes (what to learn) |',
    '|---|---|---|---|---|---|',
    '| src-hermes-agent | https://github.com/NousResearch/hermes-agent | `agent-loop`, `tool-use` | unverified | *(set at first study)* | How a lab-grade harness structures its loop. |',
    '| src-munder-difflin | https://github.com/chaitanyagiri/munder-difflin | `orchestration`, `hive` | MIT | `b91a49f` (2026-08-28, [RB-001](./briefs/RB-001.md)) | The project’s own inspiration. |',
    ''
  ].join('\n')

  const at = '2026-08-28T09:00:00.000Z'
  const { entries: parsed, rejected } = parseWatchlistMarkdown(table, at)

  /** The License cell `src-odysseus` carried on `main` at 75ec37c: 166 characters. */
  const READING_IN_THE_CELL =
    'unverified (read as AGPL-3.0 on 2026-10-02; **pattern-learning only whatever verification finds** — AGPL code intake is refused under FR-13.5 for this MIT repository)'

  it('reads only the source rows — not the header, separator or prose', () => {
    expect(parsed.map((row) => row.id)).toEqual(['src-hermes-agent', 'src-munder-difflin'])
    // Prose and the header are not source rows, so they are not "rejected"
    // either: rejection is for a row that WAS a source row and failed.
    expect(rejected).toEqual([])
  })

  it('reads the pin when there is one and null when the cell is a placeholder', () => {
    expect(parsed[0]?.pin).toBeNull()
    expect(parsed[1]?.pin).toBe('b91a49f')
  })

  it('strips the backticks off tags, which scope every study', () => {
    expect(parsed[0]?.tags).toEqual(['agent-loop', 'tool-use'])
  })

  it('carries the license verbatim, unverified included', () => {
    expect(parsed[0]?.license).toBe('unverified')
    expect(parsed[1]?.license).toBe('MIT')
  })

  it('stamps the registrar and the seed time, which the table does not record', () => {
    expect(parsed.every((row) => row.registeredBy === 'architect')).toBe(true)
    expect(parsed.every((row) => row.registeredAt === at)).toBe(true)
  })

  it('refuses a row the schema refuses, naming it and the field, instead of returning it', () => {
    // The 2026-10-11 defect, at its source: this reader returned the cell as
    // it stood, and the seed wrote a file no later read could parse.
    expect(READING_IN_THE_CELL).toHaveLength(166)
    const read = parseWatchlistMarkdown(
      [
        `| src-odysseus | https://github.com/odysseus-dev/odysseus | \`agent-loop\` | ${READING_IN_THE_CELL} | *(set at first study)* | n |`,
        '| src-fine | https://github.com/o/fine | `x` | MIT | `abc1234` | n |'
      ].join('\n'),
      at
    )
    expect(read.entries.map((row) => row.id)).toEqual(['src-fine'])
    expect(read.rejected).toHaveLength(1)
    expect(read.rejected[0]?.id).toBe('src-odysseus')
    expect(read.rejected[0]?.reasons.join('; ')).toMatch(/^license: /)
  })

  it('returns only entries the schema accepts — every source row is one or the other', () => {
    const read = parseWatchlistMarkdown(
      `${table}\n| src-long | https://github.com/o/long | \`x\` | ${READING_IN_THE_CELL} | \`abc1234\` | n |\n`,
      at
    )
    for (const row of read.entries) expect(watchlistEntrySchema.safeParse(row).success).toBe(true)
    expect([...read.entries.map((row) => row.id), ...read.rejected.map((row) => row.id)]).toEqual([
      'src-hermes-agent',
      'src-munder-difflin',
      'src-long'
    ])
  })

  it('reports a row whose url cell has no url rather than inventing one or skipping it', () => {
    const read = parseWatchlistMarkdown(
      '| src-broken | *(pending)* | `x` | MIT | `abc1234` | n |',
      at
    )
    expect(read.entries).toEqual([])
    expect(read.rejected).toEqual([
      { id: 'src-broken', reasons: ['url: the Source cell holds no http(s) url'] }
    ])
  })

  it('takes only an http(s) url out of the Source cell — an ftp link is no url', () => {
    const read = parseWatchlistMarkdown(
      '| src-ftp | ftp://example.com/repo | `x` | MIT | n | n |',
      at
    )
    expect(read.rejected.map((row) => row.id)).toEqual(['src-ftp'])
  })

  it('reports a row with no tags, because tags scope every study', () => {
    const read = parseWatchlistMarkdown(
      '| src-untagged | https://github.com/o/r |  | MIT | n | n |',
      at
    )
    expect(read.entries).toEqual([])
    expect(read.rejected[0]?.id).toBe('src-untagged')
    expect(read.rejected[0]?.reasons.join('; ')).toMatch(/^tags: /)
  })
})
