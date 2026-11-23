import { Clipboard, ClipboardCopy, ClipboardPaste } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import { DropdownMenuItem } from '@/components/ui/dropdown-menu'

import type { Session } from '../../session/session'
import { ToolbarMenu } from '../toolbar-controls'
import { useStore } from '../use-remote'

export function ClipboardMenu({
  session,
  onSend,
  onShowRemote,
}: {
  session: Session
  onSend: () => void
  onShowRemote: () => void
}) {
  const { t } = useTranslation()
  const { clipboard } = useStore(session.permissions)
  const remoteText = useStore(session.remoteClipboard)

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
    </ToolbarMenu>
  )
}
