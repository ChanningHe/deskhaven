import { ClipboardCopy, ClipboardPaste, Keyboard, Maximize, Minimize, Monitor } from 'lucide-react'
import { useId } from 'react'
import type { ComponentProps, ReactElement, ReactNode, Ref } from 'react'
import { useTranslation } from 'react-i18next'

import { NativeSelect } from '@/components/common/native-select'
import { Button } from '@/components/ui/button'
import { Separator } from '@/components/ui/separator'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'

import { supportedKeyboardModes } from '../input/keyboard'
import { KeyboardMode } from '../proto/gen/message_pb.js'
import type { Session } from '../session/session'
import { useStore } from './use-remote'

const MODE_KEYS = {
  [KeyboardMode.Map]: 'map',
  [KeyboardMode.Translate]: 'translate',
  [KeyboardMode.Legacy]: 'legacy',
} as const satisfies Partial<Record<KeyboardMode, string>>

type ShownMode = keyof typeof MODE_KEYS

function isShownMode(mode: KeyboardMode): mode is ShownMode {
  return mode in MODE_KEYS
}

// Native selects dressed as ghost toolbar buttons, with room on the left for an icon.
const SELECT_CLASS =
  'h-8 border-transparent bg-transparent pr-1 pl-8 hover:bg-muted dark:bg-transparent dark:hover:bg-muted/50'

/** A control with its hint below it; the hint repeats or extends the control's name. */
function Hinted({ hint, render }: { hint: string; render: ReactElement }) {
  return (
    <Tooltip>
      <TooltipTrigger render={render} />
      <TooltipContent side="bottom">{hint}</TooltipContent>
    </Tooltip>
  )
}

/** An icon-only button: its label is its accessible name, and its tooltip unless a hint is given. */
function IconButton({
  label,
  hint = label,
  ...props
}: { label: string; hint?: string } & ComponentProps<typeof Button>) {
  return (
    <Hinted
      hint={hint}
      render={<Button variant="ghost" size="icon" aria-label={label} {...props} />}
    />
  )
}

/** A labelled native select with a leading icon. */
function IconSelect({
  id,
  label,
  hint,
  icon,
  ...props
}: { id: string; label: string; hint: string; icon: ReactNode } & ComponentProps<'select'>) {
  return (
    <Hinted
      hint={hint}
      render={
        <span className="relative inline-flex items-center">
          <span
            aria-hidden
            className="pointer-events-none absolute left-2.5 text-muted-foreground [&_svg]:size-4"
          >
            {icon}
          </span>
          <label htmlFor={id} className="sr-only">
            {label}
          </label>
          <NativeSelect id={id} className={SELECT_CLASS} {...props} />
        </span>
      }
    />
  )
}

function Divider() {
  return <Separator orientation="vertical" className="mx-1 my-1.5" />
}

interface ToolbarProps {
  ref?: Ref<HTMLDivElement>
  session: Session
  fullscreen: boolean
  onFullscreen: () => void
  onSendClipboard: () => void
  onShowRemoteClipboard: () => void
}

/** Input, display and clipboard controls, in that order, as compact icon groups. */
export function Toolbar({
  ref,
  session,
  fullscreen,
  onFullscreen,
  onSendClipboard,
  onShowRemoteClipboard,
}: ToolbarProps) {
  const { t } = useTranslation()
  const id = useId()
  const peer = useStore(session.peer)
  const { keyboard, clipboard } = useStore(session.permissions)
  const remoteText = useStore(session.remoteClipboard)
  const modes = peer
    ? supportedKeyboardModes(peer.info.platform, peer.info.version).filter(isShownMode)
    : []

  return (
    <div
      ref={ref}
      role="toolbar"
      aria-label={t('remote.toolbar.label')}
      className="flex min-w-0 flex-wrap items-stretch gap-0.5"
    >
      <Hinted
        hint={t('remote.toolbar.ctrlAltDelHint')}
        render={
          <Button
            variant="ghost"
            className="px-2 text-xs"
            disabled={!keyboard}
            onClick={() => {
              session.ctrlAltDel()
            }}
          >
            {t('remote.toolbar.ctrlAltDel')}
          </Button>
        }
      />
      <IconSelect
        id={`${id}-mode`}
        label={t('remote.toolbar.keyboardMode')}
        hint={t('remote.toolbar.keyboardModeHint')}
        icon={<Keyboard />}
        disabled={!keyboard || modes.length < 2}
        value={peer?.keyboardMode ?? ''}
        onChange={(event) => {
          const mode = modes.find((m) => String(m) === event.target.value)
          if (mode !== undefined) session.setKeyboardMode(mode)
        }}
      >
        {modes.map((mode) => (
          <option key={mode} value={mode}>
            {t(`remote.toolbar.modes.${MODE_KEYS[mode]}`)}
          </option>
        ))}
      </IconSelect>
      <Divider />
      {peer && peer.displays.length > 1 ? (
        <IconSelect
          id={`${id}-display`}
          label={t('remote.toolbar.display')}
          hint={t('remote.toolbar.displayHint')}
          icon={<Monitor />}
          value={peer.current}
          onChange={(event) => {
            session.switchDisplay(Number(event.target.value))
          }}
        >
          {peer.displays.map((_, index) => (
            <option key={index} value={index}>
              {index + 1}
            </option>
          ))}
        </IconSelect>
      ) : null}
      <IconButton
        label={t(fullscreen ? 'remote.toolbar.exitFullscreen' : 'remote.toolbar.fullscreen')}
        onClick={onFullscreen}
      >
        {fullscreen ? <Minimize aria-hidden /> : <Maximize aria-hidden />}
      </IconButton>
      <Divider />
      <IconButton
        label={t('remote.toolbar.sendClipboard')}
        hint={t('remote.toolbar.sendClipboardHint')}
        disabled={!clipboard}
        onClick={onSendClipboard}
      >
        <ClipboardPaste aria-hidden />
      </IconButton>
      <IconButton
        label={t('remote.toolbar.remoteClipboard')}
        hint={t('remote.toolbar.remoteClipboardHint')}
        disabled={!clipboard || remoteText === null}
        onClick={onShowRemoteClipboard}
      >
        <ClipboardCopy aria-hidden />
      </IconButton>
    </div>
  )
}
