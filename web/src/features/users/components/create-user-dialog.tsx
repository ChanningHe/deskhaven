import { useQueryClient } from '@tanstack/react-query'
import { UserPlus } from 'lucide-react'
import { useId, useState } from 'react'
import type { SubmitEvent } from 'react'
import { useTranslation } from 'react-i18next'

import { useCreateUser } from '@/api/generated/users/users'
import type { NewUserBody, RoleView } from '@/api/generated/model'
import { ErrorAlert } from '@/components/common/error-alert'
import { NativeSelect } from '@/components/common/native-select'
import { SecretRevealDialog } from '@/components/common/secret-reveal-dialog'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { useMe } from '@/features/auth/session'

import { invalidateAccounts } from '../invalidate'
import { assignableRoles } from '../user-rules'

interface Draft {
  username: string
  display_name: string
  email: string
  role: RoleView
  setPassword: boolean
  password: string
}

const EMPTY: Draft = {
  username: '',
  display_name: '',
  email: '',
  role: 'user',
  setPassword: false,
  password: '',
}

function toBody(draft: Draft): NewUserBody {
  return {
    username: draft.username.trim(),
    display_name: draft.display_name,
    role: draft.role,
    ...(draft.email.trim() === '' ? {} : { email: draft.email.trim() }),
    ...(draft.setPassword ? { password: draft.password } : {}),
  }
}

/** "New user": a local account; the generated temporary password is shown once. */
export function CreateUserButton() {
  const { t } = useTranslation()
  const me = useMe()
  const queryClient = useQueryClient()
  const id = useId()
  const [open, setOpen] = useState(false)
  const [draft, setDraft] = useState<Draft>(EMPTY)
  const mutation = useCreateUser()
  const created = mutation.data
  const set = (patch: Partial<Draft>) => {
    setDraft((previous) => ({ ...previous, ...patch }))
  }

  const submit = async (event: SubmitEvent<HTMLFormElement>) => {
    event.preventDefault()
    const result = await mutation.mutateAsync({ data: toBody(draft) }).catch(() => undefined)
    if (result === undefined) return
    setOpen(false)
    setDraft(EMPTY)
    void invalidateAccounts(queryClient)
    if (result.temporary_password === undefined) mutation.reset()
  }

  return (
    <>
      <Button
        onClick={() => {
          mutation.reset()
          setOpen(true)
        }}
      >
        <UserPlus aria-hidden />
        {t('users.create.action')}
      </Button>
      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>{t('users.create.title')}</DialogTitle>
            <DialogDescription>{t('users.create.description')}</DialogDescription>
          </DialogHeader>
          <form className="grid gap-3" onSubmit={(event) => void submit(event)}>
            <Label htmlFor={`${id}-username`}>{t('users.fields.username')}</Label>
            <Input
              id={`${id}-username`}
              required
              autoComplete="off"
              value={draft.username}
              onChange={(e) => {
                set({ username: e.target.value })
              }}
            />
            <Label htmlFor={`${id}-display`}>{t('users.fields.displayName')}</Label>
            <Input
              id={`${id}-display`}
              value={draft.display_name}
              onChange={(e) => {
                set({ display_name: e.target.value })
              }}
            />
            <Label htmlFor={`${id}-email`}>{t('users.fields.email')}</Label>
            <Input
              id={`${id}-email`}
              type="email"
              value={draft.email}
              onChange={(e) => {
                set({ email: e.target.value })
              }}
            />
            <Label htmlFor={`${id}-role`}>{t('users.fields.role')}</Label>
            <NativeSelect
              id={`${id}-role`}
              value={draft.role}
              onChange={(e) => {
                set({ role: e.target.value as RoleView })
              }}
            >
              {assignableRoles(me).map((role) => (
                <option key={role} value={role}>
                  {t(`roles.${role}`)}
                </option>
              ))}
            </NativeSelect>
            <label className="flex items-center gap-2 text-sm">
              <Checkbox
                checked={draft.setPassword}
                onCheckedChange={(checked) => {
                  set({ setPassword: checked })
                }}
              />
              {t('users.create.setPassword')}
            </label>
            {draft.setPassword ? (
              <Input
                aria-label={t('users.fields.password')}
                type="password"
                autoComplete="new-password"
                required
                value={draft.password}
                onChange={(e) => {
                  set({ password: e.target.value })
                }}
              />
            ) : (
              <p className="text-xs text-muted-foreground">{t('users.create.generatedHint')}</p>
            )}
            {mutation.error ? <ErrorAlert error={mutation.error} /> : null}
            <DialogFooter>
              <Button type="submit" disabled={mutation.isPending} aria-busy={mutation.isPending}>
                {t('users.create.submit')}
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>
      <SecretRevealDialog
        open={created?.temporary_password !== undefined}
        title={t('users.create.secretTitle', { username: created?.user.username ?? '' })}
        description={t('users.create.secretDescription')}
        secret={created?.temporary_password ?? ''}
        onDone={() => {
          mutation.reset()
        }}
      />
    </>
  )
}
