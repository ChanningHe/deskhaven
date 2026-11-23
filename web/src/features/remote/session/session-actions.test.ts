import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import {
  ControlKey,
  KeyboardMode,
  PeerInfoSchema,
  PermissionInfo_Permission,
  PermissionInfoSchema,
  WindowsSessionSchema,
  WindowsSessionsSchema,
} from '../proto/gen/message_pb.js'
import type { SessionAction } from './messages'
import { PEER_INFO, activeSession, misc, settle } from './testing'

const PLATFORMS = ['Windows', 'Linux', 'Mac OS', 'Android'] as const

const META = { union: { case: 'controlKey', value: ControlKey.Meta } }

const SAME_ON_EVERY_PLATFORM: Readonly<
  Record<Exclude<SessionAction, 'ctrl_alt_del' | 'alt_tab'>, object[]>
> = {
  lock_screen: [{ down: true, union: { case: 'controlKey', value: ControlKey.LockScreen } }],
  // A Linux peer reads the press of a modifier as a release, so the Win key goes down and up.
  win: [
    { ...META, down: true, press: false },
    { ...META, down: false, press: false },
  ],
  print_screen: [{ press: true, union: { case: 'controlKey', value: ControlKey.Snapshot } }],
}

function peer(platform: string) {
  return create(PeerInfoSchema, { ...PEER_INFO, platform })
}

describe('session.action', () => {
  describe.each(PLATFORMS)('on a %s peer', (platform) => {
    it.each(Object.entries(SAME_ON_EVERY_PLATFORM))(
      'sends %s as legacy key events',
      async (kind, events) => {
        const r = await activeSession(peer(platform))
        r.session.action(kind as SessionAction)
        const sent = r.sentAfterLogin().map((m) => m.union)
        expect(sent).toHaveLength(events.length)
        expect(sent).toMatchObject(
          events.map((event) => ({
            case: 'keyEvent',
            value: { mode: KeyboardMode.Legacy, ...event },
          })),
        )
      },
    )
  })

  it.each([
    ['Windows', ControlKey.Alt],
    ['Linux', ControlKey.Alt],
    ['Mac OS', ControlKey.Meta],
  ])('sends Tab with the app-switch modifier of a %s peer', async (platform, modifier) => {
    const r = await activeSession(peer(platform))
    r.session.action('alt_tab')
    expect(r.sentAfterLogin().map((m) => m.union.value)).toMatchObject([
      {
        mode: KeyboardMode.Legacy,
        press: true,
        union: { case: 'controlKey', value: ControlKey.Tab },
        modifiers: [modifier],
      },
    ])
  })

  it('sends the CtrlAltDel key to a Windows peer', async () => {
    const r = await activeSession(peer('Windows'))
    r.session.action('ctrl_alt_del')
    expect(r.sentAfterLogin().map((m) => m.union.value)).toMatchObject([
      {
        mode: KeyboardMode.Legacy,
        down: true,
        union: { case: 'controlKey', value: ControlKey.CtrlAltDel },
      },
    ])
  })

  it('sends Delete with Ctrl and Alt to a peer other than Windows', async () => {
    const r = await activeSession(peer('Linux'))
    r.session.action('ctrl_alt_del')
    expect(r.sentAfterLogin().map((m) => m.union.value)).toMatchObject([
      {
        mode: KeyboardMode.Legacy,
        press: true,
        union: { case: 'controlKey', value: ControlKey.Delete },
        modifiers: [ControlKey.Alt, ControlKey.Control],
      },
    ])
  })

  it('sends nothing without keyboard permission', async () => {
    const r = await activeSession()
    r.peerSends(
      misc({
        case: 'permissionInfo',
        value: create(PermissionInfoSchema, {
          permission: PermissionInfo_Permission.Keyboard,
          enabled: false,
        }),
      }),
    )
    await settle()
    for (const kind of ['ctrl_alt_del', 'lock_screen', 'win', 'alt_tab', 'print_screen'] as const) {
      r.session.action(kind)
    }
    expect(r.sentAfterLogin()).toEqual([])
  })

  it('sends nothing while the session is not active', async () => {
    const info = peer('Windows')
    info.windowsSessions = create(WindowsSessionsSchema, {
      sessions: [
        create(WindowsSessionSchema, { sid: 1, name: 'Console' }),
        create(WindowsSessionSchema, { sid: 2, name: 'RDP' }),
      ],
      currentSid: 2,
    })
    const r = await activeSession(info, 'choose_session')
    r.session.action('lock_screen')
    r.session.action('alt_tab')
    expect(r.sentAfterLogin()).toEqual([])
  })
})
