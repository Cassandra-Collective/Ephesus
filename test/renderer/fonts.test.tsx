// @vitest-environment jsdom
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { PIXEL_FACES } from '../../src/renderer/src/fonts'

vi.mock('../../src/renderer/src/floor/FloorCanvas', () => ({ FloorCanvas: () => null }))
vi.mock('../../src/renderer/src/AgentPanel', () => ({ AgentPanel: () => null }))
vi.mock('../../src/renderer/src/AgentDock', () => ({ AgentDock: () => null }))
vi.mock('../../src/renderer/src/CommandBar', () => ({ CommandBar: () => null }))
vi.mock('../../src/renderer/src/AutonomyBadge', () => ({ AutonomyBadge: () => null }))
vi.mock('../../src/renderer/src/ConsentGate', () => ({ ConsentGate: () => null }))

const { App } = await import('../../src/renderer/src/App')

let root: Root | null = null
let host: HTMLDivElement
const originalFonts = Object.getOwnPropertyDescriptor(document, 'fonts')

interface FontFailureOptions {
  unavailable?: string
  invalidSignature?: boolean
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

  const fontBytes = options.invalidSignature
    ? new Uint8Array([60, 104, 116, 109]).buffer
    : new Uint8Array([0x77, 0x4f, 0x46, 0x32]).buffer
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

const strip = (): string => host.textContent ?? ''

function missingNames(): string[] {
  const title = host.querySelector('span[title^="missing:"]')?.getAttribute('title')
  return title?.replace(/^missing: /, '').split(', ').sort() ?? []
}

afterEach(() => {
  act(() => root?.unmount())
  host?.remove()
  root = null
  vi.unstubAllGlobals()
  if (originalFonts) Object.defineProperty(document, 'fonts', originalFonts)
  else Reflect.deleteProperty(document, 'fonts')
  delete (window as { eph?: unknown }).eph
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
  })

  it('shows all missing faces when the server returns non-WOFF2 content', async () => {
    const { added } = stubFontLoading({ invalidSignature: true })
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
