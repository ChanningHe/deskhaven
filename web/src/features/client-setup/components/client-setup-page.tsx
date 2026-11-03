import { TriangleAlert } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import { useGetClientConfig } from '@/api/generated/client-config/client-config'
import type { ClientConfigView } from '@/api/generated/model'
import { ErrorAlert } from '@/components/common/error-alert'
import { PageHeader } from '@/components/common/page-header'

import { AddressSection } from './address-section'
import { CommandsSection } from './commands-section'
import { ConfigStringSection } from './config-string-section'
import { WindowsNameSection } from './windows-name-section'

/** Client configuration (08 §3.2, 05 §5): what to give RustDesk clients to use this server. */
export function ClientSetupPage() {
  const { t } = useTranslation()
  const query = useGetClientConfig()
  return (
    <div className="space-y-6">
      <PageHeader title={t('nav.clientSetup')} description={t('pages.clientSetup.description')} />
      {query.data === undefined ? (
        query.error ? (
          <ErrorAlert
            error={query.error}
            title={t('pages.clientSetup.loadFailed')}
            onRetry={() => void query.refetch()}
          />
        ) : (
          <p className="text-sm text-muted-foreground">{t('common.loading')}</p>
        )
      ) : (
        <ClientSetup config={query.data} />
      )}
    </div>
  )
}

function ClientSetup({ config }: { config: ClientConfigView }) {
  return (
    <div className="space-y-8">
      <HostWarning source={config.host_source} />
      <AddressSection config={config} />
      <ConfigStringSection config={config} />
      <WindowsNameSection config={config} />
      <CommandsSection configString={config.config_string ?? null} />
    </div>
  )
}

/** Why the ID server address may be wrong or missing: `public_url` is not set. */
function HostWarning({ source }: { source: ClientConfigView['host_source'] }) {
  const { t } = useTranslation()
  if (source === 'public_url') return null
  return (
    <p
      role="note"
      className="flex items-start gap-2 rounded-lg border border-warning/30 bg-warning/5 px-3 py-2 text-sm text-warning"
    >
      <TriangleAlert aria-hidden className="mt-0.5 size-4 shrink-0" />
      {source === 'request_host'
        ? t('pages.clientSetup.warning.requestHost')
        : t('pages.clientSetup.warning.unknown')}
    </p>
  )
}
