import { effectScope } from 'vue'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { COPY_CONFIRMATION_DURATION, useCopy } from './useCopy'

afterEach(() => {
  vi.useRealTimers()
  vi.unstubAllGlobals()
})

/**
 * Run `useCopy` inside a disposable scope, as a component's setup would.
 */
function setupCopy() {
  const scope = effectScope()
  const state = scope.run(() => useCopy<'a' | 'b'>('Copy failed'))!
  return { ...state, scope }
}

describe('useCopy', () => {
  it('confirms the copied target for a short time through the browser clipboard', async () => {
    vi.useFakeTimers()
    const writeText = vi.fn(async () => { })
    vi.stubGlobal('navigator', { clipboard: { writeText } })
    const { copied, error, copy } = setupCopy()

    expect(await copy('value', 'a')).toBe(true)
    expect(writeText).toHaveBeenCalledWith('value')
    expect(copied.value).toBe('a')
    expect(error.value).toBe('')
    vi.advanceTimersByTime(COPY_CONFIRMATION_DURATION - 1)
    expect(copied.value).toBe('a')
    vi.advanceTimersByTime(1)
    expect(copied.value).toBeNull()
  })

  it('restarts the confirmation when another target is copied', async () => {
    vi.useFakeTimers()
    vi.stubGlobal('navigator', {
      clipboard: {
        /**
         * Accept a clipboard write without accessing the operating system in this test.
         */
        writeText: async () => { }
      }
    })
    const { copied, copy } = setupCopy()

    await copy('first', 'a')
    vi.advanceTimersByTime(COPY_CONFIRMATION_DURATION - 100)
    await copy('second', 'b')
    vi.advanceTimersByTime(100)
    expect(copied.value).toBe('b')
    vi.advanceTimersByTime(COPY_CONFIRMATION_DURATION)
    expect(copied.value).toBeNull()
  })

  it('reports a failure and clears an earlier confirmation', async () => {
    const writeText = vi.fn(async () => { })
    vi.stubGlobal('navigator', { clipboard: { writeText } })
    const { copied, error, copy, reset } = setupCopy()

    await copy('value', 'a')
    writeText.mockRejectedValueOnce(new DOMException('denied', 'NotAllowedError'))
    expect(await copy('value', 'b')).toBe(false)
    expect(copied.value).toBeNull()
    expect(error.value).toBe('Copy failed')
    reset()
    expect(error.value).toBe('')
  })

  it('writes through the Electron bridge instead of the browser clipboard', async () => {
    const bridgeWrite = vi.fn(async () => { })
    const browserWrite = vi.fn(async () => { })
    vi.stubGlobal('window', { sandpost: { runtime: 'electron', clipboard: { writeText: bridgeWrite } } })
    vi.stubGlobal('navigator', { clipboard: { writeText: browserWrite } })
    const { copied, copy } = setupCopy()

    expect(await copy('From: a@example.test', 'a')).toBe(true)
    expect(bridgeWrite).toHaveBeenCalledWith('From: a@example.test')
    expect(browserWrite).not.toHaveBeenCalled()
    expect(copied.value).toBe('a')
  })

  it('cancels the pending confirmation timer when its scope is disposed', async () => {
    vi.useFakeTimers()
    vi.stubGlobal('navigator', {
      clipboard: {
        /**
         * Accept a clipboard write without accessing the operating system in this test.
         */
        writeText: async () => { }
      }
    })
    const { copy, scope } = setupCopy()

    await copy('value', 'a')
    scope.stop()
    expect(vi.getTimerCount()).toBe(0)
  })
})
