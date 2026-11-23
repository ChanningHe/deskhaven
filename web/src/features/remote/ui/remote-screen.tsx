import { LoaderCircle } from 'lucide-react'
import { useId, useLayoutEffect, useRef } from 'react'
import type { CSSProperties, ReactNode } from 'react'
import { useTranslation } from 'react-i18next'

import { cn } from '@/lib/utils'

import type { Session } from '../session/session'
import { fitSize } from './draw'
import { useStore } from './use-remote'
import { useCursor, useElementSize, useScreenInput, useVideo } from './use-screen'
import { useScreenKeys } from './use-screen-keys'

/**
 * The peer's screen, scaled to fit with its aspect ratio kept: the video canvas, and over it
 * the cursor canvas, which takes the pointer. The local pointer is hidden over it; the peer's
 * cursor is drawn instead. Keys go to an invisible text field on the screen, so that IMEs work
 * there; `onLeave` takes focus elsewhere when the user asks to leave the screen.
 */
export function RemoteScreen({
  session,
  onLeave,
  children,
}: {
  session: Session
  onLeave: () => void
  children?: ReactNode
}) {
  const { t } = useTranslation()
  const hintId = useId()
  const box = useRef<HTMLDivElement>(null)
  const video = useRef<HTMLCanvasElement>(null)
  const overlay = useRef<HTMLCanvasElement>(null)
  const keys = useRef<HTMLTextAreaElement>(null)
  const peer = useStore(session.peer)
  const frame = useVideo(session, video)
  const boxSize = useElementSize(box)
  const { paint, moveTo } = useCursor(session, overlay)
  useScreenInput(session, overlay, keys, moveTo)
  const hint = useScreenKeys(session, keys, onLeave)

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
          'has-[textarea:focus]:ring-2 has-[textarea:focus]:ring-ring',
        )}
        style={fitStyle}
      >
        <canvas ref={video} aria-hidden className="absolute inset-0 size-full" />
        <canvas
          ref={overlay}
          width={cursorWidth}
          height={cursorHeight}
          aria-hidden
          data-testid="remote-screen"
          className="absolute inset-0 size-full cursor-none touch-none"
        />
        <textarea
          ref={keys}
          aria-label={t('remote.screen')}
          aria-describedby={hintId}
          autoComplete="off"
          autoCapitalize="off"
          spellCheck={false}
          className="pointer-events-none absolute top-0 left-0 size-px resize-none overflow-hidden border-0 p-0 opacity-0 outline-none"
        />
        <p
          id={hintId}
          className={cn(
            'pointer-events-none absolute top-3 left-1/2 -translate-x-1/2 rounded-full bg-foreground px-3 py-1.5 text-xs whitespace-nowrap text-background shadow-md transition-opacity duration-300',
            hint ? 'opacity-100' : 'opacity-0',
          )}
        >
          {t('remote.leaveHint')}
        </p>
        {children}
      </div>
    </div>
  )
}
