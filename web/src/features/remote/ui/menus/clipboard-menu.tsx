import { CircleAlert, Clipboard, ClipboardCopy, ClipboardPaste, KeyRound } from 'lucide-react'
import { useId } from 'react'
import { useTranslation } from 'react-i18next'

import { DropdownMenuItem, DropdownMenuSeparator } from '@/components/ui/dropdown-menu'

import type { Session } from '../../session/session'
import { MenuSwitch, ToolbarMenu } from '../toolbar-controls'
import type { ClipboardSyncControl } from '../use-clipboard-sync'
import { useStore } from '../use-remote'

export type ClipboardSyncSwitch = Pick<
  ClipboardSyncControl,
  'access' | 'setEnabled' | 'requestAccess'
> & { enabled: boolean }

export function ClipboardMenu({
  session,
  viewOnly,
  sync,
  onSend,
  onShowRemote,
}: {
  session: Session
  viewOnly: boolean
  sync: ClipboardSyncSwitch
  onSend: () => void
  onShowRemote: () => void
}) {
  const { t } = useTranslation()
  const deniedId = useId()
  const { clipboard } = useStore(session.permissions)
  const sending = clipboard && !viewOnly
  const remoteText = useStore(session.remoteClipboard)
  const denied = sync.enabled && sync.access === 'denied'

  return (
    <ToolbarMenu
      label={t('remote.toolbar.clipboardMenu')}
      hint={t('remote.toolbar.clipboardMenuHint')}
      icon={<Clipboard aria-hidden />}
    >
      <DropdownMenuItem disabled={!sending} onClick={onSend}>
        <ClipboardPaste aria-hidden />
        {t('remote.toolbar.sendClipboard')}
      </DropdownMenuItem>
      <DropdownMenuItem disabled={!clipboard || remoteText === null} onClick={onShowRemote}>
        <ClipboardCopy aria-hidden />
        {t('remote.toolbar.remoteClipboard')}
      </DropdownMenuItem>
      <DropdownMenuSeparator />
      <MenuSwitch
        checked={sync.enabled}
        disabled={!sending}
        onCheckedChange={(on: boolean) => {
          sync.setEnabled(on)
        }}
        aria-describedby={denied ? deniedId : undefined}
        closeOnClick
      >
        {t('remote.toolbar.clipboardSync')}
      </MenuSwitch>
      {sync.enabled && sync.access === 'prompt' ? (
        <DropdownMenuItem
          disabled={!sending}
          onClick={sync.requestAccess}
          className="mt-0.5 bg-warning/10 py-1.5"
        >
          <KeyRound aria-hidden className="text-warning" />
          {t('remote.toolbar.clipboardAllow')}
        </DropdownMenuItem>
      ) : null}
      {denied ? (
        <p
          id={deniedId}
          className="mt-0.5 flex gap-1.5 rounded-md bg-warning/10 px-1.5 py-1.5 text-xs leading-relaxed text-muted-foreground"
        >
          <CircleAlert aria-hidden className="mt-0.5 size-3.5 shrink-0 text-warning" />
          <span>{t('remote.toolbar.clipboardDenied')}</span>
        </p>
      ) : null}
    </ToolbarMenu>
  )
}
