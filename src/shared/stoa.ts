import { z } from 'zod'

/**
 * The Stoa (ADR-0017, FR-13, SDD §4.7/§7.7, UC-14).
 *
 * The colonnade where the city's scholars taught — the company's research
 * department, and the one place external evidence may enter the Gymnasium's
 * loop. FR-12.1 admits only recorded evidence into a proposal; without the
 * Stoa the company can learn from its own friction but is structurally blind
 * to every other harness solving the same problems.
 *
 * ADR-0017's four hard rules are what this module is for. Two of them live
 * here, in code:
 *
 * - **R1 — sources are Architect-registered only.** The Stoa can never widen
 *   its own reading list. That is the FR-12.3 mirror one subsystem over: a
 *   researcher that can add its own sources has the same authority problem as
 *   a proposer that can approve its own proposal.
 * - **R3 — a brief is evidence, never a change.** Briefs archive immutably and
 *   are cited by id; nothing here writes code, prompts or config.
 *
 * The other two are enforced where they can be: R2 (studied content is data,
 * never instructions) in the researcher spawn plan and its prompt, and R4 (the
 * budget slice) by the Gymnasium's existing accounting.
 *
 * The watchlist file is `agora/stoa/watchlist.json` and its ENTRY shape is
 * SDD §4.7 verbatim — `id`, `pin` and `license` are the provenance chain that
 * makes a brief's claims checkable, so nothing is invented alongside them.
 */

export const STOA_SCHEMA_VERSION = 1

/**
 * v1 studies git repositories only.
 *
 * The field exists anyway because §4.7 says so: it leaves room for non-git
 * sources later without re-opening the governance, and a source with no
 * pinnable commit is exactly the kind ADR-0017 deferred.
 */
export const SOURCE_KINDS = ['git'] as const

export const sourceKindSchema = z.enum(SOURCE_KINDS)

export type SourceKind = z.infer<typeof sourceKindSchema>

export const sourceIdSchema = z
  .string()
  .min(1)
  .max(64)
  .regex(/^src-[a-z0-9][a-z0-9-]*$/, 'a source id like src-hermes-agent')

export const briefIdSchema = z
  .string()
  .min(1)
  .max(32)
  .regex(/^RB-\d{3,}$/, 'a brief id like RB-001')

/**
 * One watchlist entry — SDD §4.7, field for field.
 *
 * `license: "unverified"` is a legal value and NOT a defect: it permits study
 * and refuses pattern intake (FR-13.5). Recording "we have not checked" is the
 * honest state for a source the Architect has just pasted in, and a schema that
 * refused it would push people to guess a license instead.
 */
export const watchlistEntrySchema = z
  .object({
    id: sourceIdSchema,
    url: z.string().min(1).max(500).url(),
    kind: sourceKindSchema,
    /** What the Architect wants learned. Tags SCOPE every study — FR-13.2. */
    tags: z.array(z.string().min(1).max(64)).min(1).max(16),
    /**
     * As verified at registration; "unverified" ⇒ study yes, intake no. Free
     * text on purpose (narrowing it would refuse files already on disk); what
     * it PERMITS is `licenseStanding`'s question, and only a value on
     * `INTAKE_LICENSES` permits intake.
     */
    license: z.string().min(1).max(120),
    /**
     * The commit every study runs against, or null for "registered, not yet
     * studiable". §4.7 types this as a string because it describes a source
     * that has been studied; the build-phase watchlist carries "(set at first
     * study)" for the rest, and FR-13.2 requires a PINNED snapshot — so an
     * unpinned entry must be representable and must not be studiable.
     */
    pin: z.string().min(1).max(64).nullable(),
    /** Only ever "architect" (FR-13.1). The type has one inhabitant on purpose. */
    registeredBy: z.literal('architect'),
    registeredAt: z.string().min(1).max(64),
    notes: z.string().max(4_000)
  })
  .strict()

export type WatchlistEntry = z.infer<typeof watchlistEntrySchema>

/**
 * The watchlist file.
 *
 * `retired` is a sibling of `sources`, not a flag on an entry, and that is a
 * deliberate structural choice: `sources` then contains ONLY studiable sources
 * by construction, so a consumer that forgets to filter cannot study a retired
 * one. It is the same idiom SDD §2 already uses for `inbox/` → `inbox/.done/`
 * — processed, never deleted. The retired entry is kept verbatim because the
 * ledger habit applies here too (a struck-through row, not a missing one).
 */
export const watchlistSchema = z
  .object({
    schemaVersion: z.literal(STOA_SCHEMA_VERSION),
    sources: z.array(watchlistEntrySchema).max(200),
    retired: z.array(watchlistEntrySchema).max(200)
  })
  .strict()

export type Watchlist = z.infer<typeof watchlistSchema>

export const EMPTY_WATCHLIST: Watchlist = {
  schemaVersion: STOA_SCHEMA_VERSION,
  sources: [],
  retired: []
}

/**
 * What the Architect types on the reading desk: a URL, tags, a license and
 * notes. The id, the registrar and the timestamp are supplied by main — the
 * renderer has no business naming any of them (invariant §2).
 */
export const registerDraftSchema = z
  .object({
    url: z.string().min(1).max(500).url(),
    tags: z.array(z.string().min(1).max(64)).min(1).max(16),
    license: z.string().min(1).max(120),
    pin: z.string().min(1).max(64).nullable(),
    notes: z.string().max(4_000)
  })
  .strict()

export type RegisterDraft = z.infer<typeof registerDraftSchema>

export type WatchlistParse =
  | { readonly ok: true; readonly watchlist: Watchlist }
  | { readonly ok: false; readonly reasons: readonly string[] }

/**
 * Contract: parses the watchlist file, or lists everything wrong with it.
 *
 * Every reason at once, for the same reason `parseGymProposal` does it: the
 * caller's only feedback is this list.
 */
export function parseWatchlist(body: string): WatchlistParse {
  let raw: unknown
  try {
    raw = JSON.parse(body)
  } catch (err) {
    return { ok: false, reasons: [`stoa: watchlist is not JSON — ${reason(err)}`] }
  }
  const parsed = watchlistSchema.safeParse(raw)
  if (parsed.success) return { ok: true, watchlist: parsed.data }
  return { ok: false, reasons: issueReasons(parsed.error.issues, 'watchlist') }
}

/**
 * Contract: the stable id for a source URL — `src-<repo-slug>`.
 *
 * Derived from the URL's last path segment so the same repository always gets
 * the same id, which is what makes a brief's `source` line resolvable years
 * later. Falls back to the host when the path is empty, and never returns the
 * bare prefix.
 */
export function sourceIdFor(url: string): string {
  const trimmed = url
    .trim()
    .replace(/\.git$/, '')
    .replace(/\/+$/, '')
  const tail =
    trimmed
      .split('/')
      .filter((part) => part.length > 0)
      .pop() ?? ''
  const slug = tail
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-|-$/g, '')
    .slice(0, 48)
  return slug.length > 0 ? `src-${slug}` : 'src-source'
}

/** Contract: a free id, given what is already registered or retired. */
export function uniqueSourceId(base: string, taken: readonly string[]): string {
  if (!taken.includes(base)) return base
  for (let n = 2; n < 1_000; n += 1) {
    const candidate = `${base}-${String(n)}`
    if (!taken.includes(candidate)) return candidate
  }
  throw new Error('stoa: could not mint a free source id')
}

/**
 * Who may curate the watchlist.
 *
 * One inhabitant, exactly like `GymDecider`: FR-13.1 gives the Stoa no word for
 * a second registrar, so a caller trying to express "the researcher added it"
 * has nothing to write down.
 */
export type StoaRegistrar = 'architect'

export interface RegistrarCheck {
  readonly allowed: boolean
  readonly because: string
}

/**
 * Contract: may this actor curate the watchlist? (R1, FR-13.1)
 *
 * The refusal is the point of the function. An agent MAY propose a source — in
 * a session report, through `/improve`, in a brief's candidates — and MAY NOT
 * register one, because a research department that widens its own reading list
 * has quietly appointed itself the authority on what the company should learn.
 */
export function checkRegistrar(who: string): RegistrarCheck {
  if (who !== 'architect') {
    return {
      allowed: false,
      because: `only the Architect may curate the Stoa watchlist; "${who}" may not (ADR-0017 R1). Agents propose sources; they never register them.`
    }
  }
  return { allowed: true, because: 'the Architect curates the watchlist' }
}

export interface StudyCheck {
  readonly allowed: boolean
  readonly because: string
}

/**
 * Contract: may this entry be studied right now? (FR-13.2)
 *
 * A study runs read-only over a PINNED snapshot, so an unpinned entry is not
 * studiable — the pin is what makes a finding's file citation resolve to the
 * same bytes the researcher read. Retired entries never reach here: they are
 * not in `sources` at all.
 */
export function checkStudiable(entry: WatchlistEntry): StudyCheck {
  if (entry.pin === null) {
    return {
      allowed: false,
      because: `${entry.id} has no pinned commit; a study runs against a pinned snapshot (FR-13.2)`
    }
  }
  return { allowed: true, because: `${entry.id} is pinned at ${entry.pin}` }
}

/**
 * The licenses pattern intake may come from (FR-13.5, DECISIONS-LOG 2026-10-11).
 *
 * An ALLOWLIST, because intake must fail closed. The rule this replaced refused
 * only the exact string `unverified`, so every other value read as a verified
 * license: a cell reading `unverified (read as AGPL-3.0 …)` permitted intake
 * from the very row it was written to refuse. Anything not on this list (a
 * reading, a guess, `TBD`, a copyleft license somebody did verify) now refuses.
 *
 * The list is MIT-COMPATIBLE identifiers, not every license that exists, for
 * the reason DD-M9-6 gave for the AGPL row: this repository is MIT, so code
 * under a license it cannot carry is refused "whatever verification finds".
 * Intake from a listed license still needs attribution and a decision memo
 * (ENGINEERING-STANDARDS §5); being listed is necessary, never sufficient.
 */
export const INTAKE_LICENSES = [
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
] as const

/**
 * What a watchlist license says about intake.
 *
 * Three answers rather than a boolean because the researcher is told different
 * things: `unverified` means nobody has checked, and `incompatible` means the
 * value is not one intake may come from — which must never be rendered as
 * "nobody has checked" when somebody has.
 */
export type LicenseStanding = 'permitted' | 'unverified' | 'incompatible'

/**
 * Contract: the standing of one license value. Pure; total over strings.
 *
 * Trimmed and compared ignoring case, as SPDX identifiers are; nothing else is
 * normalised, so `MIT (read 2026-10-03)` is not `MIT`.
 */
export function licenseStanding(license: string): LicenseStanding {
  const value = license.trim().toLowerCase()
  if (value === 'unverified') return 'unverified'
  return INTAKE_LICENSES.some((id) => id.toLowerCase() === value) ? 'permitted' : 'incompatible'
}

/**
 * Contract: may patterns from this source be taken into the codebase? (FR-13.5)
 *
 * Study is always allowed; INTAKE is not. An unverified license means the
 * company does not know what it is allowed to do with the code, and "we did not
 * check" is not a licence to copy. Each refusal names the list, because the
 * panel shows it (`intakeBlocked`) to the person who has to fix the entry.
 */
export function checkIntake(entry: WatchlistEntry): StudyCheck {
  const standing = licenseStanding(entry.license)
  if (standing === 'unverified') {
    return {
      allowed: false,
      because: `${entry.id}'s license is unverified; study is permitted, pattern intake is not (FR-13.5)`
    }
  }
  if (standing === 'incompatible') {
    return {
      allowed: false,
      because: `${entry.id}'s license "${entry.license.trim()}" is not one pattern intake may come from; study is permitted, intake needs exactly one of ${INTAKE_LICENSES.join(', ')} (FR-13.5). A reading of a license belongs in the notes, not the license field`
    }
  }
  return { allowed: true, because: `${entry.id} is ${entry.license.trim()}` }
}

/** Contract: the next brief id, given the ones already archived. Never reuses one. */
export function nextBriefId(ids: readonly string[]): string {
  const highest = ids.reduce((high, id) => {
    const parsed = Number.parseInt(id.slice('RB-'.length), 10)
    return Number.isNaN(parsed) ? high : Math.max(high, parsed)
  }, 0)
  return `RB-${String(highest + 1).padStart(3, '0')}`
}

/** A table row with a source id that did not become an entry, and every reason why. */
export interface RejectedRow {
  readonly id: string
  readonly reasons: readonly string[]
}

/** The build-phase table, read: entries the schema accepts, and the rows it did not. */
export interface WatchlistTable {
  readonly entries: readonly WatchlistEntry[]
  readonly rejected: readonly RejectedRow[]
}

/**
 * Contract: reads the build-phase watchlist table (FR-13.7). Every row whose id
 * cell is a source id comes back EITHER as an entry `watchlistEntrySchema`
 * accepted OR as a rejected row naming its reasons — never neither.
 *
 * `docs/stoa/WATCHLIST.md` is a markdown table a human maintains; the running
 * system's watchlist is JSON. This is the bridge, and it exists for the same
 * reason `parseLedger` does: the build-phase archive is the real archive, and
 * the product inherits it rather than starting empty on the day it ships.
 *
 * Lines whose id cell does not look like a source id are skipped — that covers
 * the header, the separator, and any prose the table grows around it. A row
 * that IS a source row and fails is reported instead (invariant §7): this
 * reader once returned whatever a cell held, and a 166-character License cell
 * reached the seed, which wrote a file every later read refused
 * (DECISIONS-LOG 2026-10-11).
 */
export function parseWatchlistMarkdown(markdown: string, registeredAt: string): WatchlistTable {
  const entries: WatchlistEntry[] = []
  const rejected: RejectedRow[] = []
  for (const line of markdown.split('\n')) {
    if (!line.startsWith('|')) continue
    const cells = line.split('|').map((cell) => cell.trim())
    // ['', id, source, tags, license, pin, notes, '']
    const id = cells[1] ?? ''
    if (!/^src-[a-z0-9][a-z0-9-]*$/.test(id)) continue
    // Only an http(s) url is taken out of the Source cell. Handing the schema
    // the raw cell instead would let `.url()` admit an `ftp:` link, or any
    // scheme a cell happened to hold.
    const url = /(https?:\/\/\S+)/.exec(cells[2] ?? '')?.[1]
    if (url === undefined) {
      rejected.push({ id, reasons: ['url: the Source cell holds no http(s) url'] })
      continue
    }
    // An empty Tags cell is the schema's to refuse (`tags` needs one): tags
    // scope every study (FR-13.2), so a row without them is not a source.
    const tags = (cells[3] ?? '')
      .split(',')
      .map((tag) => tag.replace(/`/g, '').trim())
      .filter((tag) => tag.length > 0)
    // The pin cell is either a backticked sha or the "(set at first study)"
    // placeholder; anything that is not a sha means "not yet pinned".
    const pin = /`([0-9a-f]{7,40})`/i.exec(cells[5] ?? '')?.[1] ?? null
    const candidate: WatchlistEntry = {
      id,
      url,
      kind: 'git',
      tags,
      license: (cells[4] ?? '').replace(/`/g, '').trim() || 'unverified',
      pin,
      registeredBy: 'architect',
      // The build-phase table records no registration date. The seed run's
      // timestamp is the honest answer — the entry entered the RUNNING
      // system now — and the `seeded` log event says where it came from.
      registeredAt,
      notes: cells[6] ?? ''
    }
    // The TYPE above is what this reader used to return, and the type cannot
    // say "at most 120 characters". The schema can, so it decides.
    const checked = watchlistEntrySchema.safeParse(candidate)
    if (checked.success) entries.push(checked.data)
    else rejected.push({ id, reasons: issueReasons(checked.error.issues, 'entry') })
  }
  return { entries, rejected }
}

/** `path: message` for each issue — the shape every Stoa refusal reads in. */
export function issueReasons(
  issues: readonly { readonly path: readonly PropertyKey[]; readonly message: string }[],
  whole: string
): string[] {
  return issues.map((issue) => {
    const where = issue.path.length > 0 ? issue.path.map(String).join('.') : whole
    return `${where}: ${issue.message}`
  })
}

function reason(err: unknown): string {
  return err instanceof Error ? err.message : String(err)
}

/**
 * The plan for one study spawn (FR-13.2, NFR-17, SDD §7.7).
 *
 * A plan, not a spawn: this is a pure description that the caller executes, so
 * the properties NFR-17 cares about are assertable without running anything —
 * which is exactly what S-STOA asserts.
 *
 * Three of its fields are the security posture, and none of them is optional:
 *
 * - `envGrants` is `readonly []` and there is no parameter that could fill it.
 *   A researcher reads arbitrary third-party text; handing it a credential is
 *   the one mistake that turns a bad README into a breach.
 * - `readOnly` says the checkout is never written back. The study observes.
 * - `cwd` is under a scratch root that is NEVER the Agora or one of its
 *   worktrees. ADR-0004 gives the Agora exactly one working copy, and a
 *   checkout of somebody else's repository inside the company's own record is
 *   a category error before it is a risk.
 */
export interface StudyPlan {
  readonly sourceId: string
  readonly url: string
  /** The commit the study runs against; citations resolve inside it. */
  readonly commit: string
  /** Scratch checkout path — never the Agora, never an Agora worktree. */
  readonly cwd: string
  /** Always empty (NFR-17). There is no argument that changes this. */
  readonly envGrants: readonly string[]
  /** Always true — the study observes and never writes back. */
  readonly readOnly: boolean
  /** The study question, scoped by the entry's tags (FR-13.2). */
  readonly question: string
  /** What the license permits, carried into the brief's license note. */
  readonly license: string
  /** False unless the license is on `INTAKE_LICENSES`: study yes, pattern intake no. */
  readonly intakePermitted: boolean
}

export type StudyPlanOutcome =
  { readonly ok: true; readonly plan: StudyPlan } | { readonly ok: false; readonly reason: string }

/**
 * Contract: the read-only, secret-free plan for studying one source.
 *
 * Refuses rather than degrades (UC-14 alternate 2a): an unpinned source is not
 * studiable, and the refusal is visible instead of the study being silently
 * skipped. An unverified LICENSE is not a refusal — SDD §4.7 and TEST-STRATEGY
 * S-STOA both make study permitted and pattern intake refused, so the plan
 * carries `intakePermitted: false` and the study proceeds. So does any other
 * license intake may not come from (`checkIntake`).
 */
export function buildStudyPlan(entry: WatchlistEntry, root: string): StudyPlanOutcome {
  const studiable = checkStudiable(entry)
  if (!studiable.allowed || entry.pin === null) {
    return { ok: false, reason: studiable.because }
  }
  // Trailing separators trimmed here rather than inside the template below:
  // a regex in a `${}` is exactly the thing a later edit mis-escapes.
  const scratchRoot = trimTrailingSeparators(root)
  return {
    ok: true,
    plan: {
      sourceId: entry.id,
      url: entry.url,
      commit: entry.pin,
      // Deliberately joined here rather than taken as a parameter: a caller
      // that could choose the directory could choose the Agora.
      cwd: `${scratchRoot}/stoa/${entry.id}@${entry.pin}`,
      envGrants: [],
      readOnly: true,
      question:
        `Study ${entry.url} at ${entry.pin}, scoped to: ${entry.tags.join(', ')}. ${entry.notes}`.trim(),
      license: entry.license,
      intakePermitted: checkIntake(entry).allowed
    }
  }
}

/**
 * Contract: is this path safe to check a watched source out into?
 *
 * The Agora is the company's single working copy (ADR-0004) and `worktrees/`
 * holds isolated checkouts of the company's OWN targets. A third-party
 * repository belongs in neither, so this refuses both rather than trusting the
 * caller to have read the ADR.
 */
export function isSafeScratchPath(cwd: string, agoraRoot: string, worktreesRoot: string): boolean {
  // Split/join rather than a backslash regex: this compares Windows and POSIX
  // paths, and the escaping is the part a future edit gets wrong.
  const normal = (p: string): string => p.split('\\').join('/').replace(/\/+$/, '').toLowerCase()
  const target = normal(cwd)
  return (
    !target.startsWith(`${normal(agoraRoot)}/`) &&
    !target.startsWith(`${normal(worktreesRoot)}/`) &&
    target !== normal(agoraRoot) &&
    target !== normal(worktreesRoot)
  )
}

/** Contract: `<root>/` and `<root>` name the same directory. Trims either separator. */
function trimTrailingSeparators(p: string): string {
  let end = p.length
  while (end > 0 && (p[end - 1] === '/' || p[end - 1] === '\\')) end -= 1
  return p.slice(0, end)
}
