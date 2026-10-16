/**
 * Copy text to the system clipboard.
 *
 * Inside Electron the main process writes the clipboard, so the renderer needs no clipboard
 * permission and the copy works even when focus sits in a sandboxed message frame. Browsers use
 * the asynchronous Clipboard API.
 */
export async function copyText(text: string): Promise<void> {
  const bridge = typeof window !== 'undefined' ? window.sandpost : undefined
  if (
    bridge?.runtime === 'electron' &&
    typeof bridge.clipboard?.writeText === 'function'
  ) {
    await bridge.clipboard.writeText(text)
    return
  }
  await navigator.clipboard.writeText(text)
}
