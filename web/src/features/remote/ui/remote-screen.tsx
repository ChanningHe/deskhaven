import { LoaderCircle } from 'lucide-react'
import { useId, useLayoutEffect, useRef } from 'react'
import type { CSSProperties } from 'react'
import { useTranslation } from 'react-i18next'

import { cn } from '@/lib/utils'

import { cursorScale } from '../cursor/css-cursor'
import type { Session } from '../session/session'
import type { RemoteSettings } from '../settings/settings'
import { fitSize } from './draw'
import { FrozenScreen } from './frozen-screen'
import { useStore } from './use-remote'
import { useCursor, useElementSize, useScreenInput, useVideo } from './use-screen'
import { pasteKey, useScreenKeys } from './use-screen-keys'

const noPaste = () => undefined

/**
 * The peer's screen, scaled to fit with its aspect ratio kept, or one peer pixel to a CSS pixel
 * with scroll bars when that is larger: the wheel goes to the peer. The video canvas, and over it
 * the cursor canvas, which takes the pointer. Keys go to an invisible text field on the screen,
 * so that IMEs work there; `onLeave` takes focus elsewhere when the user asks to leave the screen.
 */
export function RemoteScreen({
  session,
  scale,
  showRemoteCursor,
  viewOnly = false,
  onLeave,
  onPointerEnter,
  onGesture,
  onPaste,
  frozen,
  onFrozen,
}: {
  session: Session
  scale: RemoteSettings['scale']
  showRemoteCursor: boolean
  viewOnly?: boolean
  onLeave: () => void
  onPointerEnter?: () => void
  onGesture?: () => void
  onPaste?: (text: string) => void
  /** The last session's frame, shown until this one draws its first. */
  frozen?: HTMLCanvasElement | null | undefined
  onFrozen?: (frame: HTMLCanvasElement | null) => void
}) {
  const { t } = useTranslation()
  const hintId = useId()
  const box = useRef<HTMLDivElement>(null)
  const video = useRef<HTMLCanvasElement>(null)
  const overlay = useRef<HTMLCanvasElement>(null)
  const keys = useRef<HTMLTextAreaElement>(null)
  const peer = useStore(session.peer)
  const frame = useVideo(session, video, onFrozen)
  const still = frame ? null : frozen
  const boxSize = useElementSize(box)
  const display = peer?.display ?? null
  const cursorWidth = display?.width ?? frame?.width ?? 1
  const cursorHeight = display?.height ?? frame?.height ?? 1
  const size = !frame ? null : scale === 'original' ? frame : boxSize && fitSize(frame, boxSize)
  const { cursor, moveTo } = useCursor(session, overlay, {
    showRemote: showRemoteCursor,
    viewOnly,
    scale: size ? cursorScale(size.width, cursorWidth) : null,
  })
  useScreenInput(session, overlay, keys, moveTo)
  const hint = useScreenKeys(session, keys, onLeave, onPaste ?? noPaste)

  useLayoutEffect(() => {
    const target = box.current
    if (scale !== 'fit' || !target) return
    target.scrollTop = 0
    target.scrollLeft = 0
  }, [scale])

  const sizeStyle = {
    '--screen-w': `${size?.width ?? 0}px`,
    '--screen-h': `${size?.height ?? 0}px`,
  } as CSSProperties

  return (
    <div
      ref={box}
      className={cn(
        'relative flex min-h-0 flex-1',
        scale === 'original' ? 'overflow-auto' : 'overflow-hidden',
      )}
    >
      {still ? <FrozenScreen canvas={still} scale={scale} hint={t('remote.resuming')} /> : null}
      {size || still ? null : (
        <p
          role="status"
          className="absolute inset-0 m-auto flex size-fit items-center gap-2 text-sm text-muted-foreground"
        >
          <LoaderCircle aria-hidden className="size-4 animate-spin" />
          {t('remote.connecting')}
        </p>
      )}
      <div
        className={cn(
          'relative m-auto h-(--screen-h) w-(--screen-w) shrink-0',
          size && 'bg-black shadow-sm ring-1 ring-foreground/10',
          'has-[textarea:focus]:ring-2 has-[textarea:focus]:ring-ring',
        )}
        style={sizeStyle}
        onPointerEnter={onPointerEnter}
        onPointerDownCapture={onGesture}
        onKeyDownCapture={(e) => {
          if (!pasteKey(e.nativeEvent)) onGesture?.()
        }}
      >
        <canvas ref={video} aria-hidden className="absolute inset-0 size-full" />
        <canvas
          ref={overlay}
          width={cursorWidth}
          height={cursorHeight}
          aria-hidden
          data-testid="remote-screen"
          className="absolute inset-0 size-full cursor-(--remote-cursor) touch-none"
          style={{ '--remote-cursor': cursor } as CSSProperties}
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
      </div>
    </div>
  )
}
