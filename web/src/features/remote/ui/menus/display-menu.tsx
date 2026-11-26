import { Monitor } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import {
  DropdownMenuGroup,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
} from '@/components/ui/dropdown-menu'

import type { Session } from '../../session/session'
import { TRANSPORTS, type RemoteSettings } from '../../settings/settings'
import { MenuSwitch, ToolbarMenu } from '../toolbar-controls'
import { useStore } from '../use-remote'
import { CodecSubmenu } from './codec-submenu'
import { QualityGroup } from './quality-group'
import { ResolutionSubmenu } from './resolution-submenu'

const FPS_STEPS = [5, 10, 15, 24, 30, 45, 60]
const SCALES: readonly RemoteSettings['scale'][] = ['fit', 'original']

type OnSettings = (patch: Partial<RemoteSettings>) => void

function FpsSubmenu({ fps, onSettings }: { fps: number; onSettings: OnSettings }) {
  const { t } = useTranslation()
  const steps = FPS_STEPS.includes(fps) ? FPS_STEPS : [...FPS_STEPS, fps].sort((a, b) => a - b)
  return (
    <DropdownMenuSub>
      <DropdownMenuSubTrigger>
        <span className="flex flex-1 justify-between gap-3">
          {t('remote.toolbar.fps')}
          <span className="text-muted-foreground tabular-nums">
            {t('remote.toolbar.fpsValue', { fps })}
          </span>
        </span>
      </DropdownMenuSubTrigger>
      <DropdownMenuSubContent>
        <DropdownMenuRadioGroup
          value={fps}
          onValueChange={(value: unknown) => {
            const picked = steps.find((step) => step === value)
            if (picked !== undefined) onSettings({ fps: picked })
          }}
        >
          {steps.map((step) => (
            <DropdownMenuRadioItem key={step} value={step} closeOnClick>
              {t('remote.toolbar.fpsValue', { fps: step })}
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>
      </DropdownMenuSubContent>
    </DropdownMenuSub>
  )
}

function SessionQualityGroup({
  session,
  quality,
  onSettings,
}: {
  session: Session
  quality: RemoteSettings['quality']
  onSettings: OnSettings
}) {
  const display = useStore(session.peer)?.display ?? null
  const { codec } = useStore(session.stats)
  return (
    <QualityGroup
      quality={quality}
      size={display}
      codec={codec}
      onChange={(next) => {
        onSettings({ quality: next })
      }}
    />
  )
}

function MonitorGroup({ session }: { session: Session }) {
  const { t } = useTranslation()
  const peer = useStore(session.peer)
  if (!peer || peer.displays.length < 2) return null
  return (
    <>
      <DropdownMenuSeparator />
      <DropdownMenuGroup>
        <DropdownMenuLabel>{t('remote.toolbar.monitor')}</DropdownMenuLabel>
        <DropdownMenuRadioGroup
          value={peer.current}
          onValueChange={(value: unknown) => {
            if (typeof value === 'number') session.switchDisplay(value)
          }}
        >
          {peer.displays.map((_, index) => (
            <DropdownMenuRadioItem key={index} value={index} closeOnClick>
              {t('remote.toolbar.monitorN', { n: index + 1 })}
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>
      </DropdownMenuGroup>
    </>
  )
}

/** What the user asked for, not what the governors use right now: the stats overlay shows that. */
export function DisplayMenu({
  session,
  viewOnly,
  settings,
  onSettings,
  stats,
  onStats,
}: {
  session: Session
  viewOnly: boolean
  settings: RemoteSettings
  onSettings: OnSettings
  stats: boolean
  onStats: (on: boolean) => void
}) {
  const { t } = useTranslation()
  return (
    <ToolbarMenu
      label={t('remote.toolbar.displayMenu')}
      hint={t('remote.toolbar.displayMenuHint')}
      icon={<Monitor aria-hidden />}
    >
      <SessionQualityGroup session={session} quality={settings.quality} onSettings={onSettings} />
      <DropdownMenuSeparator />
      <FpsSubmenu fps={settings.fps} onSettings={onSettings} />
      <CodecSubmenu
        session={session}
        codec={settings.codec}
        onChange={(codec) => {
          onSettings({ codec })
        }}
      />
      <DropdownMenuSeparator />
      <DropdownMenuGroup>
        <DropdownMenuLabel>{t('remote.toolbar.scale')}</DropdownMenuLabel>
        <DropdownMenuRadioGroup
          value={settings.scale}
          onValueChange={(value: unknown) => {
            const scale = SCALES.find((s) => s === value)
            if (scale !== undefined) onSettings({ scale })
          }}
        >
          {SCALES.map((scale) => (
            <DropdownMenuRadioItem key={scale} value={scale} closeOnClick>
              {t(`remote.toolbar.scales.${scale}`)}
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>
      </DropdownMenuGroup>
      <MonitorGroup session={session} />
      <ResolutionSubmenu session={session} viewOnly={viewOnly} />
      <DropdownMenuSeparator />
      <DropdownMenuGroup>
        <DropdownMenuLabel>{t('remote.toolbar.transport')}</DropdownMenuLabel>
        <DropdownMenuRadioGroup
          value={settings.transport}
          onValueChange={(value: unknown) => {
            const transport = TRANSPORTS.find((option) => option === value)
            if (transport !== undefined) onSettings({ transport })
          }}
        >
          {TRANSPORTS.map((transport) => (
            <DropdownMenuRadioItem key={transport} value={transport} closeOnClick>
              {t(`remote.toolbar.transports.${transport}`)}
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>
      </DropdownMenuGroup>
      <DropdownMenuSeparator />
      <MenuSwitch
        checked={settings.showRemoteCursor}
        onCheckedChange={(showRemoteCursor: boolean) => {
          onSettings({ showRemoteCursor })
        }}
        closeOnClick
      >
        {t('remote.toolbar.showRemoteCursor')}
      </MenuSwitch>
      <MenuSwitch checked={stats} onCheckedChange={onStats} closeOnClick>
        {t('remote.toolbar.stats')}
      </MenuSwitch>
    </ToolbarMenu>
  )
}
