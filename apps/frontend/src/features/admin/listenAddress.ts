import type { SmtpServer } from '@/api/contracts'

/**
 * Return the raw runtime bind address, preserving wildcard hosts and port zero.
 */
export function configuredListenerAddress(server: Pick<SmtpServer, 'listenAddress' | 'listener'>): string | null {
  return server.listener?.configuredAddress ?? server.listenAddress
}

/**
 * Split a runtime `host:port` (IPv6 hosts are bracketed) into host and port.
 */
export function splitAddress(address: string | null): { host: string; port: string } {
  if (!address) return { host: '127.0.0.1', port: '' }
  const separator = address.lastIndexOf(':')
  return { host: address.slice(0, separator).replace(/^\[|\]$/g, ''), port: address.slice(separator + 1) }
}

/**
 * Return the host and port an SMTP client should connect to for a bound listener address.
 *
 * A wildcard bind (0.0.0.0 or ::) accepts connections on every interface, so the host the
 * browser used to reach Sandpost is the best guess for external clients; desktop and local
 * pages fall back to loopback.
 */
export function connectionAddress(address: string, pageHost = globalThis.location?.hostname ?? ''): { host: string; port: string } {
  const { host, port } = splitAddress(address)
  if (host !== '0.0.0.0' && host !== '::') return { host, port }
  const browserHost = pageHost.replace(/^\[|\]$/g, '')
  if (browserHost && browserHost !== 'app' && !browserHost.endsWith('.localhost')) return { host: browserHost, port }
  return { host: host === '::' ? '::1' : '127.0.0.1', port }
}
