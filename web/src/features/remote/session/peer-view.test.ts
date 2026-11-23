import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import {
  DisplayInfoSchema,
  KeyboardMode,
  PeerInfoSchema,
  SwitchDisplaySchema,
} from '../proto/gen/message_pb.js'
import { peerView, withDisplays, withSwitch } from './peer-view'

function info(embedded: [boolean, boolean], currentDisplay = 0) {
  return create(PeerInfoSchema, {
    currentDisplay,
    displays: [
      create(DisplayInfoSchema, { width: 1280, height: 800, cursorEmbedded: embedded[0] }),
      create(DisplayInfoSchema, { x: 1280, width: 800, height: 600, cursorEmbedded: embedded[1] }),
    ],
  })
}

describe('PeerView cursor_embedded', () => {
  it('comes from the current display of PeerInfo', () => {
    expect(peerView(info([false, true]), KeyboardMode.Map).cursorEmbedded).toBe(false)
    expect(peerView(info([false, true], 1), KeyboardMode.Map).cursorEmbedded).toBe(true)
  })

  it('follows a later PeerInfo, and is kept when it leaves the current display out', () => {
    const view = peerView(info([false, false]), KeyboardMode.Map)
    expect(withDisplays(view, info([true, false])).cursorEmbedded).toBe(true)
    expect(withDisplays(view, create(PeerInfoSchema)).cursorEmbedded).toBe(false)
  })

  it('comes from SwitchDisplay', () => {
    const view = peerView(info([false, false]), KeyboardMode.Map)
    const on = create(SwitchDisplaySchema, {
      display: 1,
      width: 800,
      height: 600,
      cursorEmbedded: true,
    })
    const switched = withSwitch(view, on)
    expect(switched.cursorEmbedded).toBe(true)
    const off = create(SwitchDisplaySchema, { display: 0, width: 1280, height: 800 })
    expect(withSwitch(switched, off).cursorEmbedded).toBe(false)
  })
})
