import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { afterEach, describe, expect, it } from 'vitest'
import { PromptStore } from '../../src/main/prompts'
import { Stoa } from '../../src/main/stoa'
import { checkIntake, parseWatchlist } from '../../src/shared/stoa'
import { removeTempDir } from '../tmpdir'

const REPO = fileURLToPath(new URL('../../', import.meta.url))
/** A table whose rows are each a way a License or Source cell has gone wrong. */
const DEFECTIVE_TABLE = fs.readFileSync(
  path.join(REPO, 'test', 'fixtures', 'stoa-seed-defects', 'WATCHLIST.md'),
  'utf8'
)

/**
 * The Stoa's driver (ADR-0017, FR-13, SDD §4.7/§7.7).
 *
 * The watchlist decides what the company is allowed to read, so the tests that
 * matter are about what it refuses: a registrar who is not the Architect, a
 * retire that would delete rather than move, and a seed that would bring the
 * table across without the briefs its rows are cited by.
 */

const homes: string[] = []
afterEach(() => {
  for (const home of homes.splice(0)) {
    removeTempDir(home)
  }
})

interface Rig {
  readonly stoa: Stoa
  readonly home: string
  readonly agoraRoot: string
  readonly seedFrom: string
  readonly events: { kind: string; event?: unknown; [key: string]: unknown }[]
  readonly degraded: string[]
  readonly commits: string[]
  /** A second driver over the SAME home: a later boot reads only what is on disk. */
  reopen(): { readonly stoa: Stoa; readonly degraded: string[] }
}

const SEED_TABLE = [
  '| ID | Source | Tags | License | Pin | Notes (what to learn) |',
  '|---|---|---|---|---|---|',
  '| src-hermes-agent | https://github.com/NousResearch/hermes-agent | `agent-loop` | unverified | *(set at first study)* | Loop shape. |',
  '| src-munder-difflin | https://github.com/chaitanyagiri/munder-difflin | `orchestration` | MIT | `b91a49f` | The inspiration. |',
  ''
].join('\n')

function rig(
  options: {
    readonly seed?: boolean
    readonly briefs?: boolean
    /** The build-phase table to seed from; the two-row SEED_TABLE by default. */
    readonly table?: string
    readonly prompts?: boolean
  } = {}
): Rig {
  const home = fs.mkdtempSync(path.join(os.tmpdir(), 'eph-stoa-'))
  homes.push(home)
  const agoraRoot = path.join(home, 'agora')
  const seedFrom = path.join(home, 'docs-stoa')
  fs.mkdirSync(agoraRoot, { recursive: true })
  if (options.seed !== false) {
    fs.mkdirSync(seedFrom, { recursive: true })
    fs.writeFileSync(path.join(seedFrom, 'WATCHLIST.md'), options.table ?? SEED_TABLE, 'utf8')
    if (options.briefs !== false) {
      fs.mkdirSync(path.join(seedFrom, 'briefs'), { recursive: true })
      fs.writeFileSync(
        path.join(seedFrom, 'briefs', 'RB-001-munder-difflin.md'),
        '# RB-001 — Munder Difflin orchestration\n\nbody\n',
        'utf8'
      )
      fs.writeFileSync(path.join(seedFrom, 'briefs', 'README.md'), '# not a brief\n', 'utf8')
    }
  }
  const events: Rig['events'] = []
  const degraded: string[] = []
  const commits: string[] = []
  const prompts =
    options.prompts === true
      ? new PromptStore(path.join(home, 'prompts'), path.join(REPO, 'prompts'))
      : null
  const stoa = new Stoa({
    agoraRoot,
    seedFrom,
    // Outside the Agora, as `index.ts` passes it: the default sits inside it,
    // and `plan()` refuses a checkout there (ADR-0004).
    scratchRoot: path.join(home, 'scratch'),
    prompts,
    onLogEvent: (draft) => events.push(draft),
    onDegraded: (detail) => degraded.push(detail),
    commitSoon: (subject) => commits.push(subject),
    now: () => new Date('2026-08-28T09:00:00.000Z')
  })
  const reopen = (): { stoa: Stoa; degraded: string[] } => {
    const later: string[] = []
    return {
      stoa: new Stoa({
        agoraRoot,
        seedFrom,
        onDegraded: (detail) => later.push(detail),
        now: () => new Date('2026-08-29T09:00:00.000Z')
      }),
      degraded: later
    }
  }
  return { stoa, home, agoraRoot, seedFrom, events, degraded, commits, reopen }
}

const DRAFT = {
  url: 'https://github.com/anomalyco/opencode',
  tags: ['engine-cli'],
  license: 'unverified',
  pin: null,
  notes: 'session model'
}

describe('seeding from the build-phase archive (FR-13.7)', () => {
  it('reads the repo watchlist table into the agora watchlist at first use', () => {
    const { stoa, agoraRoot } = rig()
    expect(stoa.sources().map((row) => row.id)).toEqual(['src-hermes-agent', 'src-munder-difflin'])
    const onDisk = parseWatchlist(
      fs.readFileSync(path.join(agoraRoot, 'stoa', 'watchlist.json'), 'utf8')
    )
    expect(onDisk.ok).toBe(true)
  })

  it('brings the briefs across WITH the table, not after it', () => {
    // The Gymnasium seeded a ledger without the proposals its rows linked to
    // and every link broke (M5 close-out audit, finding 3). A seeded source
    // whose brief did not cross over is the same half-archive.
    const { stoa } = rig()
    expect(stoa.briefs().map((row) => row.id)).toEqual(['RB-001'])
    expect(stoa.brief('RB-001')).toContain('Munder Difflin')
  })

  it('answers a seeded brief even as the FIRST call on a fresh home', () => {
    // M5b close-out audit, finding 2: `brief()` read the archive directory
    // without seeding first, so a fresh home whose first Stoa-touching action
    // was `briefExists('RB-001')` — a gym proposal citing a seeded brief —
    // false-refused it. The order of first calls must not decide the answer.
    const { stoa } = rig()
    expect(stoa.brief('RB-001')).toContain('Munder Difflin')
  })

  it('reads a brief title off its heading and ignores non-brief files', () => {
    const { stoa } = rig()
    expect(stoa.briefs()).toEqual([
      { id: 'RB-001', title: 'Munder Difflin orchestration', file: 'RB-001-munder-difflin.md' }
    ])
  })

  it('logs the seed and asks for a commit', () => {
    const { stoa, events, commits } = rig()
    stoa.sources()
    const seeded = events.find((e) => e.event === 'seeded')
    expect(seeded).toMatchObject({ kind: 'stoa', sources: 2, briefs: 1 })
    expect(commits).toContain('stoa: seed the watchlist from the build-phase archive')
  })

  it('seeds once — a second read does not re-copy or duplicate rows', () => {
    const { stoa, events } = rig()
    stoa.sources()
    stoa.sources()
    expect(events.filter((e) => e.event === 'seeded')).toHaveLength(1)
    expect(stoa.sources()).toHaveLength(2)
  })

  it('degrades visibly with no archive rather than failing the boot', () => {
    const { stoa, degraded } = rig({ seed: false })
    expect(stoa.sources()).toEqual([])
    expect(degraded[0]).toContain('no build-phase watchlist')
  })

  it('seeds the table even when the archive has no briefs directory', () => {
    const { stoa } = rig({ briefs: false })
    expect(stoa.sources()).toHaveLength(2)
    expect(stoa.briefs()).toEqual([])
  })
})

describe('a table row the schema refuses (FR-13.7, invariant §7; DECISIONS-LOG 2026-10-11)', () => {
  // Until 2026-10-11 the seed wrote and returned the table's rows unchecked.
  // With a 166-character License cell on `main`, a fresh home's FIRST read
  // listed it and allowed intake from it, and every LATER read found the file
  // invalid and the Stoa empty. These go through the real seed path: a temp
  // home, a table on disk, the first call that touches the Stoa.
  const SEEDED = [
    'src-fixture-good',
    'src-fixture-unverified',
    'src-fixture-annotated',
    'src-fixture-pending',
    'src-fixture-copyleft'
  ]

  it('seeds the rows the schema accepts and leaves out the ones it refuses', () => {
    const { stoa } = rig({ table: DEFECTIVE_TABLE })
    expect(stoa.sources().map((row) => row.id)).toEqual(SEEDED)
  })

  it('agrees with itself: the first read and a later boot read the same watchlist', () => {
    const fresh = rig({ table: DEFECTIVE_TABLE })
    const first = fresh.stoa.watchlist()
    const onDisk = parseWatchlist(
      fs.readFileSync(path.join(fresh.agoraRoot, 'stoa', 'watchlist.json'), 'utf8')
    )
    expect(onDisk.ok).toBe(true)
    // A second driver over the same home is what a restart is: it has only
    // the file. The defect was invisible to any test that asked the first one.
    const later = fresh.reopen()
    expect(later.stoa.watchlist()).toEqual(first)
    if (onDisk.ok) expect(onDisk.watchlist).toEqual(first)
    expect(later.degraded).toEqual([])
    expect(fresh.degraded.some((d) => d.includes('not a valid watchlist'))).toBe(false)
  })

  it('reports every refused row by id and reason, in ONE degradation', () => {
    // One, because a degradation cause holds one message and a later report
    // under it replaces the earlier one (`DegradationLog.report`): a row per
    // report would leave only the last row visible.
    const { stoa, degraded } = rig({ table: DEFECTIVE_TABLE })
    stoa.sources()
    expect(degraded).toHaveLength(1)
    const [report] = degraded
    expect(report).toContain('2 row(s) were not seeded')
    expect(report).toMatch(/src-fixture-long \(license: [^)]*120/)
    expect(report).toContain('src-fixture-nourl (url: the Source cell holds no http(s) url)')
    // And it says how to get a fixed row back, because the seed runs once.
    expect(report).toContain('The seed runs once')
    expect(report).toContain('aside and restart')
  })

  it('records the refused rows in the seeded event, for the durable log', () => {
    const { stoa, events } = rig({ table: DEFECTIVE_TABLE })
    stoa.sources()
    const seeded = events.find((e) => e.event === 'seeded')
    expect(seeded).toMatchObject({ kind: 'stoa', sources: SEEDED.length })
    const rejected = seeded?.['rejected'] as { id: string; reasons: string[] }[] | undefined
    expect(rejected?.map((row) => row.id)).toEqual(['src-fixture-long', 'src-fixture-nourl'])
    expect(rejected?.[0]?.reasons[0]).toMatch(/^license: /)
    expect(seeded).not.toHaveProperty('refused')
  })

  it('reports nothing and records no refusal for a table that seeds cleanly', () => {
    const { stoa, degraded, events } = rig()
    stoa.sources()
    expect(degraded).toEqual([])
    expect(events.find((e) => e.event === 'seeded')?.['rejected']).toEqual([])
  })

  it('refuses intake for every seeded row whose license is not on the list', () => {
    const { stoa } = rig({ table: DEFECTIVE_TABLE })
    // Asked where production asks it: the plan a study is run from.
    const intake = Object.fromEntries(
      SEEDED.map((id) => {
        const planned = stoa.plan(id)
        return [id, planned.ok ? planned.plan.intakePermitted : null]
      })
    )
    expect(intake).toEqual({
      'src-fixture-good': true,
      'src-fixture-unverified': false,
      'src-fixture-annotated': false,
      'src-fixture-pending': false,
      'src-fixture-copyleft': false
    })
    const annotated = stoa.sources().find((row) => row.id === 'src-fixture-annotated')
    expect(annotated?.license).toBe('unverified (read as MIT)')
    if (annotated) expect(checkIntake(annotated).allowed).toBe(false)
  })

  it('tells the researcher which refusal it is, from prompts/stoa/', () => {
    // "Nobody has checked" is false of a license somebody read, so a verified
    // license that intake may not come from gets its own note.
    const { stoa } = rig({ table: DEFECTIVE_TABLE, prompts: true })
    const words = (id: string): string => {
      const planned = stoa.plan(id)
      return planned.ok ? (planned.instructions ?? '') : ''
    }
    expect(words('src-fixture-good')).toContain('Pattern-learning is unrestricted')
    expect(words('src-fixture-unverified')).toContain('nobody has checked')
    expect(words('src-fixture-copyleft')).toContain('**AGPL-3.0**, which is not one of')
    expect(words('src-fixture-copyleft')).toContain('MIT, MIT-0, Apache-2.0')
    expect(words('src-fixture-copyleft')).not.toContain('nobody has checked')
    expect(words('src-fixture-annotated')).toContain('intake is not')
    expect(words('src-fixture-annotated')).not.toContain('nobody has checked')
  })

  it('seeds nothing, and says so, when the rows do not make one watchlist', () => {
    // The file-level rule (at most 200 sources) is checked before writing too:
    // a seed that wrote 201 rows would write a file no later read accepts.
    const rows = Array.from(
      { length: 201 },
      (_, n) =>
        `| src-bulk-${String(n)} | https://example.invalid/bulk/${String(n)} | \`x\` | MIT | \`abc1234\` | n |`
    )
    const fresh = rig({
      table: ['| ID | Source | Tags | License | Pin | Notes |', ...rows, ''].join('\n')
    })
    expect(fresh.stoa.sources()).toEqual([])
    expect(fresh.degraded).toHaveLength(1)
    expect(fresh.degraded[0]).toContain('do not make one watchlist')
    expect(fresh.degraded[0]).toContain('sources:')
    expect(fresh.events.find((e) => e.event === 'seeded')).toMatchObject({ sources: 0 })
    expect(fresh.events.find((e) => e.event === 'seeded')?.['refused']).toBeDefined()
    expect(fresh.reopen().stoa.watchlist()).toEqual(fresh.stoa.watchlist())
  })
})

describe('registering a source (FR-13.1, ADR-0017 R1)', () => {
  it('registers for the Architect and mints the id from the url', () => {
    const { stoa } = rig()
    expect(stoa.register(DRAFT, 'architect')).toEqual({ ok: true, id: 'src-opencode' })
    expect(stoa.sources().map((row) => row.id)).toContain('src-opencode')
  })

  it.each(['artemis', 'agent.artemis', 'researcher', 'orchestrator'])(
    'refuses "%s" — the Stoa can never widen its own reading list',
    (who) => {
      const { stoa, events } = rig()
      const outcome = stoa.register(DRAFT, who)
      expect(outcome.ok).toBe(false)
      expect(stoa.sources().map((row) => row.id)).not.toContain('src-opencode')
      // A refused curation attempt is an event in its own right (NFR-13).
      expect(events.some((e) => e.event === 'register-refused' && e.by === who)).toBe(true)
    }
  )

  it('stamps the registrar and the time itself — the caller supplies neither', () => {
    const { stoa } = rig()
    stoa.register(DRAFT, 'architect')
    const row = stoa.sources().find((entry) => entry.id === 'src-opencode')
    expect(row?.registeredBy).toBe('architect')
    expect(row?.registeredAt).toBe('2026-08-28T09:00:00.000Z')
  })

  it('refuses a draft the schema does not accept, without writing anything', () => {
    const { stoa } = rig()
    const before = stoa.sources().length
    expect(stoa.register({ ...DRAFT, url: 'not-a-url' }, 'architect').ok).toBe(false)
    expect(stoa.register({ ...DRAFT, tags: [] }, 'architect').ok).toBe(false)
    expect(stoa.sources()).toHaveLength(before)
  })

  it('refuses a duplicate url rather than registering the same repo twice', () => {
    const { stoa } = rig()
    stoa.register(DRAFT, 'architect')
    const again = stoa.register(DRAFT, 'architect')
    expect(again.ok).toBe(false)
    if (!again.ok) expect(again.reason).toContain('already registered')
  })

  it('registers unpinned — and the entry is then not studiable (FR-13.2)', () => {
    const { stoa } = rig()
    stoa.register(DRAFT, 'architect')
    expect(stoa.sources().find((row) => row.id === 'src-opencode')?.pin).toBeNull()
  })

  it('logs the registration with the provenance chain', () => {
    const { stoa, events } = rig()
    stoa.register(DRAFT, 'architect')
    expect(events.find((e) => e.event === 'registered')).toMatchObject({
      kind: 'stoa',
      sourceId: 'src-opencode',
      by: 'architect',
      license: 'unverified',
      pin: null
    })
  })
})

describe('retiring a source (FR-13.1)', () => {
  it('moves the entry to the retired list, verbatim — never deletes it', () => {
    const { stoa, agoraRoot } = rig()
    const before = stoa.sources().find((row) => row.id === 'src-munder-difflin')
    expect(stoa.retire('src-munder-difflin', 'architect')).toEqual({
      ok: true,
      id: 'src-munder-difflin'
    })
    const list = parseWatchlist(
      fs.readFileSync(path.join(agoraRoot, 'stoa', 'watchlist.json'), 'utf8')
    )
    expect(list.ok).toBe(true)
    if (!list.ok) return
    expect(list.watchlist.sources.map((row) => row.id)).not.toContain('src-munder-difflin')
    // Verbatim: a brief that cites this source must still resolve to what it
    // was when the study ran.
    expect(list.watchlist.retired).toEqual([before])
  })

  it('takes the source out of the studiable set by construction', () => {
    // `sources` is the studiable set. A retired entry is not filtered out of
    // it — it is not IN it — so a caller that forgets to filter cannot study
    // a retired source.
    const { stoa } = rig()
    stoa.retire('src-munder-difflin', 'architect')
    expect(stoa.sources().map((row) => row.id)).toEqual(['src-hermes-agent'])
  })

  it.each(['artemis', 'researcher'])('refuses "%s"', (who) => {
    const { stoa, events } = rig()
    expect(stoa.retire('src-munder-difflin', who).ok).toBe(false)
    expect(stoa.sources().map((row) => row.id)).toContain('src-munder-difflin')
    expect(events.some((e) => e.event === 'retire-refused' && e.by === who)).toBe(true)
  })

  it('says so rather than pretending, for an unknown or already-retired id', () => {
    const { stoa } = rig()
    const unknown = stoa.retire('src-nope', 'architect')
    expect(unknown.ok).toBe(false)
    if (!unknown.ok) expect(unknown.reason).toContain('no source')
    stoa.retire('src-hermes-agent', 'architect')
    const again = stoa.retire('src-hermes-agent', 'architect')
    expect(again.ok).toBe(false)
    if (!again.ok) expect(again.reason).toContain('already retired')
  })

  it('never loses an entry across register and retire', () => {
    const { stoa } = rig()
    stoa.register(DRAFT, 'architect')
    stoa.retire('src-hermes-agent', 'architect')
    stoa.retire('src-opencode', 'architect')
    const list = stoa.watchlist()
    expect([...list.sources, ...list.retired]).toHaveLength(3)
  })

  it('re-registering a retired url mints a fresh id rather than reusing one', () => {
    const { stoa } = rig()
    stoa.retire('src-munder-difflin', 'architect')
    const again = stoa.register(
      {
        url: 'https://github.com/chaitanyagiri/munder-difflin',
        tags: ['x'],
        license: 'MIT',
        pin: null,
        notes: ''
      },
      'architect'
    )
    // The retired row keeps `src-munder-difflin`, so the new registration
    // cannot silently inherit the citations that pointed at the old one.
    expect(again).toEqual({ ok: true, id: 'src-munder-difflin-2' })
  })
})

describe('a damaged watchlist', () => {
  it('is reported and read as empty — and is never overwritten', () => {
    const { stoa, agoraRoot, degraded } = rig()
    stoa.sources()
    const file = path.join(agoraRoot, 'stoa', 'watchlist.json')
    fs.writeFileSync(file, '{ "schemaVersion": 1, "sources": [ { "id": "nope" } ] }', 'utf8')
    expect(stoa.sources()).toEqual([])
    expect(degraded.some((d) => d.includes('not a valid watchlist'))).toBe(true)
    // The file may be the only copy of what was registered; a boot that
    // "repaired" it by truncating would destroy the record it could not read.
    expect(fs.readFileSync(file, 'utf8')).toContain('"nope"')
  })

  it('refuses to register or retire over it, and leaves every byte where it was', () => {
    // Until 2026-10-11 the writers took `watchlist()`'s empty answer as the
    // truth. `register` wrote a one-entry file over everything it could not
    // read — on a home seeded with the 166-character License cell, one
    // registration left only the new source — and `retire` answered "no
    // source on the watchlist", which was false. Both now refuse, and say how
    // to recover.
    const { stoa, agoraRoot, events } = rig()
    stoa.sources()
    const file = path.join(agoraRoot, 'stoa', 'watchlist.json')
    const valid = parseWatchlist(fs.readFileSync(file, 'utf8'))
    if (!valid.ok) throw new Error('the rig seeds a valid watchlist')
    // The real shape of the defect: a seeded file whose one fault is a cell
    // over the schema's cap.
    const damaged = {
      ...valid.watchlist,
      sources: valid.watchlist.sources.map((row, n) =>
        n === 0 ? { ...row, license: `unverified (${'a reading '.repeat(15)})` } : row
      )
    }
    fs.writeFileSync(file, `${JSON.stringify(damaged, null, 2)}\n`, 'utf8')
    const before = fs.readFileSync(file)

    const registered = stoa.register(DRAFT, 'architect')
    const retired = stoa.retire('src-munder-difflin', 'architect')

    expect(registered.ok).toBe(false)
    expect(retired.ok).toBe(false)
    for (const outcome of [registered, retired]) {
      if (outcome.ok) continue
      expect(outcome.reason).toContain(file)
      expect(outcome.reason).toContain('sources.0.license')
      expect(outcome.reason).toContain('aside and restart')
    }
    if (!registered.ok) expect(registered.reason).toContain('refuses to register')
    if (!retired.ok) expect(retired.reason).toContain('refuses to retire')
    expect(fs.readFileSync(file).equals(before)).toBe(true)
    // A refused curation is an event in its own right (NFR-13).
    expect(events.filter((e) => e.event === 'register-refused')).toHaveLength(1)
    expect(events.filter((e) => e.event === 'retire-refused')).toHaveLength(1)
    expect(events.some((e) => e.event === 'registered' || e.event === 'retired')).toBe(false)
  })

  it('still refuses a non-Architect first, before it reads the file at all', () => {
    const { stoa, agoraRoot } = rig()
    stoa.sources()
    fs.writeFileSync(path.join(agoraRoot, 'stoa', 'watchlist.json'), '{ nope', 'utf8')
    const outcome = stoa.register(DRAFT, 'artemis')
    expect(outcome.ok).toBe(false)
    if (!outcome.ok) expect(outcome.reason).toContain('R1')
  })
})

describe('briefs are evidence, and immutable (FR-13.4)', () => {
  it('answers null for a brief that is not on file', () => {
    const { stoa } = rig()
    expect(stoa.brief('RB-404')).toBeNull()
  })

  it('exposes no way to write one — the archive is read-only from here', () => {
    const { stoa } = rig()
    const surface = Object.getOwnPropertyNames(Object.getPrototypeOf(stoa)).sort()
    // An exhaustive list on purpose: it fails the day somebody adds a method,
    // which forces the question "what writes the archive now?" to be answered
    // deliberately rather than noticed later. It has already earned its keep —
    // M5b.2 added `fileBrief`, and this assertion is where that was declared.
    //
    // `fileBrief` is the ONLY writer, and it is write-once: a second filing
    // under an existing name is refused, never merged (FR-13.4), because the
    // proposals citing a brief must keep resolving to the words their author
    // read.
    expect(surface).toEqual(
      [
        'brief',
        'briefs',
        'briefsDir',
        'constructor',
        'fileBrief',
        'instructionsFor',
        'plan',
        // `readWatchlist` and `unreadable` (2026-10-11) write nothing: they
        // are how `register` and `retire` learn the file is unreadable and
        // refuse, instead of writing over it.
        'readWatchlist',
        'refuse',
        'refuseBrief',
        'register',
        'retire',
        'seed',
        'sources',
        'stoaDir',
        'unreadable',
        'watchlist',
        'watchlistPath',
        'write'
      ].sort()
    )
  })
})
