/**
 * Electron on macOS draws the traffic lights over the top-left of the renderer instead of a title bar.
 */
export const insetTitleBar =
  typeof window !== 'undefined' && window.sandpost?.titleBar === 'inset'
