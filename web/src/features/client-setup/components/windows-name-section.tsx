import { useTranslation } from 'react-i18next'

import type { ClientConfigView } from '@/api/generated/model'
import { SectionCard } from '@/components/common/section-card'

import { CopyField } from './copy-field'

/** Windows file names the client reads its server settings from. */
export function WindowsNameSection({ config }: { config: ClientConfigView }) {
  const { t } = useTranslation()
  const { windows_filename_encoded: encoded, windows_filename_plain: plain } = config
  if (encoded == null) return null
  return (
    <SectionCard
      title={t('pages.clientSetup.windows.title')}
      description={t('pages.clientSetup.windows.description')}
    >
      <div className="space-y-4">
        <CopyField
          label={t('pages.clientSetup.windows.encoded')}
          value={encoded}
          hint={t('pages.clientSetup.windows.encodedHint')}
          multiline
        />
        {plain == null ? (
          <p className="text-sm text-muted-foreground">
            {t('pages.clientSetup.windows.plainUnavailable')}
          </p>
        ) : (
          <CopyField
            label={t('pages.clientSetup.windows.plain')}
            value={plain}
            hint={t('pages.clientSetup.windows.plainHint')}
            multiline
          />
        )}
      </div>
    </SectionCard>
  )
}
