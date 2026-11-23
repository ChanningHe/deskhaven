import { describe, expect, it } from 'vitest'

import { msg } from '../__fixtures__/harness'
import { TransportError } from '../transport/transport'
import { vp9Frame } from '../video/fake-decoder'
import { MAX_FAILURES } from '../video/pipeline'
import { startSession } from './session'
import { FakeClock, FakeHandshake, fakeLinkSession, misc, settle } from './testing'

describe('why the session closed', () => {
  it('user: we hung up', async () => {
    const { session } = await fakeLinkSession('relay', new FakeClock())
    session.close()
    expect(session.state.get()).toEqual({ kind: 'closed', cause: { by: 'user', reason: '' } })
  })

  it.each(['Closed manually by the peer', 'closed', 'connection closed', 'timeout'])(
    'peer: Misc.close_reason "%s" stays the peer\'s, whatever it reads like',
    async (reason) => {
      const { session, handshake } = await fakeLinkSession('relay', new FakeClock())
      handshake.peerSends(misc({ case: 'closeReason', value: reason }))
      await settle()
      expect(session.state.get()).toEqual({ kind: 'closed', cause: { by: 'peer', reason } })
    },
  )

  it('peer: the login failed', async () => {
    const handshake = new FakeHandshake('relay')
    const session = startSession(handshake)
    handshake.events.push({ kind: 'state', state: { kind: 'failed', reason: 'Offline' } })
    handshake.events.push({ kind: 'closed', cause: { by: 'peer', reason: 'Offline' } })
    await settle()
    expect(session.state.get()).toEqual({
      kind: 'closed',
      cause: { by: 'peer', reason: 'Offline' },
    })
  })

  it('link: the handshake lost the link', async () => {
    const { session, handshake } = await fakeLinkSession('relay', new FakeClock())
    const cause = { by: 'link', reason: 'relay connection closed (code 1006)' } as const
    handshake.events.push({ kind: 'closed', cause })
    await settle()
    expect(session.state.get()).toEqual({ kind: 'closed', cause })
  })

  it('link: a send fails', async () => {
    const { session, handshake } = await fakeLinkSession('relay', new FakeClock())
    handshake.send = () => {
      throw new TransportError('relay connection is closed')
    }
    session.ctrlAltDel()
    expect(session.state.get()).toEqual({
      kind: 'closed',
      cause: { by: 'link', reason: 'relay connection is closed' },
    })
    expect(handshake.closedWith).toMatchObject({ by: 'link' })
  })

  it('local: a send fails for another reason than the link', async () => {
    const { session, handshake } = await fakeLinkSession('relay', new FakeClock())
    handshake.send = () => {
      throw new Error('the session is not keyed')
    }
    session.ctrlAltDel()
    expect(session.state.get()).toEqual({
      kind: 'closed',
      cause: { by: 'local', reason: 'the session is not keyed' },
    })
  })

  it('local: the video decoder keeps failing', async () => {
    const { session, handshake, decoders } = await fakeLinkSession('relay', new FakeClock())
    for (let i = 0; i < MAX_FAILURES; i++) {
      handshake.peerSends(msg({ case: 'videoFrame', value: vp9Frame(true) }))
      await settle()
      decoders.last.fail()
    }
    const state = session.state.get()
    expect(state).toMatchObject({ kind: 'closed', cause: { by: 'local' } })
    expect(state.kind === 'closed' && state.cause.reason).toContain('video decoding failed')
  })
})
