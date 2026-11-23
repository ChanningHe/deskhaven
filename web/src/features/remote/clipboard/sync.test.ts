import { describe, expect, it } from 'vitest'

import { AUTO_LIMIT_BYTES, ClipboardSync, syncAllowed } from './sync'

type Step =
  | { baseline: string | null }
  | { local: string; sends: string | null; started?: true }
  | { pasted: string; sends: string | null }
  | { remote: string; writes: string | null }
  | { wrote: string }
  | { take: string | null }
  | 'drop'
  | 'readStarts'
  | 'writeStarts'

function run(steps: Step[]) {
  const sync = new ClipboardSync()
  let read = sync.readStarts()
  for (const step of steps) {
    if (step === 'drop') sync.dropPending()
    else if (step === 'readStarts') read = sync.readStarts()
    else if (step === 'writeStarts') sync.writeStarts()
    else if ('baseline' in step) sync.baseline(step.baseline)
    else if ('local' in step) {
      const stamp = step.started ? read : sync.readStarts()
      expect(sync.local(step.local, stamp), step.local).toBe(step.sends)
    } else if ('pasted' in step) expect(sync.pasted(step.pasted), step.pasted).toBe(step.sends)
    else if ('remote' in step) expect(sync.remote(step.remote), step.remote).toBe(step.writes)
    else if ('wrote' in step) sync.wrote(step.wrote)
    else expect(sync.takePending()).toBe(step.take)
  }
}

const OVER_LIMIT = 'é'.repeat(AUTO_LIMIT_BYTES / 2 + 1)

describe('ClipboardSync, local to remote', () => {
  it.each<[string, Step[]]>([
    ['sends a change after the baseline', [{ baseline: 'B' }, { local: 'X', sends: 'X' }]],
    ['never sends the baseline', [{ baseline: 'B' }, { local: 'B', sends: null }]],
    [
      'takes the first read as the baseline when the activation read failed',
      [{ baseline: null }, { local: 'B', sends: null }, { local: 'X', sends: 'X' }],
    ],
    [
      'takes the first read as the baseline with no baseline call',
      [
        { local: 'B', sends: null },
        { local: 'X', sends: 'X' },
      ],
    ],
    [
      'sends a text once while it stays the same',
      [{ baseline: 'B' }, { local: 'X', sends: 'X' }, { local: 'X', sends: null }],
    ],
    [
      'sends the baseline text again once the user copies it after something else',
      [{ baseline: 'B' }, { local: 'X', sends: 'X' }, { local: 'B', sends: 'B' }],
    ],
    ['compares with CRLF as LF', [{ baseline: 'a\r\nb' }, { local: 'a\nb', sends: null }]],
    ['never sends an empty text', [{ baseline: 'B' }, { local: '', sends: null }]],
    [
      `never sends more than ${AUTO_LIMIT_BYTES} UTF-8 bytes`,
      [{ baseline: 'B' }, { local: OVER_LIMIT, sends: null }],
    ],
    [
      `sends exactly ${AUTO_LIMIT_BYTES} UTF-8 bytes`,
      [
        { baseline: 'B' },
        { local: 'é'.repeat(AUTO_LIMIT_BYTES / 2), sends: 'é'.repeat(AUTO_LIMIT_BYTES / 2) },
      ],
    ],
    [
      'does not send back what the peer copied',
      [{ baseline: 'B' }, { remote: 'R', writes: 'R' }, { local: 'R', sends: null }],
    ],
    [
      'does not send back what the peer copied, with other line ends',
      [
        { baseline: 'B' },
        { remote: 'r1\nr2', writes: 'r1\nr2' },
        { local: 'r1\r\nr2', sends: null },
      ],
    ],
    [
      'does not loop: A sent, the peer echoes A',
      [
        { baseline: 'B' },
        { local: 'A', sends: 'A' },
        { remote: 'A', writes: null },
        { local: 'A', sends: null },
      ],
    ],
    [
      'keeps what the peer copied when the local text has not changed since',
      [
        { baseline: 'B' },
        { local: 'X', sends: 'X' },
        { remote: 'R', writes: 'R' },
        'drop',
        { local: 'X', sends: null },
      ],
    ],
    [
      'a new baseline replaces the old one',
      [{ baseline: 'B' }, { baseline: 'C' }, { local: 'C', sends: null }],
    ],
  ])('%s', (_, steps) => {
    run(steps)
  })
})

describe('ClipboardSync, remote to local', () => {
  it.each<[string, Step[]]>([
    [
      'holds what the peer copied for writing',
      [{ baseline: 'B' }, { remote: 'R', writes: 'R' }, { take: 'R' }, { take: null }],
    ],
    [
      'has nothing to write when the local text is the same already',
      [{ baseline: 'a\r\nb' }, { remote: 'a\nb', writes: null }, { take: null }],
    ],
    [
      `has nothing to write over ${AUTO_LIMIT_BYTES} UTF-8 bytes`,
      [{ baseline: 'B' }, { remote: OVER_LIMIT, writes: null }, { take: null }],
    ],
    [
      'drops the pending write',
      [{ baseline: 'B' }, { remote: 'R', writes: 'R' }, 'drop', { take: null }],
    ],
    [
      'a newer peer text replaces the pending one',
      [
        { baseline: 'B' },
        { remote: 'R1', writes: 'R1' },
        { remote: 'R2', writes: 'R2' },
        { take: 'R2' },
      ],
    ],
    [
      'a local change drops the pending write',
      [{ baseline: 'B' }, { remote: 'R', writes: 'R' }, { local: 'Y', sends: 'Y' }, { take: null }],
    ],
    [
      'a written text is the local text and no longer pending',
      [
        { baseline: 'B' },
        { remote: 'R', writes: 'R' },
        { wrote: 'R' },
        { take: null },
        { local: 'R', sends: null },
        { local: 'B', sends: 'B' },
      ],
    ],
    [
      'discards a read that started before a write',
      [
        { baseline: 'B' },
        'readStarts',
        { remote: 'R', writes: 'R' },
        'writeStarts',
        { wrote: 'R' },
        { local: 'B', sends: null, started: true },
        { local: 'R', sends: null },
        { local: 'B', sends: 'B' },
      ],
    ],
    [
      'discards a read that started before a write that failed',
      [
        { baseline: 'B' },
        'readStarts',
        { remote: 'R', writes: 'R' },
        'writeStarts',
        { local: 'B', sends: null, started: true },
        { take: 'R' },
      ],
    ],
    [
      'an unchanged local read keeps the pending write',
      [{ baseline: 'B' }, { remote: 'R', writes: 'R' }, { local: 'B', sends: null }, { take: 'R' }],
    ],
  ])('%s', (_, steps) => {
    run(steps)
  })
})

describe('ClipboardSync, pasted', () => {
  it.each<[string, Step[]]>([
    [
      'sends the text although it is the baseline',
      [{ baseline: 'B' }, { pasted: 'B', sends: 'B' }],
    ],
    ['sends with no baseline taken', [{ pasted: 'X', sends: 'X' }]],
    [
      'sends a text once while the peer has it',
      [
        { pasted: 'X', sends: 'X' },
        { pasted: 'X', sends: null },
      ],
    ],
    [
      'sends a text again once the peer has another',
      [
        { pasted: 'X', sends: 'X' },
        { remote: 'R', writes: 'R' },
        { pasted: 'X', sends: 'X' },
      ],
    ],
    [
      'does not send what the automatic path sent',
      [{ baseline: 'B' }, { local: 'X', sends: 'X' }, { pasted: 'X', sends: null }],
    ],
    [
      'is not sent again by the automatic path',
      [{ baseline: 'B' }, { pasted: 'X', sends: 'X' }, { local: 'X', sends: null }],
    ],
    [
      'compares with CRLF as LF',
      [
        { remote: 'a\nb', writes: 'a\nb' },
        { pasted: 'a\r\nb', sends: null },
      ],
    ],
    ['never sends an empty text', [{ pasted: '', sends: null }]],
    [
      `never sends more than ${AUTO_LIMIT_BYTES} UTF-8 bytes`,
      [{ pasted: OVER_LIMIT, sends: null }],
    ],
    [
      'drops the pending write',
      [
        { baseline: 'B' },
        { remote: 'R', writes: 'R' },
        { pasted: 'X', sends: 'X' },
        { take: null },
      ],
    ],
  ])('%s', (_, steps) => {
    run(steps)
  })
})

describe('syncAllowed', () => {
  const ALL = { active: true, clipboard: true, clipboardSync: true, viewOnly: false }

  it('allows the automatic path only when every condition holds', () => {
    expect(syncAllowed(ALL)).toBe(true)
    expect(syncAllowed({ ...ALL, active: false })).toBe(false)
    expect(syncAllowed({ ...ALL, clipboard: false })).toBe(false)
    expect(syncAllowed({ ...ALL, clipboardSync: false })).toBe(false)
    expect(syncAllowed({ ...ALL, viewOnly: true })).toBe(false)
  })
})
