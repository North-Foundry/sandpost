import { createSharedComposable, useDark, useToggle } from '@vueuse/core'

/**
 * Shares one persisted light/dark preference, defaulting to the system appearance.
 */
export const useTheme = createSharedComposable(() => {
  const isDark = useDark({
    attribute: 'data-theme',
    valueDark: 'dark',
    valueLight: 'light',
    storageKey: 'sandpost.theme',
  })

  return { isDark, toggleTheme: useToggle(isDark) }
})
