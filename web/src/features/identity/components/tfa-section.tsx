import { useQueryClient } from '@tanstack/react-query'
import { useId } from 'react'
import type { ReactNode } from 'react'
import { useTranslation } from 'react-i18next'

import { isApiError } from '@/api/api-error'
import type { SettingView } from '@/api/generated/model'
import {
  getListSettingsQueryKey,
  useListSettings,
  useUpdateSettings,
} from '@/api/generated/settings/settings'
import { ErrorAlert } from '@/components/common/error-alert'
import { NativeSelect } from '@/components/common/native-select'
import { SectionCard } from '@/components/common/section-card'
import { SourceBadge } from '@/components/common/source-badge'
import { Button } from '@/components/ui/button'
import { Label } from '@/components/ui/label'
import { valueSource } from '@/features/settings/format'
import { useRebasedDraft } from '@/lib/use-rebased-draft'

import { useReportDirty } from '../report-dirty'
import type { DirtyReporter } from '../report-dirty'
import { isTfaChoice, TFA_CHOICES, TFA_KEY } from '../tfa-policy'
import type { TfaChoice } from '../tfa-policy'

interface TfaSectionProps {
  isOwner: boolean
  onDirtyChange: DirtyReporter
}

export function TfaSection({ isOwner, onDirtyChange }: TfaSectionProps) {
  const { t } = useTranslation()
  if (isOwner) return <OwnerTfa onDirtyChange={onDirtyChange} />
  return (
    <TfaCard status={undefined}>
      <p className="text-sm text-muted-foreground">{t('identity.tfa.ownerOnly')}</p>
    </TfaCard>
  )
}

function TfaCard({ status, children }: { status: ReactNode; children: ReactNode }) {
  const { t } = useTranslation()
  return (
    <SectionCard
      title={t('identity.tfa.title')}
      description={t('identity.tfa.description')}
      status={status}
    >
      <div className="grid gap-4">
        {children}
        <p className="text-xs text-muted-foreground">{t('identity.tfa.oidcNote')}</p>
      </div>
    </SectionCard>
  )
}

/** Only the owner reads the settings: an admin's page never requests them. */
function OwnerTfa({ onDirtyChange }: { onDirtyChange: DirtyReporter }) {
  const query = useListSettings()
  const setting = query.error ? undefined : query.data?.find((s) => s.key === TFA_KEY)
  return (
    <TfaCard
      status={
        setting === undefined ? undefined : <SourceBadge source={valueSource(setting.source)} />
      }
    >
      {query.error ? <ErrorAlert error={query.error} onRetry={() => void query.refetch()} /> : null}
      {setting === undefined ? null : (
        <PolicyForm setting={setting} onDirtyChange={onDirtyChange} />
      )}
    </TfaCard>
  )
}

interface PolicyFormProps {
  setting: SettingView
  onDirtyChange: DirtyReporter
}

function PolicyForm({ setting, onDirtyChange }: PolicyFormProps) {
  const { t } = useTranslation()
  const id = useId()
  const queryClient = useQueryClient()
  const current: TfaChoice = isTfaChoice(setting.value) ? setting.value : 'off'
  // Follows the server copy until the user picks a value (another owner may change it).
  const { draft, changed, conflicts, set, reset } = useRebasedDraft({ value: current })
  useReportDirty(changed.length > 0, onDirtyChange)
  const choice = draft.value
  const mutation = useUpdateSettings()
  const stale = isApiError(mutation.error) && mutation.error.code === 'version_conflict'

  const save = async () => {
    const sent = { value: choice }
    const change = { key: TFA_KEY, value: choice, version: setting.version }
    const saved = await mutation.mutateAsync({ data: { changes: [change] } }).catch(() => undefined)
    if (saved === undefined) {
      // A stale version: the refetched copy carries the current one for the next save.
      await queryClient.invalidateQueries({ queryKey: getListSettingsQueryKey() })
      return
    }
    queryClient.setQueryData(getListSettingsQueryKey(), saved)
    const after = saved.find((s) => s.key === TFA_KEY)?.value
    reset({ value: isTfaChoice(after) ? after : choice }, sent)
  }

  return (
    <div className="grid gap-2">
      <Label htmlFor={id}>{t('identity.tfa.label')}</Label>
      <div className="flex flex-wrap items-center gap-2">
        <NativeSelect
          id={id}
          className="h-9"
          value={choice}
          disabled={setting.pinned || mutation.isPending}
          onChange={(event) => {
            const value = event.target.value
            if (isTfaChoice(value)) set({ value })
          }}
        >
          {TFA_CHOICES.map((value) => (
            <option key={value} value={value}>
              {t(`identity.tfa.choices.${value}`)}
            </option>
          ))}
        </NativeSelect>
        {setting.pinned ? null : (
          <Button disabled={changed.length === 0 || mutation.isPending} onClick={() => void save()}>
            {t('identity.tfa.save')}
          </Button>
        )}
      </div>
      {setting.pinned ? <PinnedNote source={setting.source} /> : null}
      {stale || conflicts.length > 0 ? (
        <p role="status" className="text-xs text-warning">
          {t('identity.tfa.conflict')}
        </p>
      ) : mutation.error ? (
        <ErrorAlert error={mutation.error} />
      ) : null}
      {mutation.isSuccess && changed.length === 0 ? (
        <p role="status" className="text-xs text-muted-foreground">
          {t('identity.tfa.saved')}
        </p>
      ) : null}
    </div>
  )
}

function PinnedNote({ source }: { source: SettingView['source'] }) {
  const { t } = useTranslation()
  const text =
    source.kind === 'env'
      ? t('settings.pinned.env', { name: source.var })
      : source.kind === 'file'
        ? t('settings.pinned.file', { path: source.path })
        : t('settings.pinned.other')
  return <p className="text-xs text-warning">{text}</p>
}
