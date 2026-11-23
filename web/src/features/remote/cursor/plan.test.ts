import { describe, expect, it } from 'vitest'

import { localCursor, paintedCursor, type CursorScene } from './plan'

const CSS = 'url("blob:x") 1 2, auto'
const NORMAL: CursorScene = {
  embedded: false,
  control: true,
  showRemote: true,
  hasShape: true,
  css: CSS,
}

describe('cursor plan', () => {
  it.each<
    [
      string,
      Partial<CursorScene>,
      string,
      ReturnType<typeof paintedCursor>,
      ReturnType<typeof paintedCursor>,
    ]
  >([
    ['in control with an image', {}, CSS, null, 'remote'],
    ['in control, remote cursor off', { showRemote: false }, CSS, null, null],
    ['without input rights', { control: false }, 'default', 'remote', 'remote'],
    [
      'without input rights, remote cursor off',
      { control: false, showRemote: false },
      'default',
      'remote',
      'remote',
    ],
    ['cursor_embedded', { embedded: true }, 'default', null, null],
    [
      'cursor_embedded without input rights',
      { embedded: true, control: false },
      'default',
      null,
      null,
    ],
    [
      'in control, no image (too large, failed, being made)',
      { css: null },
      'none',
      'local',
      'remote',
    ],
    [
      'in control, no image, remote cursor off',
      { css: null, showRemote: false },
      'none',
      'local',
      'local',
    ],
    ['in control, no shape yet', { css: null, hasShape: false }, 'default', null, null],
  ])('%s', (_, patch, local, led, moved) => {
    const scene = { ...NORMAL, ...patch }
    expect(localCursor(scene)).toBe(local)
    expect(paintedCursor(scene, false)).toBe(led)
    expect(paintedCursor(scene, true)).toBe(moved)
  })
})
