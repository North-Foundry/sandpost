import { describe, expect, it } from 'vitest'
import {
  constrainInspectorHeight,
  createEmailPreviewDocument,
  visiblePreviewRectangle,
} from './previewLayout'

describe('constrainInspectorHeight', () => {
  it('reserves the preview and tools minima when there is enough room', () => {
    expect(constrainInspectorHeight(20, 500)).toBe(140)
    expect(constrainInspectorHeight(400, 500)).toBe(340)
    expect(constrainInspectorHeight(250, 500)).toBe(250)
  })

  it('scales both minima proportionally when available space is below 300px', () => {
    expect(constrainInspectorHeight(0, 150)).toBe(70)
    expect(constrainInspectorHeight(200, 150)).toBe(70)
    expect(constrainInspectorHeight(40, 150)).toBe(70)
    expect(constrainInspectorHeight(100, 0)).toBe(0)
  })
})

describe('visiblePreviewRectangle', () => {
  const rectangle = { x: 10, y: 20, width: 100, height: 50 }

  it('returns fully contained geometry across every clip', () => {
    expect(visiblePreviewRectangle(rectangle, [
      { x: 0, y: 0, width: 200, height: 100 },
      { x: 10.4, y: 20.4, width: 99.7, height: 49.7 },
    ])).toBe(rectangle)
  })

  it('rejects partial clipping, non-positive size, and non-finite geometry', () => {
    expect(visiblePreviewRectangle(rectangle, [{ x: 0, y: 0, width: 109, height: 100 }])).toBeNull()
    expect(visiblePreviewRectangle({ ...rectangle, width: 0 }, [])).toBeNull()
    expect(visiblePreviewRectangle({ ...rectangle, x: Number.NaN }, [])).toBeNull()
    expect(visiblePreviewRectangle(rectangle, [{ x: 0, y: 0, width: Number.POSITIVE_INFINITY, height: 100 }])).toBeNull()
  })
})

describe('createEmailPreviewDocument', () => {
  it('wraps malformed or hostile markup in the restrictive preview policy', () => {
    const document = createEmailPreviewDocument('<img src="https://tracker.test/pixel"><script>parent.pwned=true</script><img src="data:image/png;base64,AA=="><style>body{color:red}</style>')
    expect(document).toContain('default-src \'none\'; script-src \'none\'; connect-src \'none\'; img-src data:; style-src \'unsafe-inline\'; font-src \'none\'; media-src \'none\'; object-src \'none\'; frame-src \'none\'; form-action \'none\'; base-uri \'none\'')
    expect(document).toContain('<body><img src="https://tracker.test/pixel"><script>parent.pwned=true</script>')
    expect(document).toContain('<img src="data:image/png;base64,AA=="><style>body{color:red}</style></body></html>')
    expect(document).toContain('<meta name="viewport" content="width=device-width,initial-scale=1">')
  })
})
