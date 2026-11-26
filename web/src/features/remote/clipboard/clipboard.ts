import { create } from '@bufbuild/protobuf'

import { utf8 } from '../bytes'
import { inflate } from '../cursor/zstd'
import {
  ClipboardFormat,
  ClipboardSchema,
  MessageSchema,
  type Clipboard,
  type Message,
} from '../proto/gen/message_pb.js'

/** Text clipboard limit, both ways; far above what people copy, far below the 64 MiB frame. */
export const MAX_CLIPBOARD_BYTES = 16 << 20

/** Null over the limit. A plain `Clipboard` message: every peer version takes it. */
export function clipboardMessage(text: string): Message | null {
  const content = utf8(text)
  if (content.length > MAX_CLIPBOARD_BYTES) return null
  return create(MessageSchema, {
    union: {
      case: 'clipboard',
      value: create(ClipboardSchema, { compress: false, content, format: ClipboardFormat.Text }),
    },
  })
}

/**
 * From `Clipboard` or the Text entry of `MultiClipboards`, as the peer reads them; null for
 * other formats and for content that is not valid (compressed) UTF-8.
 */
export function clipboardText(message: Message): string | null {
  const { union } = message
  let entry: Clipboard | undefined
  if (union.case === 'clipboard') entry = union.value
  if (union.case === 'multiClipboards') {
    entry = union.value.clipboards.find((c) => c.format === ClipboardFormat.Text)
  }
  if (entry?.format !== ClipboardFormat.Text) return null
  const content = entry.compress ? inflate(entry.content, MAX_CLIPBOARD_BYTES) : entry.content
  if (!content || content.length > MAX_CLIPBOARD_BYTES) return null
  try {
    return new TextDecoder('utf-8', { fatal: true }).decode(content)
  } catch {
    return null
  }
}
