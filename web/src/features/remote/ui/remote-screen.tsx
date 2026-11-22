import { LoaderCircle } from 'lucide-react'
import { useLayoutEffect, useRef } from 'react'
import type { CSSProperties } from 'react'
import { useTranslation } from 'react-i18next'

import { cn } from '@/lib/utils'

import type { Session } from '../session/session'
import { fitSize } from './draw'
import { useStore } from './use-remote'
import { useCursor, useElementSize, useScreenInput, useVideo } from './use-screen'

/**
 * The peer's screen, scaled to fit with its aspect ratio kept: the video canvas, and over it
 * the cursor canvas, which takes focus and input. The local pointer is hidden over it; the
 * peer's cursor is drawn instead.
 */
export function RemoteScreen({ session }: { session: Session }) {
  const { t } = useTranslation()
  const box = useRef<HTMLDivElement>(null)
  const video = useRef<HTMLCanvasElement>(null)
  const overlay = useRef<HTMLCanvasElement>(null)
  const peer = useStore(session.peer)
  const frame = useVideo(session, video)
  const boxSize = useElementSize(box)
  const { paint, moveTo } = useCursor(session, overlay)
  useScreenInput(session, overlay, moveTo)

  const display = peer?.display ?? null
  const cursorWidth = display?.width ?? frame?.width ?? 1
  const cursorHeight = display?.height ?? frame?.height ?? 1
  // A canvas clears whenever its size is set, so the cursor is painted again after.
  useLayoutEffect(paint, [paint, cursorWidth, cursorHeight])

  const fit = frame && boxSize ? fitSize(frame, boxSize) : null
  const fitStyle = {
    '--screen-w': `${fit?.width ?? 0}px`,
    '--screen-h': `${fit?.height ?? 0}px`,
  } as CSSProperties

  return (
    <div
      ref={box}
      className="relative flex min-h-0 flex-1 items-center justify-center overflow-hidden"
    >
      {fit ? null : (
        <p role="status" className="absolute flex items-center gap-2 text-sm text-muted-foreground">
          <LoaderCircle aria-hidden className="size-4 animate-spin" />
          {t('remote.connecting')}
        </p>
      )}
      <div
        className={cn(
          'relative h-(--screen-h) w-(--screen-w)',
          fit && 'bg-black shadow-sm ring-1 ring-foreground/10',
        )}
        style={fitStyle}
      >
        <canvas ref={video} aria-hidden className="absolute inset-0 size-full" />
        <canvas
          ref={overlay}
          width={cursorWidth}
          height={cursorHeight}
          tabIndex={0}
          role="application"
          aria-label={t('remote.screen')}
          data-testid="remote-screen"
          className="absolute inset-0 size-full cursor-none touch-none outline-none focus-visible:ring-2 focus-visible:ring-ring"
        />
      </div>
    </div>
  )
}
