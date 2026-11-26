import { describe, expect, it } from 'vitest'

import { limitHints } from './limit-hints'

describe('limitHints', () => {
  it('sends shortcuts and Esc in full screen where the keyboard can be locked', () => {
    expect(limitHints({ keyboardLock: true, clipboard: 'granted' })).toEqual([
      'remote.limits.shortcutsFullscreen',
      'remote.limits.escape',
      'remote.limits.clipboardBoth',
    ])
  })

  it('points to the send keys menu, and says nothing of Esc, without Keyboard Lock', () => {
    expect(limitHints({ keyboardLock: false, clipboard: 'granted' })).toEqual([
      'remote.limits.shortcutsMenu',
      'remote.limits.clipboardBoth',
    ])
  })

  it.each([
    ['granted', 'remote.limits.clipboardBoth'],
    ['prompt', 'remote.limits.clipboardAsk'],
    [null, 'remote.limits.clipboardAsk'],
    ['denied', 'remote.limits.clipboardDenied'],
  ] as const)('reads clipboard access %s as %s', (clipboard, line) => {
    expect(limitHints({ keyboardLock: false, clipboard }).at(-1)).toBe(line)
  })

  it('says nothing of the clipboard where the browser cannot report its access', () => {
    expect(limitHints({ keyboardLock: false, clipboard: 'unsupported' })).toEqual([
      'remote.limits.shortcutsMenu',
    ])
  })
})
