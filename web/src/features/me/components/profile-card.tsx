import { useQueryClient } from '@tanstack/react-query'
import { useId } from 'react'
import type { SubmitEvent } from 'react'
import { useTranslation } from 'react-i18next'

import { useUpdateMyProfile } from '@/api/generated/me/me'
import type { MyAccount } from '@/api/generated/model'
import { ErrorAlert } from '@/components/common/error-alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { useRebasedDraft } from '@/lib/use-rebased-draft'

import { refreshAccount } from '../invalidate'
import { Section } from './section'

const NAME_MAX_LENGTH = 128

/**
 * The display name is editable. The email is shown read-only: an administrator changes it
 * (an address typed by its holder would be unverified yet reserve the real owner's).
 */
export function ProfileCard({ account }: { account: MyAccount }) {
  const { t } = useTranslation()
  const id = useId()
  const queryClient = useQueryClient()
  const { draft, changed, set, reset } = useRebasedDraft({ display_name: account.display_name })
  const mutation = useUpdateMyProfile()

  const submit = async (event: SubmitEvent<HTMLFormElement>) => {
    event.preventDefault()
    const data = { display_name: draft.display_name }
    const saved = await mutation.mutateAsync({ data }).catch(() => undefined)
    if (saved === undefined) return
    reset({ display_name: saved.display_name }, data)
    await refreshAccount(queryClient, saved)
  }

  return (
    <Section title={t('me.profile.title')} description={t('me.profile.description')}>
      <form className="grid max-w-md gap-2" onSubmit={(event) => void submit(event)}>
        <Label htmlFor={`${id}-name`}>{t('me.profile.displayName')}</Label>
        <Input
          id={`${id}-name`}
          maxLength={NAME_MAX_LENGTH}
          value={draft.display_name}
          placeholder={account.username}
          onChange={(e) => {
            set({ display_name: e.target.value })
          }}
        />
        <div className="mt-2 flex items-center gap-2">
          <Label htmlFor={`${id}-email`}>{t('me.profile.email')}</Label>
          {account.email === null || account.email === undefined ? null : (
            <Badge variant={account.email_verified ? 'secondary' : 'outline'}>
              {account.email_verified ? t('me.profile.verified') : t('me.profile.unverified')}
            </Badge>
          )}
        </div>
        <Input
          id={`${id}-email`}
          type="email"
          readOnly
          value={account.email ?? ''}
          placeholder={t('me.profile.noEmail')}
        />
        <p className="text-xs text-muted-foreground">{t('me.profile.emailAdminHint')}</p>
        {mutation.error ? <ErrorAlert error={mutation.error} /> : null}
        <Button
          type="submit"
          size="sm"
          className="w-fit"
          disabled={changed.length === 0 || mutation.isPending}
        >
          {t('me.profile.save')}
        </Button>
      </form>
    </Section>
  )
}
