import { useTranslation } from 'react-i18next'

import type { ClientConfigView } from '@/api/generated/model'

import { CopyField } from './copy-field'

/** ID server, relay, API server and public key, as a client's server dialog takes them. */
export function AddressSection({ config }: { config: ClientConfigView }) {
  const { t } = useTranslation()
  const relayHint =
    config.relay_servers.length === 0
      ? t('pages.clientSetup.addresses.relayAuto')
      : t('pages.clientSetup.addresses.relayList')
  return (
    <section aria-labelledby="client-addresses" className="space-y-4">
      <h2 id="client-addresses" className="text-base font-semibold">
        {t('pages.clientSetup.addresses.title')}
      </h2>
      <div className="grid gap-4 md:grid-cols-2">
        <CopyField label={t('pages.clientSetup.addresses.idServer')} value={config.id_server} />
        <CopyField
          label={t('pages.clientSetup.addresses.relayServer')}
          value={config.relay_servers.join(', ')}
          hint={relayHint}
        />
        <CopyField
          label={t('pages.clientSetup.addresses.apiServer')}
          value={config.api_server}
          hint={config.api_server === '' ? t('pages.clientSetup.addresses.apiUnset') : undefined}
        />
        <CopyField label={t('pages.clientSetup.addresses.publicKey')} value={config.public_key} />
      </div>
    </section>
  )
}
