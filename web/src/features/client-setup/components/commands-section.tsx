import { useTranslation } from 'react-i18next'

import { SectionCard } from '@/components/common/section-card'

import { assignCommand, configCommand, deployCommand } from '../commands'
import { CopyField } from './copy-field'

export function CommandsSection({ configString }: { configString: string | null }) {
  const { t } = useTranslation()
  return (
    <SectionCard
      title={t('pages.clientSetup.commands.title')}
      description={t('pages.clientSetup.commands.description')}
    >
      <div className="space-y-4">
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
      </div>
    </SectionCard>
  )
}
