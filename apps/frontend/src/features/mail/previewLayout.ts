/**
 * Rectangle geometry used to position native preview tools against the renderer.
 */
export interface PreviewRectangle {
  x: number
  y: number
  width: number
  height: number
}

const PREVIEW_POLICY = "default-src 'none'; script-src 'none'; connect-src 'none'; img-src data:; style-src 'unsafe-inline'; font-src 'none'; media-src 'none'; object-src 'none'; frame-src 'none'; form-action 'none'; base-uri 'none'"

/**
 * Clamp the inspector height while reserving space for both preview and tools.
 */
export function constrainInspectorHeight(requested: number, available: number): number {
  if (!Number.isFinite(available) || available <= 0) return 0
  const scale = Math.min(1, available / 300)
  const previewMinimum = 160 * scale
  const toolsMinimum = 140 * scale
  const minimum = toolsMinimum
  const maximum = available - previewMinimum
  const desired = Number.isFinite(requested) ? requested : minimum
  return Math.max(minimum, Math.min(maximum, desired))
}

/**
 * Return a rectangle only when it remains fully visible within every clip.
 */
export function visiblePreviewRectangle(
  rectangle: PreviewRectangle,
  clippingRectangles: PreviewRectangle[],
): PreviewRectangle | null {
  /**
   * Accept finite rectangle coordinates and strictly positive dimensions before clipping.
   */
  const isValid = ({ x, y, width, height }: PreviewRectangle): boolean =>
    [x, y, width, height].every(Number.isFinite) && width > 0 && height > 0

  if (!isValid(rectangle)) return null

  const tolerance = 0.5
  const right = rectangle.x + rectangle.width
  const bottom = rectangle.y + rectangle.height
  if (![right, bottom].every(Number.isFinite)) return null
  for (const clipping of clippingRectangles) {
    if (!isValid(clipping)) return null
    const clippingRight = clipping.x + clipping.width
    const clippingBottom = clipping.y + clipping.height
    if (![clippingRight, clippingBottom].every(Number.isFinite)) return null
    if (rectangle.x < clipping.x - tolerance
      || rectangle.y < clipping.y - tolerance
      || right > clippingRight + tolerance
      || bottom > clippingBottom + tolerance) return null
  }
  return rectangle
}

/**
 * Wrap captured markup with the restrictive policy shared by browser and native previews.
 */
export function createEmailPreviewDocument(html: string): string {
  return `<!doctype html><html><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="${PREVIEW_POLICY}"><meta name="viewport" content="width=device-width,initial-scale=1"></head><body>${html}</body></html>`
}
