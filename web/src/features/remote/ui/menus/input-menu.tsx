import { Keyboard } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import {
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
} from '@/components/ui/dropdown-menu'

import { supportedKeyboardModes } from '../../input/keyboard'
import { KeyboardMode } from '../../proto/gen/message_pb.js'
import { SESSION_ACTIONS, type SessionAction } from '../../session/messages'
import { PLATFORM_MACOS } from '../../session/peer'
import type { Session } from '../../session/session'
import { ToolbarMenu } from '../toolbar-controls'
import { useStore } from '../use-remote'

const MODE_KEYS = {
  [KeyboardMode.Map]: 'map',
  [KeyboardMode.Translate]: 'translate',
  [KeyboardMode.Legacy]: 'legacy',
} as const satisfies Partial<Record<KeyboardMode, string>>

type ShownMode = keyof typeof MODE_KEYS

function isShownMode(mode: KeyboardMode): mode is ShownMode {
  return mode in MODE_KEYS
}

function keyLabel(kind: SessionAction, platform: string | undefined): SessionAction | 'cmd_tab' {
  return kind === 'alt_tab' && platform === PLATFORM_MACOS ? 'cmd_tab' : kind
}

export function InputMenu({ session, viewOnly }: { session: Session; viewOnly: boolean }) {
  const { t } = useTranslation()
  const peer = useStore(session.peer)
  const keyboard = useStore(session.permissions).keyboard && !viewOnly
  const modes = peer
    ? supportedKeyboardModes(peer.info.platform, peer.info.version).filter(isShownMode)
    : []

  return (
    <ToolbarMenu
      label={t('remote.toolbar.inputMenu')}
      hint={t('remote.toolbar.inputMenuHint')}
      icon={<Keyboard aria-hidden />}
    >
      {modes.length > 0 ? (
        <>
          <DropdownMenuGroup>
            <DropdownMenuLabel>{t('remote.toolbar.keyboardMode')}</DropdownMenuLabel>
            <DropdownMenuRadioGroup
              value={peer?.keyboardMode}
              onValueChange={(value: unknown) => {
                const mode = modes.find((m) => m === value)
                if (mode !== undefined) session.setKeyboardMode(mode)
              }}
            >
              {modes.map((mode) => (
                <DropdownMenuRadioItem
                  key={mode}
                  value={mode}
                  disabled={!keyboard || modes.length < 2}
                  closeOnClick
                >
                  {t(`remote.toolbar.modes.${MODE_KEYS[mode]}`)}
                </DropdownMenuRadioItem>
              ))}
            </DropdownMenuRadioGroup>
          </DropdownMenuGroup>
          <DropdownMenuSeparator />
        </>
      ) : null}
      <DropdownMenuSub>
        <DropdownMenuSubTrigger disabled={!keyboard}>
          {t('remote.toolbar.sendKeys')}
        </DropdownMenuSubTrigger>
        <DropdownMenuSubContent>
          {SESSION_ACTIONS.map((kind) => (
            <DropdownMenuItem
              key={kind}
              disabled={!keyboard}
              onClick={() => {
                session.action(kind)
              }}
            >
              {t(`remote.toolbar.keys.${keyLabel(kind, peer?.info.platform)}`)}
            </DropdownMenuItem>
          ))}
        </DropdownMenuSubContent>
      </DropdownMenuSub>
    </ToolbarMenu>
  )
}
