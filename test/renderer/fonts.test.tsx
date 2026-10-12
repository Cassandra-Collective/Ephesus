// @vitest-environment jsdom
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { PIXEL_FACES } from '../../src/renderer/src/fonts'

/**
 * A pixel face that did not load is SHOWN on the status strip (#11).
 *
 * UI-DESIGN §3 requires the three faces bundled, and `scripts/sync-fonts.cjs`
 * lets an install finish without one on purpose, because `fonts.ts` is meant to
 * report a missing face rather than let the app render in a fallback nobody can
 * name (BUILD-PROMPT §3.7). Nothing proved that report ever reached the screen.
 * These tests mount the shell and read the strip and its `missing:` title, along
 * the production call path: `main.tsx:11` renders `App`, `App.tsx:142` runs
 * `loadPixelFonts`, and `App.tsx:389–399` draws the result. A loader that works
 * behind a strip that never shows it fails here.
 *
 * `loadPixelFonts` can lose a face four ways, and each has a case: the response
 * is not ok, the bytes are not WOFF2, `fetch` rejects, or `FontFace.load()`
 * rejects. Faces settle in any order, so names are compared as sets.
 *
 * Children are replaced only where they would get in the way, as in
 * `bridge-heartbeat.test.tsx`: `FloorCanvas` is Pixi and renders on the default
 * tab; `AgentPanel` pulls in xterm through `TerminalPanel`; `AgentDock` and
 * `CommandBar` render only on a ready bridge, which no test here installs; and
 * `AutonomyBadge` sits inside the strip and would add text of its own.
 * `ConsentGate` stays real: without a bridge it renders nothing.
 */

vi.mock('../../src/renderer/src/floor/FloorCanvas', () => ({ FloorCanvas: () => null }))
vi.mock('../../src/renderer/src/AgentPanel', () => ({ AgentPanel: () => null }))
vi.mock('../../src/renderer/src/AgentDock', () => ({ AgentDock: () => null }))
vi.mock('../../src/renderer/src/CommandBar', () => ({ CommandBar: () => null }))
vi.mock('../../src/renderer/src/AutonomyBadge', () => ({ AutonomyBadge: () => null }))

const { App } = await import('../../src/renderer/src/App')

let root: Root | null = null
let host: HTMLDivElement
const originalFonts = Object.getOwnPropertyDescriptor(document, 'fonts')

/** `wOF2`, the signature `fonts.ts` checks for. */
const WOFF2 = [0x77, 0x4f, 0x46, 0x32]
/** `<htm`: what a dev server sends for a missing file, the SPA's index.html. */
const HTML = [0x3c, 0x68, 0x74, 0x6d]
/** `wOFF`: WOFF1, what a renamed `.woff` delivers. Its first three bytes are WOFF2's. */
const WOFF1 = [0x77, 0x4f, 0x46, 0x46]

interface FontFailureOptions {
  unavailable?: string
  /** The first bytes every font request answers with; WOFF2's when absent. */
  signature?: readonly number[]
  rejectFetch?: string
  rejectLoad?: string
}

function stubFontLoading(options: FontFailureOptions = {}) {
  const added: string[] = []
  class FakeFontFace {
    constructor(
      readonly family: string,
      readonly source: ArrayBuffer
    ) {}
    async load(): Promise<FakeFontFace> {
      if (this.family === options.rejectLoad) throw new Error('FontFace.load failed')
      return this
    }
  }

  vi.stubGlobal('FontFace', FakeFontFace)
  Object.defineProperty(document, 'fonts', {
    configurable: true,
    value: { add: (font: FakeFontFace) => added.push(font.family) }
  })

  const fontBytes = new Uint8Array(options.signature ?? WOFF2).buffer
  const fetchMock = vi.fn(async (file: string) => {
    if (file === options.rejectFetch) throw new Error('Network failure')
    return {
      ok: !options.unavailable || !file.includes(options.unavailable),
      arrayBuffer: async () => fontBytes
    }
  })
  vi.stubGlobal('fetch', fetchMock)
  return { added, fetchMock }
}

async function mount(): Promise<void> {
  host = document.createElement('div')
  document.body.append(host)
  root = createRoot(host)
  await act(async () => {
    root?.render(<App />)
  })
}

/** The status strip: the `<header>` App renders, not the whole window. */
function header(): HTMLElement {
  const found = host.querySelector('header')
  if (!found) throw new Error('App rendered no <header>, so there is no status strip to read')
  return found
}

const strip = (): string => header().textContent ?? ''

function missingNames(): string[] {
  const title = header().querySelector('span[title^="missing:"]')?.getAttribute('title')
  return (
    title
      ?.replace(/^missing: /, '')
      .split(', ')
      .sort() ?? []
  )
}

afterEach(() => {
  act(() => root?.unmount())
  host?.remove()
  root = null
  vi.unstubAllGlobals()
  if (originalFonts) Object.defineProperty(document, 'fonts', originalFonts)
  else Reflect.deleteProperty(document, 'fonts')
})

describe('renderer pixel-font degradation (#11)', () => {
  it('loads all bundled faces and shows the healthy strip state', async () => {
    const { added, fetchMock } = stubFontLoading()
    await mount()

    expect(fetchMock).toHaveBeenCalledTimes(PIXEL_FACES.length)
    expect([...added].sort()).toEqual(PIXEL_FACES.map((face) => face.family).sort())
    expect(strip()).toContain('fonts: bundled')
    expect(strip()).not.toContain('pixel faces missing')
  })

  it('reports the missing face by name in the visible strip', async () => {
    const missing = PIXEL_FACES[1]
    if (!missing) throw new Error('Expected three bundled font definitions')
    const { added } = stubFontLoading({ unavailable: missing.file })
    await mount()

    expect(added).toHaveLength(PIXEL_FACES.length - 1)
    expect(added).not.toContain(missing.family)
    expect(strip()).toContain(`fonts: 1 of ${PIXEL_FACES.length} pixel faces missing`)
    expect(missingNames()).toEqual([missing.family])
    expect(strip()).not.toContain('fonts: bundled')
  })

  it('shows all missing faces when the server returns non-WOFF2 content', async () => {
    const { added } = stubFontLoading({ signature: HTML })
    await mount()

    expect(added).toHaveLength(0)
    expect(strip()).toContain(
      `fonts: ${PIXEL_FACES.length} of ${PIXEL_FACES.length} pixel faces missing`
    )
    expect(missingNames()).toEqual(PIXEL_FACES.map((face) => face.family).sort())
    expect(strip()).not.toContain('fonts: bundled')
  })

  it('reports a WOFF1 file as missing, though it shares three bytes with WOFF2', async () => {
    // `<htm` above shares no byte with `wOF2`, so it cannot tell the real check
    // from one weakened to `some`, or to the first byte. `wOFF` differs only in
    // the fourth byte, and both weakened checks would let it through.
    const { added } = stubFontLoading({ signature: WOFF1 })
    await mount()

    expect(added).toHaveLength(0)
    expect(strip()).toContain(
      `fonts: ${PIXEL_FACES.length} of ${PIXEL_FACES.length} pixel faces missing`
    )
    expect(missingNames()).toEqual(PIXEL_FACES.map((face) => face.family).sort())
    expect(strip()).not.toContain('fonts: bundled')
  })

  it('displays a missing face when fetch rejects without rejecting the app render', async () => {
    const missing = PIXEL_FACES[0]
    if (!missing) throw new Error('Expected three bundled font definitions')
    const { added, fetchMock } = stubFontLoading({ rejectFetch: missing.file })
    await mount()

    expect(fetchMock).toHaveBeenCalledTimes(PIXEL_FACES.length)
    expect(added).toHaveLength(PIXEL_FACES.length - 1)
    expect(added).not.toContain(missing.family)
    expect(strip()).toContain(`fonts: 1 of ${PIXEL_FACES.length} pixel faces missing`)
    expect(missingNames()).toEqual([missing.family])
    expect(strip()).not.toContain('fonts: bundled')
  })

  it('reports a failed FontFace.load with valid WOFF2 bytes', async () => {
    const missing = PIXEL_FACES[2]
    if (!missing) throw new Error('Expected three bundled font definitions')
    const { added, fetchMock } = stubFontLoading({ rejectLoad: missing.family })
    await mount()

    expect(fetchMock).toHaveBeenCalledTimes(PIXEL_FACES.length)
    expect(added).toHaveLength(PIXEL_FACES.length - 1)
    expect(added).not.toContain(missing.family)
    expect(strip()).toContain(`fonts: 1 of ${PIXEL_FACES.length} pixel faces missing`)
    expect(missingNames()).toEqual([missing.family])
    expect(strip()).not.toContain('fonts: bundled')
  })
})
