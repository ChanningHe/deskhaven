import { decompress } from 'fzstd'

const ZSTD_MAGIC = 0xfd2fb528
const MAGIC_BYTES = 4
const DICT_ID_BYTES = [0, 1, 2, 4] as const
const FCS_TWO_BYTE_OFFSET = 256

/**
 * The decompressed size a zstd frame declares in its header (RFC 8878 §3.1.1.1), or null when
 * it declares none or `data` is not a whole frame header.
 */
export function frameContentSize(data: Uint8Array): number | null {
  if (data.length < MAGIC_BYTES + 1) return null
  const view = new DataView(data.buffer, data.byteOffset, data.byteLength)
  if (view.getUint32(0, true) !== ZSTD_MAGIC) return null
  const descriptor = data[MAGIC_BYTES] ?? 0
  const fcsFlag = descriptor >> 6
  const singleSegment = (descriptor >> 5) & 1
  const fcsBytes = fcsFlag === 0 ? singleSegment : 1 << fcsFlag
  if (fcsBytes === 0) return null
  const offset = MAGIC_BYTES + 1 + (singleSegment ? 0 : 1) + (DICT_ID_BYTES[descriptor & 3] ?? 0)
  if (data.length < offset + fcsBytes) return null
  switch (fcsBytes) {
    case 1:
      return view.getUint8(offset)
    case 2:
      return view.getUint16(offset, true) + FCS_TWO_BYTE_OFFSET
    case 4:
      return view.getUint32(offset, true)
    default:
      return Number(view.getBigUint64(offset, true))
  }
}

/**
 * Decompresses one zstd frame of at most `maxBytes`, as the peer's compressor writes it (with
 * its content size). Null for anything else: no declared size, a size over the limit, a second
 * frame or corrupt data.
 *
 * The output buffer is allocated from the checked header size and handed to the decoder, which
 * then never allocates by what the stream claims; a stream that overruns it fails.
 */
export function inflate(data: Uint8Array, maxBytes: number): Uint8Array | null {
  const size = frameContentSize(data)
  if (size === null || size > maxBytes) return null
  const out = new Uint8Array(size)
  try {
    return decompress(data, out) === out ? out : null
  } catch {
    return null
  }
}

const ZSTD_BLOCKSIZE_MAX = 128 << 10

/** zstd.h ZSTD_COMPRESSBOUND: the most a compressor can emit for `size` input bytes. */
export function compressBound(size: number): number {
  const small = size < ZSTD_BLOCKSIZE_MAX ? (ZSTD_BLOCKSIZE_MAX - size) >> 11 : 0
  return size + (size >> 8) + small
}
