import { inject, type InjectionKey } from 'vue'
import { isSandpostDesktopBridge } from '@/api/desktop'
import type { SandpostClient } from '@/api/contracts'
import { createHttpClient } from './http'
import { createMockClient } from '@/mocks/client'
import { createElectronClient } from './electron'

export type SandpostMode = 'http' | 'mock' | 'electron'

/**
 * Build-time mode; when it is not `mock`, the mock client and its fixtures are dropped from the bundle.
 */
const configuredMode =
  import.meta.env.VITE_SANDPOST_MODE ?? import.meta.env.MODE

/**
 * Resolve runtime selection, with the Electron bridge taking precedence over build mode.
 */
export function selectSandpostMode(
  hasElectronBridge: boolean,
  configuredMode: string | undefined
): SandpostMode {
  if (hasElectronBridge) return 'electron'
  return configuredMode === 'mock' ? 'mock' : 'http'
}

/**
 * Create the selected transport once for the application.
 */
export function createSandpostClient(): SandpostClient {
  const exposedBridge =
    typeof window !== 'undefined' ? window.sandpost : undefined
  if (
    exposedBridge?.runtime === 'electron' &&
    !isSandpostDesktopBridge(exposedBridge)
  ) {
    throw new Error(
      'The Electron preload does not provide the required authentication and mail API.'
    )
  }
  const bridge = isSandpostDesktopBridge(exposedBridge)
    ? exposedBridge
    : undefined
  switch (selectSandpostMode(bridge !== undefined, configuredMode)) {
    case 'electron':
      return createElectronClient(bridge!)
    case 'mock':
      return configuredMode === 'mock' ? createMockClient() : createHttpClient()
    default:
      return createHttpClient()
  }
}

export const sandpostClientKey: InjectionKey<SandpostClient> =
  Symbol('SandpostClient')

/**
 * Return the application-wide Sandpost transport.
 */
export function useSandpostClient(): SandpostClient {
  const client = inject(sandpostClientKey)
  if (!client) throw new Error('SandpostClient was not provided')
  return client
}
