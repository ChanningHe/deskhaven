import { useQueryClient } from '@tanstack/react-query'
import { useId } from 'react'
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
import { SourceBadge } from '@/components/common/source-badge'
import { Button } from '@/components/ui/button'
import { Label } from '@/components/ui/label'
import { valueSource } from '@/features/settings/format'
import { useRebasedDraft } from '@/lib/use-rebased-draft'

import { Section } from './form-controls'

export const TFA_KEY = 'auth.tfa.enforce'
const CHOICES = ['off', 'admins', 'all'] as const
type Choice = (typeof CHOICES)[number]

function isChoice(value: unknown): value is Choice {
  return CHOICES.some((choice) => choice === value)
}

/**
 * The 2FA enforcement policy: the `auth.tfa.enforce` setting, through the settings API
 * (owner only, versioned, read-only while the server configuration pins it).
 */
export function TfaSection({ isOwner }: { isOwner: boolean }) {
  const { t } = useTranslation()
  return (
    <Section
      id="identity-tfa"
      title={t('identity.tfa.title')}
      description={t('identity.tfa.description')}
    >
      {isOwner ? (
        <TfaPolicy />
      ) : (
        <p className="text-sm text-muted-foreground">{t('identity.tfa.ownerOnly')}</p>
      )}
      <p className="text-xs text-muted-foreground">{t('identity.tfa.oidcNote')}</p>
    </Section>
  )
}

function TfaPolicy() {
  const query = useListSettings()
  const setting = query.data?.find((s) => s.key === TFA_KEY)
  if (query.error) return <ErrorAlert error={query.error} onRetry={() => void query.refetch()} />
  if (setting === undefined) return null
  return <PolicyForm setting={setting} />
}

function PolicyForm({ setting }: { setting: SettingView }) {
  const { t } = useTranslation()
  const id = useId()
  const queryClient = useQueryClient()
  const current: Choice = isChoice(setting.value) ? setting.value : 'off'
  // Follows the server copy until the user picks a value (another owner may change it).
  const { draft, changed, conflicts, set, reset } = useRebasedDraft({ value: current })
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
    reset({ value: isChoice(after) ? after : choice }, sent)
  }

  return (
    <div className="grid gap-2">
      <div className="flex flex-wrap items-center gap-2">
        <Label htmlFor={id}>{t('identity.tfa.label')}</Label>
        <SourceBadge source={valueSource(setting.source)} />
      </div>
      <div className="flex flex-wrap items-center gap-2">
        <NativeSelect
          id={id}
          className="h-9"
          value={choice}
          disabled={setting.pinned || mutation.isPending}
          onChange={(event) => {
            const value = event.target.value
            if (isChoice(value)) set({ value })
          }}
        >
          {CHOICES.map((value) => (
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
