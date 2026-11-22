import { Link } from '@tanstack/react-router'
import { MonitorPlay } from 'lucide-react'
import { useId } from 'react'
import { useTranslation } from 'react-i18next'

import type { DeviceView } from '@/api/generated/model'
import { Button, buttonVariants } from '@/components/ui/button'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { useMe } from '@/features/auth/session'

// This file is the remote feature's only import outside its lazy route: it must not pull in
// the protocol code.

/**
 * Opens the browser remote page for a device, when the server offers it. An offline or
 * disabled device shows the action greyed out, with the reason.
 */
export function RemoteConnectButton({
  device,
}: {
  device: Pick<DeviceView, 'rustdesk_id' | 'online' | 'status'>
}) {
  const { t } = useTranslation()
  const hintId = useId()
  const me = useMe()
  if (!me.web_client_enabled) return null
  const label = (
    <>
      <MonitorPlay aria-hidden />
      {t('remote.connect')}
    </>
  )
  if (device.online && device.status === 'active') {
    return (
      <Link
        to="/remote/$deviceId"
        params={{ deviceId: device.rustdesk_id }}
        className={buttonVariants({ variant: 'outline', size: 'sm' })}
      >
        {label}
      </Link>
    )
  }
  const hint = t(device.status === 'active' ? 'remote.offline' : 'remote.disabled')
  return (
    <Tooltip>
      <TooltipTrigger
        render={<span tabIndex={0} aria-describedby={hintId} className="inline-flex rounded-lg" />}
      >
        <Button variant="outline" size="sm" disabled>
          {label}
        </Button>
        <span id={hintId} hidden>
          {hint}
        </span>
      </TooltipTrigger>
      <TooltipContent>{hint}</TooltipContent>
    </Tooltip>
  )
}
