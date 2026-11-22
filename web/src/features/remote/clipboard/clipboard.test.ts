import { create, type MessageInitShape } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import { decodeBase64 } from '../bytes'
import {
  ClipboardFormat,
  ClipboardSchema,
  MessageSchema,
  MultiClipboardsSchema,
  type Clipboard,
  type Message,
} from '../proto/gen/message_pb.js'
import { MAX_CLIPBOARD_BYTES, clipboardMessage, clipboardText } from './clipboard'

// "hello, 剪贴板" compressed by the zstd 1.5.7 CLI.
const COMPRESSED = decodeBase64('KLUv/SQQgQAAaGVsbG8sIOWJqui0tOadv4Z3aNc=')

function clip(fields: MessageInitShape<typeof ClipboardSchema>): Clipboard {
  return create(ClipboardSchema, { format: ClipboardFormat.Text, ...fields })
}

function multi(...clipboards: Clipboard[]): Message {
  return create(MessageSchema, {
    union: { case: 'multiClipboards', value: create(MultiClipboardsSchema, { clipboards }) },
  })
}

describe('clipboardMessage', () => {
  it('sends text as one uncompressed Text clipboard', () => {
    const message = clipboardMessage('héllo')
    expect(message?.union.case).toBe('clipboard')
    const value = message?.union.value as Clipboard
    expect(value.format).toBe(ClipboardFormat.Text)
    expect(value.compress).toBe(false)
    expect(new TextDecoder().decode(value.content)).toBe('héllo')
  })

  it(`refuses text over ${MAX_CLIPBOARD_BYTES} bytes`, () => {
    expect(clipboardMessage('x'.repeat(MAX_CLIPBOARD_BYTES + 1))).toBeNull()
  })

  it('round-trips through clipboardText', () => {
    const message = clipboardMessage('剪贴板 ✓')
    expect(message && clipboardText(message)).toBe('剪贴板 ✓')
  })
})

describe('clipboardText', () => {
  it('reads a plain Clipboard message', () => {
    const message = create(MessageSchema, {
      union: { case: 'clipboard', value: clip({ content: new TextEncoder().encode('abc') }) },
    })
    expect(clipboardText(message)).toBe('abc')
  })

  it('takes the Text entry of MultiClipboards and inflates it', () => {
    const message = multi(
      clip({ format: ClipboardFormat.Html, content: new TextEncoder().encode('<b>x</b>') }),
      clip({ compress: true, content: COMPRESSED }),
    )
    expect(clipboardText(message)).toBe('hello, 剪贴板')
  })

  it.each([
    ['only an image', multi(clip({ format: ClipboardFormat.ImagePng, content: Uint8Array.of(1) }))],
    ['invalid UTF-8', multi(clip({ content: Uint8Array.of(0xff, 0xfe) }))],
    ['corrupt compressed text', multi(clip({ compress: true, content: Uint8Array.of(1, 2) }))],
    ['another message', create(MessageSchema, {})],
  ])('is null for %s', (_, message: Message) => {
    expect(clipboardText(message)).toBeNull()
  })
})
