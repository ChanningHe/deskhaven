import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import {
  CursorDataSchema,
  PeerInfoSchema,
  WindowsSessionSchema,
  WindowsSessionsSchema,
} from '../proto/gen/message_pb.js'
import { msg } from '../__fixtures__/harness'
import { PEER_INFO, activeSession, settle } from './testing'
import { MAX_WINDOWS_SESSIONS } from './windows-sessions'

const CONSOLE = create(WindowsSessionSchema, { sid: 1, name: 'Console' })
const RDP = create(WindowsSessionSchema, { sid: 2, name: 'RDP: ChanningOnPVE (running)' })

function withSessions(sessions = [CONSOLE, RDP], currentSid = 2) {
  return create(PeerInfoSchema, {
    ...PEER_INFO,
    platform: 'Windows',
    windowsSessions: create(WindowsSessionsSchema, { sessions, currentSid }),
  })
}

const selected = (r: Awaited<ReturnType<typeof activeSession>>) =>
  r
    .sentAfterLogin()
    .flatMap((m) =>
      m.union.case === 'misc' && m.union.value.union.case === 'selectedSid'
        ? [m.union.value.union.value]
        : [],
    )

describe('session on a peer that shares Windows sessions', () => {
  it('asks which session before it goes active, the current one first in line', async () => {
    const r = await activeSession(withSessions(), 'choose_session')
    expect(r.session.state.get()).toEqual({
      kind: 'choose_session',
      sessions: [
        { sid: 1, name: 'Console' },
        { sid: 2, name: 'RDP: ChanningOnPVE (running)' },
      ],
      current: 2,
    })
    expect(selected(r)).toEqual([])
  })

  it('sends the chosen session and goes active on the current one', async () => {
    const r = await activeSession(withSessions(), 'choose_session')
    r.session.chooseWindowsSession(2)
    await settle()
    expect(selected(r)).toEqual([2])
    expect(r.session.state.get()).toEqual({ kind: 'active' })
  })

  it('ignores a session the peer did not offer', async () => {
    const r = await activeSession(withSessions(), 'choose_session')
    r.session.chooseWindowsSession(7)
    await settle()
    expect(selected(r)).toEqual([])
    expect(r.session.state.get().kind).toBe('choose_session')
  })

  it('waits for the peer to switch after another session was chosen, and names the switch', async () => {
    const r = await activeSession(withSessions(), 'choose_session')
    r.session.chooseWindowsSession(1)
    await settle()
    expect(selected(r)).toEqual([1])
    expect(r.session.state.get()).toEqual({ kind: 'connecting' })
    r.socket.drop(1006)
    await settle()
    expect(r.session.state.get()).toEqual({
      kind: 'closed',
      cause: { by: 'session_switch', reason: 'relay connection closed (code 1006)' },
    })
  })

  it('goes active and forgets the switch when the peer serves this connection after all', async () => {
    const r = await activeSession(withSessions(), 'choose_session')
    r.session.chooseWindowsSession(1)
    await settle()
    r.peerSends(msg({ case: 'cursorData', value: create(CursorDataSchema, { id: '1' }) }))
    await settle()
    expect(r.session.state.get()).toEqual({ kind: 'active' })
    r.socket.drop(1006)
    await settle()
    const state = r.session.state.get()
    expect(state).toMatchObject({ kind: 'closed', cause: { by: 'link' } })
  })

  it('keeps our own close as ours after another session was chosen', async () => {
    const r = await activeSession(withSessions(), 'choose_session')
    r.session.chooseWindowsSession(1)
    r.session.close()
    await settle()
    expect(r.session.state.get()).toEqual({ kind: 'closed', cause: { by: 'user', reason: '' } })
  })

  it('takes at most MAX_WINDOWS_SESSIONS of an untrusted list', async () => {
    const many = Array.from({ length: MAX_WINDOWS_SESSIONS + 5 }, (_, i) =>
      create(WindowsSessionSchema, { sid: i + 1, name: `s${String(i + 1)}` }),
    )
    const r = await activeSession(withSessions(many, 1), 'choose_session')
    const state = r.session.state.get()
    expect(state.kind === 'choose_session' && state.sessions.length).toBe(MAX_WINDOWS_SESSIONS)
  })

  it('offers each session once however often the peer lists it', async () => {
    const r = await activeSession(withSessions([CONSOLE, RDP, CONSOLE]), 'choose_session')
    const state = r.session.state.get()
    expect(state.kind === 'choose_session' && state.sessions.map((s) => s.sid)).toEqual([1, 2])
  })

  it('goes straight to active without a session list', async () => {
    const r = await activeSession()
    expect(r.session.state.get()).toEqual({ kind: 'active' })
    expect(selected(r)).toEqual([])
  })
})
