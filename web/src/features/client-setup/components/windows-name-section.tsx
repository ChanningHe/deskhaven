import { useTranslation } from 'react-i18next'

import type { ClientConfigView } from '@/api/generated/model'

import { CopyField } from './copy-field'

/** Windows file names the client reads its server settings from. */
export function WindowsNameSection({ config }: { config: ClientConfigView }) {
  const { t } = useTranslation()
  const { windows_filename_encoded: encoded, windows_filename_plain: plain } = config
  if (encoded == null) return null
  return (
    <section aria-labelledby="client-windows" className="space-y-4">
      <div className="space-y-1">
        <h2 id="client-windows" className="text-base font-semibold">
          {t('pages.clientSetup.windows.title')}
        </h2>
        <p className="text-sm text-muted-foreground">
          {t('pages.clientSetup.windows.description')}
        </p>
      </div>
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
    </section>
  )
}
