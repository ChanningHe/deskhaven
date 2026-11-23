import { Clipboard, ClipboardCopy, ClipboardPaste } from 'lucide-react'
import { useId } from 'react'
import { useTranslation } from 'react-i18next'

import {
  DropdownMenuCheckboxItem,
  DropdownMenuItem,
  DropdownMenuSeparator,
} from '@/components/ui/dropdown-menu'

import type { Session } from '../../session/session'
import { ToolbarMenu } from '../toolbar-controls'
import type { ClipboardSyncControl } from '../use-clipboard-sync'
import { useStore } from '../use-remote'

export type ClipboardSyncSwitch = Pick<
  ClipboardSyncControl,
  'access' | 'setEnabled' | 'requestAccess'
> & { enabled: boolean }

export function ClipboardMenu({
  session,
  sync,
  onSend,
  onShowRemote,
}: {
  session: Session
  sync: ClipboardSyncSwitch
  onSend: () => void
  onShowRemote: () => void
}) {
  const { t } = useTranslation()
  const deniedId = useId()
  const { clipboard } = useStore(session.permissions)
  const remoteText = useStore(session.remoteClipboard)
  const denied = sync.enabled && sync.access === 'denied'

  return (
    <ToolbarMenu
      label={t('remote.toolbar.clipboardMenu')}
      hint={t('remote.toolbar.clipboardMenuHint')}
      icon={<Clipboard aria-hidden />}
    >
      <DropdownMenuItem disabled={!clipboard} onClick={onSend}>
        <ClipboardPaste aria-hidden />
        {t('remote.toolbar.sendClipboard')}
      </DropdownMenuItem>
      <DropdownMenuItem disabled={!clipboard || remoteText === null} onClick={onShowRemote}>
        <ClipboardCopy aria-hidden />
        {t('remote.toolbar.remoteClipboard')}
      </DropdownMenuItem>
      <DropdownMenuSeparator />
      <DropdownMenuCheckboxItem
        checked={sync.enabled}
        disabled={!clipboard}
        onCheckedChange={(on: boolean) => {
          sync.setEnabled(on)
        }}
        aria-describedby={denied ? deniedId : undefined}
        closeOnClick
      >
        {t('remote.toolbar.clipboardSync')}
      </DropdownMenuCheckboxItem>
      {sync.enabled && sync.access === 'prompt' ? (
        <DropdownMenuItem disabled={!clipboard} onClick={sync.requestAccess}>
          {t('remote.toolbar.clipboardAllow')}
        </DropdownMenuItem>
      ) : null}
      {denied ? (
        <p id={deniedId} className="px-2 py-1.5 text-xs text-muted-foreground">
          {t('remote.toolbar.clipboardDenied')}
        </p>
      ) : null}
    </ToolbarMenu>
  )
}
