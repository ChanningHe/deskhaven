import { useTranslation } from 'react-i18next'

import type { ClientConfigView } from '@/api/generated/model'
import { SectionCard } from '@/components/common/section-card'
import { QrCode } from '@/features/auth/components/qr-code'

import { CopyField } from './copy-field'

export function ConfigStringSection({ config }: { config: ClientConfigView }) {
  const { t } = useTranslation()
  const { config_string: configString, qr } = config
  return (
    <SectionCard title={t('pages.clientSetup.string.title')}>
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
    </SectionCard>
  )
}
