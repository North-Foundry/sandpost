/**
 * Trim and ASCII-lowercase a complete inbox email address after validating its syntax.
 */
export function normalizeInboxAddress(value: string): string {
  if (/[^\x20-\x7e]/.test(value)) {
    throw new TypeError('Invalid inbox name or address')
  }
  const address = value.trim()
  const separator = address.indexOf('@')
  if (separator < 1 || separator !== address.lastIndexOf('@') || address.length > 254) {
    throw new TypeError('Invalid inbox name or address')
  }

  const local = address.slice(0, separator)
  const domain = address.slice(separator + 1)
  const atoms = local.split('.')
  const atomPattern = /^[A-Za-z0-9!#$%&'*+/=?^_`{|}~-]+$/
  const labels = domain.split('.')
  const labelPattern = /^[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?$/
  if (
    local.length > 64 || atoms.some((atom) => !atom || !atomPattern.test(atom)) ||
    domain.length > 253 || labels.some((label) => !labelPattern.test(label))
  ) {
    throw new TypeError('Invalid inbox name or address')
  }
  return address.toLowerCase()
}
