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

function stubFontLoading(options: { unavailable?: string; invalidSignature?: boolean } = {}) {
  const added: string[] = []
  class FakeFontFace {
    constructor(readonly family: string, readonly source: ArrayBuffer) {}
    async load(): Promise<FakeFontFace> {
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
  const fetchMock = vi.fn(async (file: string) => ({
    ok: !options.unavailable || !file.includes(options.unavailable),
    arrayBuffer: async () => fontBytes
  }))
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
    expect(added).toEqual(PIXEL_FACES.map((face) => face.family))
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
    expect(host.querySelector('span[title^="missing:"]')?.getAttribute('title')).toBe(
      `missing: ${missing.family}`
    )
  })

  it('shows all missing faces when the server returns non-WOFF2 content', async () => {
    const { added } = stubFontLoading({ invalidSignature: true })
    await mount()

    expect(added).toHaveLength(0)
    expect(strip()).toContain(
      `fonts: ${PIXEL_FACES.length} of ${PIXEL_FACES.length} pixel faces missing`
    )
    expect(host.querySelector('span[title^="missing:"]')?.getAttribute('title')).toBe(
      `missing: ${PIXEL_FACES.map((face) => face.family).join(', ')}`
    )
    expect(strip()).not.toContain('fonts: bundled')
  })
})
