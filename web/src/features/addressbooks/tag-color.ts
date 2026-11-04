// Tag colours are the client's 32-bit ARGB integers (`Color.value`); the console's colour
// input works in `#rrggbb`. The console always writes an opaque colour.

const ALPHA_OPAQUE = 0xff000000
const RGB_MASK = 0x00ffffff
const HEX_PATTERN = /^#[0-9a-fA-F]{6}$/

/** The client's default tag colour (grey), as `#rrggbb`. */
export const DEFAULT_TAG_HEX = '#9e9e9e'

/** `#rrggbb` of an ARGB value (its alpha is dropped). */
export function argbToHex(argb: number): string {
  const rgb = argb & RGB_MASK
  return `#${rgb.toString(16).padStart(6, '0')}`
}

/** The opaque ARGB value of `#rrggbb`; `undefined` for anything else. */
export function hexToArgb(hex: string): number | undefined {
  if (!HEX_PATTERN.test(hex)) return undefined
  // `>>> 0`: keep the 32-bit value unsigned (the bitwise OR yields a signed one).
  return (ALPHA_OPAQUE | Number.parseInt(hex.slice(1), 16)) >>> 0
}
