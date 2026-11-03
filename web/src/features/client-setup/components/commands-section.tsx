import { useTranslation } from 'react-i18next'

import { assignCommand, configCommand, deployCommand } from '../commands'
import { CopyField } from './copy-field'

/** `--config`, `--deploy` and `--assign` examples; the API token is a placeholder. */
export function CommandsSection({ configString }: { configString: string | null }) {
  const { t } = useTranslation()
  return (
    <section aria-labelledby="client-commands" className="space-y-4">
      <div className="space-y-1">
        <h2 id="client-commands" className="text-base font-semibold">
          {t('pages.clientSetup.commands.title')}
        </h2>
        <p className="text-sm text-muted-foreground">
          {t('pages.clientSetup.commands.description')}
        </p>
      </div>
      {configString === null ? null : (
        <CopyField
          label={t('pages.clientSetup.commands.config')}
          value={configCommand(configString)}
          multiline
        />
      )}
      <CopyField
        label={t('pages.clientSetup.commands.deploy')}
        value={deployCommand()}
        hint={t('pages.clientSetup.commands.deployHint')}
        multiline
      />
      <CopyField
        label={t('pages.clientSetup.commands.assign')}
        value={assignCommand()}
        hint={t('pages.clientSetup.commands.assignHint')}
        multiline
      />
    </section>
  )
}
