import { Keyboard } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import {
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
} from '@/components/ui/dropdown-menu'

import { supportedKeyboardModes } from '../../input/keyboard'
import { KeyboardMode } from '../../proto/gen/message_pb.js'
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

export function InputMenu({ session }: { session: Session }) {
  const { t } = useTranslation()
  const peer = useStore(session.peer)
  const { keyboard } = useStore(session.permissions)
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
      <DropdownMenuItem
        disabled={!keyboard}
        onClick={() => {
          session.ctrlAltDel()
        }}
      >
        {t('remote.toolbar.ctrlAltDel')}
      </DropdownMenuItem>
    </ToolbarMenu>
  )
}
