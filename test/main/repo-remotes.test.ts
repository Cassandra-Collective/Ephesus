import { execFileSync } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { ExecGitRunner, readRemotes, type GitRunner } from '../../src/main/git'
import { deriveRepo } from '../../src/shared/repo-remote'
import { removeTempDir } from '../tmpdir'

/**
 * Reading a checkout's remotes, against **real git in temp repositories**
 * (M8.5, and TEST-STRATEGY §6's rule that git's own bookkeeping is the
 * mechanism under test).
 *
 * The parser here is a contract with a program we do not own, and the M8.4
 * defect is why that matters: a matcher written against imagined output passed
 * forty-five tests while being unable to read a single byte the real tool
 * prints. `git remote -v` writes one line per remote per DIRECTION, and a
 * parser that did not know that would report `origin` twice and make a
 * one-remote checkout look like two.
 */

const temps: string[] = []

afterEach(() => {
  for (const dir of temps.splice(0)) removeTempDir(dir)
})

const git = (cwd: string, args: readonly string[]): void => {
  execFileSync('git', ['-c', 'user.name=T', '-c', 'user.email=t@t', ...args], { cwd })
}

function repoWith(remotes: readonly (readonly [string, string])[]): string {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'eph-remotes-'))
  temps.push(root)
  const repo = path.join(root, 'target')
  fs.mkdirSync(repo, { recursive: true })
  git(repo, ['init', '-q', '-b', 'main'])
  for (const [name, url] of remotes) git(repo, ['remote', 'add', name, url])
  return repo
}

const runner = new ExecGitRunner()

/**
 * The machine's git configuration is not part of the contract.
 *
 * Before git reads the repository's own config it reads the system file, the
 * user's global file and the environment, and any of them can change what
 * `git remote -v` prints: `url.<base>.insteadOf` rewrites a remote's URLs as
 * git reports them. On 2026-10-11 a container that injected
 * `url.https://github.com/.insteadOf=git@github.com:` through
 * `GIT_CONFIG_COUNT`/`GIT_CONFIG_KEY_0`/`GIT_CONFIG_VALUE_0` turned the
 * separate push URL below into a copy of the fetch URL, and the parser read
 * one remote, correctly, where the test expected two
 * (docs/implementations/2026-10-11-hermetic-git-remotes-test.md).
 *
 * So while a case below runs, git sees only the temp repository's config and
 * the `-c` flags each caller passes. The variables are stubbed on `process.env`
 * because `ExecGitRunner` passes no `env` to `execFile`: the runner and the
 * `git` helper both inherit it. Production keeps reading the user's config, as
 * it should, since a rewrite they configured applies to their real target.
 *
 *  - `GIT_CONFIG_NOSYSTEM` skips the system file, whatever `GIT_CONFIG_SYSTEM`
 *    names.
 *  - `GIT_CONFIG_GLOBAL` replaces both `~/.gitconfig` and
 *    `$XDG_CONFIG_HOME/git/config` (git 2.32 and later). It is spelled
 *    `/dev/null`, which Git for Windows maps for itself; `os.devNull` gives
 *    `\\.\nul` there, and git exits 128 on it.
 *  - The variables that carry config values themselves are deleted, including
 *    `GIT_CONFIG_PARAMETERS`, which is how a parent `git -c …` hands its flags
 *    to the processes it starts. The match is case-sensitive on purpose. Git
 *    for Windows would read `git_config_count` as `GIT_CONFIG_COUNT`, but
 *    vitest (4.1.11) copies every variable to its uppercase name when it
 *    builds a worker's environment on Windows, so no case here sees a
 *    lowercase one, and git on POSIX ignores them. Ignoring case would be a
 *    change no test could see.
 *
 * The stubs are made per case and undone per case: vitest's `unstubEnvs`
 * option, if it is ever turned on, clears stubs before every case, which would
 * silently undo stubs made once in a `beforeAll`.
 */
const CONFIG_VALUES_FROM_ENV = /^GIT_CONFIG_(COUNT|KEY_\d+|VALUE_\d+|PARAMETERS)$/

function isolateGitFromTheHost(): void {
  for (const name of Object.keys(process.env)) {
    if (CONFIG_VALUES_FROM_ENV.test(name)) vi.stubEnv(name, undefined)
  }
  vi.stubEnv('GIT_CONFIG_NOSYSTEM', '1')
  vi.stubEnv('GIT_CONFIG_GLOBAL', '/dev/null')
}

/**
 * The host that broke this file, played on every machine.
 *
 * A clean host — CI's runner, most laptops — gives the isolation nothing to
 * undo, so deleting it, or any one line of it, would leave every case green
 * there. Each case below therefore plays the hostile host first, through every
 * route git reads the same rewrite from, and the isolation must undo all of
 * them for the separate push URL to survive. Each route is enough on its own,
 * and the cases after this block check that one route at a time: a route git
 * stopped honouring would otherwise make this decoration that stays green.
 */
const REWRITE_FROM = 'git@github.com:'
const REWRITE_TO = 'https://github.com/'

interface HostileRoute {
  readonly via: string
  readonly env: (configFile: string) => Readonly<Record<string, string | undefined>>
}

const HOSTILE_ROUTES: readonly HostileRoute[] = [
  {
    via: 'GIT_CONFIG_COUNT',
    env: () => ({
      GIT_CONFIG_COUNT: '1',
      GIT_CONFIG_KEY_0: `url.${REWRITE_TO}.insteadOf`,
      GIT_CONFIG_VALUE_0: REWRITE_FROM
    })
  },
  {
    via: 'GIT_CONFIG_PARAMETERS',
    env: () => ({ GIT_CONFIG_PARAMETERS: `'url.${REWRITE_TO}.insteadOf'='${REWRITE_FROM}'` })
  },
  { via: 'a global file', env: (configFile) => ({ GIT_CONFIG_GLOBAL: configFile }) },
  {
    via: 'a system file',
    // A host that had already switched the system file off would make this
    // route inert, so the route switches it back on.
    env: (configFile) => ({ GIT_CONFIG_SYSTEM: configFile, GIT_CONFIG_NOSYSTEM: undefined })
  }
]

function actAsTheHostileHost(routes: readonly HostileRoute[]): void {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'eph-remotes-host-'))
  temps.push(root)
  const configFile = path.join(root, 'gitconfig')
  fs.writeFileSync(configFile, `[url "${REWRITE_TO}"]\n\tinsteadOf = ${REWRITE_FROM}\n`)
  for (const route of routes) {
    for (const [name, value] of Object.entries(route.env(configFile))) vi.stubEnv(name, value)
  }
}

/** A checkout whose one remote pushes to a different URL than it fetches from. */
function repoWithSeparatePushUrl(): string {
  const repo = repoWith([['origin', 'https://github.com/owner/app.git']])
  git(repo, ['remote', 'set-url', '--push', 'origin', 'git@github.com:owner/app.git'])
  return repo
}

describe('readRemotes, against git itself', () => {
  beforeEach(() => {
    actAsTheHostileHost(HOSTILE_ROUTES)
    isolateGitFromTheHost()
  })

  afterEach(() => {
    vi.unstubAllEnvs()
  })

  it('reads one remote once, not once per direction', async () => {
    // `git remote -v` prints `origin <url> (fetch)` AND `origin <url> (push)`.
    const repo = repoWith([['origin', 'https://github.com/owner/app.git']])
    const read = await readRemotes(runner, repo)
    if (!read.ok) throw new Error(read.because)
    expect(read.remotes).toEqual([{ name: 'origin', url: 'https://github.com/owner/app.git' }])
  })

  it('reads a fork’s two remotes, and the derivation then refuses', async () => {
    const repo = repoWith([
      ['origin', 'git@github.com:me/app.git'],
      ['upstream', 'https://github.com/canonical/app.git']
    ])
    const read = await readRemotes(runner, repo)
    if (!read.ok) throw new Error(read.because)
    expect(read.remotes.map((r) => r.name).sort()).toEqual(['origin', 'upstream'])

    const derived = deriveRepo(read.remotes)
    expect(derived.ok).toBe(false)
    if (derived.ok) throw new Error('unreachable')
    expect(derived.because).toContain('me/app')
    expect(derived.because).toContain('canonical/app')
  })

  it('reads a separate push URL as its own line', async () => {
    // A push URL that differs from the fetch URL is a real configuration, and
    // git prints it as a second line under the same remote name.
    const read = await readRemotes(runner, repoWithSeparatePushUrl())
    if (!read.ok) throw new Error(read.because)
    expect(read.remotes).toHaveLength(2)
    // Both name the same repository, so the derivation is not ambiguous.
    expect(deriveRepo(read.remotes)).toEqual({ ok: true, slug: 'owner/app', from: 'origin' })
  })

  it('reports an empty list for a repository with no remote', async () => {
    // Distinct from "could not read": a fresh `git init` is a repository, and
    // the derivation's sentence for it is different from git's error.
    const read = await readRemotes(runner, repoWith([]))
    expect(read).toEqual({ ok: true, remotes: [] })
    expect(deriveRepo([])).toEqual({ ok: false, because: 'the target has no git remote' })
  })

  it('says why when the target is not a repository at all, in git’s words', async () => {
    const plain = fs.mkdtempSync(path.join(os.tmpdir(), 'eph-notrepo-'))
    temps.push(plain)
    const read = await readRemotes(runner, plain)
    expect(read.ok).toBe(false)
    if (read.ok) throw new Error('unreachable')
    expect(read.because).toContain('git could not read')
    // git's own wording, not ours: it names the problem better than a guess.
    expect(read.because.toLowerCase()).toContain('not a git repository')
  })

  it('never throws on a directory that does not exist', async () => {
    const read = await readRemotes(runner, path.join(os.tmpdir(), 'eph-nope-does-not-exist'))
    expect(read.ok).toBe(false)
  })
})

/**
 * The hostile host's premise: each route, alone and with no isolation, does
 * make git print the push URL as a copy of the fetch URL. Without this, a
 * route git ignores would leave the block above proving nothing about the
 * isolation line that undoes it.
 */
describe('the hostile host, one route at a time', () => {
  afterEach(() => {
    vi.unstubAllEnvs()
  })

  it.each(HOSTILE_ROUTES)('rewrites the push URL through $via alone', async (route) => {
    actAsTheHostileHost([route])
    const read = await readRemotes(runner, repoWithSeparatePushUrl())
    if (!read.ok) throw new Error(read.because)
    expect(read.remotes).toEqual([{ name: 'origin', url: 'https://github.com/owner/app.git' }])
  })
})

/**
 * The lines `git remote -v` does NOT print, against an injected runner.
 *
 * Real git is the right rig for everything above — it is the program whose
 * output format is the contract. But real git will not print a malformed line
 * on demand, and the parser's anchors are what stop one being read as a remote.
 * Without this, every anchor in that regex is decoration: removing the
 * `(fetch)`/`(push)` requirement changes nothing about real output, so nothing
 * would ever notice it going.
 */
describe('readRemotes reads only what is a remote line', () => {
  const answering = (stdout: string): GitRunner => ({
    run: () => Promise.resolve({ ok: true, stdout, stderr: '', code: 0 })
  })

  it('ignores a line with no direction marker', async () => {
    const read = await readRemotes(
      answering(
        [
          'origin\thttps://github.com/owner/app.git (fetch)',
          'origin\thttps://github.com/owner/app.git (push)',
          // A hint, a warning, a `git` that grew a header — none of these are
          // remotes, and each would become one without the anchor.
          'hint: some advice about remotes',
          'upstream\thttps://github.com/other/app.git',
          '  ',
          'origin\thttps://github.com/owner/app.git (fetchx)'
        ].join('\n')
      ),
      '/anywhere'
    )
    if (!read.ok) throw new Error(read.because)
    expect(read.remotes).toEqual([{ name: 'origin', url: 'https://github.com/owner/app.git' }])
    // And the derivation is therefore unambiguous, rather than being handed a
    // second "repository" that git never claimed existed.
    expect(deriveRepo(read.remotes)).toEqual({ ok: true, slug: 'owner/app', from: 'origin' })
  })

  it('reads an empty answer as no remotes, not as a failure', async () => {
    expect(await readRemotes(answering(''), '/anywhere')).toEqual({ ok: true, remotes: [] })
  })
})
