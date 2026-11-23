import { describe, expect, it } from 'vitest'

import type { AudioState } from './pipeline'
import { soundSwitch, Speaker, type SoundSwitch, type SpeakerView } from './speaker'
import { fakeAudioDeps } from './testing'

describe('Speaker', () => {
  it('knows nothing until probed, and then whether Opus decodes', async () => {
    const yes = new Speaker(fakeAudioDeps().deps)
    expect(yes.view.get().supported).toBeNull()
    await yes.probe()
    expect(yes.view.get().supported).toBe(true)

    const no = new Speaker(fakeAudioDeps({ opus: false }).deps)
    await no.probe()
    expect(no.view.get().supported).toBe(false)
  })

  it('is unsupported without audio deps or when the probe throws', async () => {
    const none = new Speaker(undefined)
    await none.probe()
    expect(none.view.get().supported).toBe(false)

    const broken = new Speaker(fakeAudioDeps({ opus: 'throws' }).deps)
    await broken.probe()
    expect(broken.view.get().supported).toBe(false)
  })

  it('opens nothing before the probe says yes', async () => {
    const f = fakeAudioDeps({ opus: false })
    const speaker = new Speaker(f.deps)
    speaker.start()
    speaker.prepare()
    await speaker.probe()
    speaker.start()
    speaker.prepare()
    expect(f.contexts).toHaveLength(0)
  })

  it('prepares a context that stays suspended without a gesture', async () => {
    const f = fakeAudioDeps()
    const speaker = new Speaker(f.deps)
    await speaker.probe()
    speaker.prepare()
    speaker.prepare()
    expect(f.contexts).toHaveLength(1)
    expect(speaker.view.get()).toMatchObject({ running: false })
    expect(speaker.view.get().output?.running).toBe(false)
  })

  it('resumes the context on start, the click', async () => {
    const f = fakeAudioDeps()
    const speaker = new Speaker(f.deps)
    await speaker.probe()
    speaker.prepare()
    speaker.start()
    await Promise.resolve()
    expect(f.contexts).toHaveLength(1)
    expect(speaker.view.get().running).toBe(true)
    expect(speaker.view.get().output?.running).toBe(true)
  })

  it('opens and resumes in one start when nothing was prepared', async () => {
    const f = fakeAudioDeps()
    const speaker = new Speaker(f.deps)
    await speaker.probe()
    speaker.start()
    await Promise.resolve()
    expect(f.contexts[0]?.resumed).toBe(1)
    expect(speaker.view.get().running).toBe(true)
  })

  it('closes the context on stop and opens a new one on the next start', async () => {
    const f = fakeAudioDeps()
    const speaker = new Speaker(f.deps)
    await speaker.probe()
    speaker.start()
    const first = speaker.view.get().output
    speaker.stop()
    expect(f.contexts[0]?.state).toBe('closed')
    expect(speaker.view.get()).toMatchObject({ output: null, running: false })
    speaker.start()
    expect(f.contexts).toHaveLength(2)
    expect(speaker.view.get().output).not.toBe(first)
  })

  it('follows the browser suspending a running context', async () => {
    const f = fakeAudioDeps()
    const speaker = new Speaker(f.deps)
    await speaker.probe()
    speaker.start()
    await Promise.resolve()
    f.contexts[0]?.browserSets('suspended')
    expect(speaker.view.get().running).toBe(false)
  })

  it('stays off when the browser will not make a context', async () => {
    const f = fakeAudioDeps({ context: 'throws' })
    const speaker = new Speaker(f.deps)
    await speaker.probe()
    expect(() => {
      speaker.start()
    }).not.toThrow()
    expect(speaker.view.get().output).toBeNull()
  })
})

describe('Speaker output', () => {
  it('stops a sound without throwing when the browser refuses to', async () => {
    const f = fakeAudioDeps()
    const speaker = new Speaker(f.deps)
    await speaker.probe()
    speaker.start()
    const output = speaker.view.get().output
    if (!output) throw new Error('no output')
    const sound = output.schedule(output.createBuffer(2, 480, 48000), 0, () => undefined)
    f.contexts[0]?.refuseStop()
    expect(() => {
      sound.stop()
    }).not.toThrow()
  })
})

describe('soundSwitch', () => {
  const view = (patch: Partial<SpeakerView> = {}): SpeakerView => ({
    supported: true,
    output: null,
    running: true,
    ...patch,
  })
  const on = { enabled: true, permitted: true, audio: 'ok' as AudioState }

  it.each<[string, SpeakerView, typeof on, SoundSwitch]>([
    ['probing', view({ supported: null }), on, 'probing'],
    ['no Opus', view({ supported: false }), on, 'unsupported'],
    [
      'no Opus, peer denies',
      view({ supported: false }),
      { ...on, permitted: false },
      'unsupported',
    ],
    ['peer denies', view(), { ...on, permitted: false }, 'denied'],
    ['peer denies while off', view(), { ...on, enabled: false, permitted: false }, 'denied'],
    ['off', view({ running: false }), { ...on, enabled: false }, 'off'],
    ['on, no gesture yet', view({ running: false }), on, 'suspended'],
    ['on and playing', view(), on, 'on'],
    ['decoding failed', view(), { ...on, audio: 'failed' }, 'failed'],
    ['the told format will not decode', view(), { ...on, audio: 'unsupported' }, 'unsupported'],
    [
      'the told format will not decode while off',
      view(),
      { ...on, enabled: false, audio: 'unsupported' },
      'unsupported',
    ],
  ])('%s', (_, speaker, session, expected) => {
    expect(soundSwitch(speaker, session)).toBe(expected)
  })
})
