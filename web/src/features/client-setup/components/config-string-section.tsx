import { useTranslation } from 'react-i18next'

import type { ClientConfigView } from '@/api/generated/model'
import { QrCode } from '@/features/auth/components/qr-code'

import { CopyField } from './copy-field'

/** The config string for the client's import box, and its QR code for the mobile apps. */
export function ConfigStringSection({ config }: { config: ClientConfigView }) {
  const { t } = useTranslation()
  const { config_string: configString, qr } = config
  return (
    <section aria-labelledby="client-string" className="space-y-4">
      <h2 id="client-string" className="text-base font-semibold">
        {t('pages.clientSetup.string.title')}
      </h2>
      {configString == null ? (
        <p className="text-sm text-muted-foreground">{t('pages.clientSetup.string.unavailable')}</p>
      ) : (
        <div className="flex flex-col gap-6 md:flex-row md:items-start">
          <div className="min-w-0 flex-1">
            <CopyField
              label={t('pages.clientSetup.string.label')}
              value={configString}
              hint={t('pages.clientSetup.string.hint')}
              multiline
            />
          </div>
          {qr == null ? null : (
            <figure className="space-y-2">
              <QrCode qr={qr} label={t('pages.clientSetup.string.qrLabel')} />
              <figcaption className="max-w-48 text-xs text-muted-foreground">
                {t('pages.clientSetup.string.qrHint')}
              </figcaption>
            </figure>
          )}
        </div>
      )}
    </section>
  )
}
