import type { QrCode as QrCodeData } from '@/api/generated/model'

interface QrCodeProps {
  qr: QrCodeData
  label: string
}

/**
 * A QR code from the server's SVG path data: React elements only (no markup
 * injection, no inline style, no `data:` URL, as the CSP requires). Always dark
 * on light, which every scanner reads, whatever the theme.
 */
export function QrCode({ qr, label }: QrCodeProps) {
  return (
    <svg
      role="img"
      aria-label={label}
      viewBox={`0 0 ${qr.size} ${qr.size}`}
      shapeRendering="crispEdges"
      className="size-48 rounded-md border"
    >
      <rect width={qr.size} height={qr.size} className="fill-white" />
      <path d={qr.path} className="fill-black" />
    </svg>
  )
}
