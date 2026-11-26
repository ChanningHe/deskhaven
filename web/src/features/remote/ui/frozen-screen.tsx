import { LoaderCircle } from 'lucide-react'
import { useLayoutEffect, useRef } from 'react'
import type { CSSProperties } from 'react'

import { cn } from '@/lib/utils'

import type { RemoteSettings } from '../settings/settings'

export function FrozenScreen({
  canvas,
  scale,
  hint,
}: {
  canvas: HTMLCanvasElement
  scale: RemoteSettings['scale']
  hint: string
}) {
  const holder = useRef<HTMLDivElement>(null)
  useLayoutEffect(() => {
    const target = holder.current
    if (!target) return
    target.replaceChildren(canvas)
    return () => {
      target.replaceChildren()
    }
  }, [canvas])

  const sizeStyle = {
    '--frozen-w': `${canvas.width}px`,
    '--frozen-h': `${canvas.height}px`,
    '--frozen-ratio': `${canvas.width / Math.max(1, canvas.height)}`,
  } as CSSProperties

  return (
    <div className="absolute inset-0">
      <div inert className="[container-type:size] absolute inset-0 flex overflow-hidden">
        <div
          className={cn(
            'relative m-auto shrink-0 bg-black shadow-sm ring-1 ring-foreground/10',
            scale === 'original'
              ? 'h-(--frozen-h) w-(--frozen-w)'
              : 'aspect-(--frozen-ratio) w-[min(100cqw,calc(100cqh*var(--frozen-ratio)))]',
          )}
          style={sizeStyle}
        >
          <div ref={holder} className="absolute inset-0 *:size-full" />
          <div className="absolute inset-0 bg-black/45" />
        </div>
      </div>
      <p
        role="status"
        className="pointer-events-none absolute top-4 left-1/2 flex -translate-x-1/2 items-center gap-2 rounded-full bg-foreground px-3 py-1.5 text-xs whitespace-nowrap text-background shadow-md"
      >
        <LoaderCircle aria-hidden className="size-3.5 animate-spin" />
        {hint}
      </p>
    </div>
  )
}
