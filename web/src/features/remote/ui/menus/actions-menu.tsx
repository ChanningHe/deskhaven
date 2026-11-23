import { Power, Zap } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { ConfirmDialog } from '@/components/common/confirm-dialog'
import { DropdownMenuItem, DropdownMenuSeparator } from '@/components/ui/dropdown-menu'

import type { Session } from '../../session/session'
import type { SessionIntent } from '../../settings/protocol'
import { MenuSwitch, ToolbarMenu } from '../toolbar-controls'
import { useStore } from '../use-remote'

export function ActionsMenu({
  session,
  intent,
  onIntent,
}: {
  session: Session
  intent: SessionIntent
  onIntent: (patch: Partial<SessionIntent>) => void
}) {
  const { t } = useTranslation()
  const { restart } = useStore(session.permissions)
  const [confirming, setConfirming] = useState(false)

  return (
    <>
      <ToolbarMenu
        label={t('remote.toolbar.actionsMenu')}
        hint={t('remote.toolbar.actionsMenuHint')}
        icon={<Zap aria-hidden />}
      >
        <MenuSwitch
          checked={intent.viewOnly}
          onCheckedChange={(viewOnly: boolean) => {
            onIntent({ viewOnly })
          }}
          closeOnClick
        >
          {t('remote.toolbar.viewOnly')}
        </MenuSwitch>
        <MenuSwitch
          checked={intent.lockAfterEnd}
          disabled={intent.viewOnly}
          onCheckedChange={(lockAfterEnd: boolean) => {
            onIntent({ lockAfterEnd })
          }}
          closeOnClick
        >
          {t('remote.toolbar.lockAfterEnd')}
        </MenuSwitch>
        <DropdownMenuSeparator />
        <DropdownMenuItem
          variant="destructive"
          disabled={!restart}
          onClick={() => {
            setConfirming(true)
          }}
        >
          <Power aria-hidden />
          {t('remote.toolbar.restart')}
        </DropdownMenuItem>
      </ToolbarMenu>
      <ConfirmDialog
        open={confirming}
        onOpenChange={setConfirming}
        title={t('remote.restart.title')}
        description={t('remote.restart.description')}
        confirmLabel={t('remote.restart.confirm')}
        onConfirm={() => {
          session.restart()
        }}
      />
    </>
  )
}
