/**
 * Electron prefixes errors thrown by main-process IPC handlers with this wrapper text.
 */
const IPC_PREFIX = /^Error invoking remote method '[^']+': (?:[A-Za-z]*Error: )?/

/**
 * Return a capitalized API error detail or exception message, removing Electron's
 * IPC wrapper when present. Use the caller's fallback when no message is available.
 */
export function describeError(error: unknown, fallback: string): string {
  if (error && typeof error === 'object' && 'detail' in error && typeof error.detail === 'string' && error.detail) {
    return capitalize(error.detail)
  }

  if (error instanceof Error && error.message) return capitalize(error.message.replace(IPC_PREFIX, ''))
  return fallback
}

/**
 * Capitalize server messages, which are lowercase sentence fragments.
 */
function capitalize(message: string): string {
  return message.charAt(0).toUpperCase() + message.slice(1)
}

/**
 * Present a server message fragment as a capitalized sentence with final punctuation.
 */
export function asSentence(message: string): string {
  const text = capitalize(message.trim())
  return /[.!?]$/.test(text) ? text : `${text}.`
}
