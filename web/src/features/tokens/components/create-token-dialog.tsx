import { useQueryClient } from '@tanstack/react-query'
import { Plus, TriangleAlert } from 'lucide-react'
import { useId, useState } from 'react'
import type { SubmitEvent } from 'react'
import { useTranslation } from 'react-i18next'

import { useCreateApiToken } from '@/api/generated/api-tokens/api-tokens'
import type { ScopeView } from '@/api/generated/model'
import { ErrorAlert } from '@/components/common/error-alert'
import { NativeSelect } from '@/components/common/native-select'
import { SecretRevealDialog } from '@/components/common/secret-reveal-dialog'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { SECRET_MUTATION } from '@/features/users/secret-mutation'

import { invalidateTokens } from '../invalidate'
import {
  DEFAULT_EXPIRY,
  EXPIRY_CHOICES,
  expiryFromChoice,
  NAME_MAX_LENGTH,
  SCOPE_KEYS,
  SCOPES,
} from '../token-rules'
import type { ExpiryChoice } from '../token-rules'

function toggled(scopes: readonly ScopeView[], scope: ScopeView, on: boolean): ScopeView[] {
  const others = scopes.filter((candidate) => candidate !== scope)
  return on ? SCOPES.filter((s) => s === scope || others.includes(s)) : others
}

/** "New token": the form, then the secret exactly once (the API returns it only here). */
export function CreateTokenButton() {
  const { t } = useTranslation()
  const id = useId()
  const queryClient = useQueryClient()
  const [open, setOpen] = useState(false)
  const [name, setName] = useState('')
  const [scopes, setScopes] = useState<readonly ScopeView[]>([])
  const [expiry, setExpiry] = useState<ExpiryChoice>(DEFAULT_EXPIRY)
  const mutation = useCreateApiToken(SECRET_MUTATION)
  const ready = name.trim() !== '' && scopes.length > 0

  const resetForm = () => {
    setName('')
    setScopes([])
    setExpiry(DEFAULT_EXPIRY)
  }

  const submit = async (event: SubmitEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (!ready) return
    const data = {
      name: name.trim(),
      scopes: [...scopes],
      expires_at: expiryFromChoice(expiry, Date.now()),
    }
    const created = await mutation.mutateAsync({ data }).catch(() => undefined)
    if (created === undefined) return
    setOpen(false)
    resetForm()
    void invalidateTokens(queryClient)
  }

  return (
    <>
      <Button
        onClick={() => {
          mutation.reset()
          setOpen(true)
        }}
      >
        <Plus aria-hidden />
        {t('tokens.create.action')}
      </Button>
      <Dialog
        open={open}
        onOpenChange={(next) => {
          setOpen(next)
          // Cancelled: the next opening starts empty, without the last error.
          if (!next) {
            resetForm()
            if (!mutation.isPending) mutation.reset()
          }
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t('tokens.create.title')}</DialogTitle>
          </DialogHeader>
          <form className="grid gap-3" onSubmit={(event) => void submit(event)}>
            <Label htmlFor={`${id}-name`}>{t('tokens.fields.name')}</Label>
            <Input
              id={`${id}-name`}
              required
              maxLength={NAME_MAX_LENGTH}
              value={name}
              placeholder={t('tokens.fields.namePlaceholder')}
              onChange={(e) => {
                setName(e.target.value)
              }}
            />
            <fieldset className="grid gap-2">
              <legend className="mb-1 text-sm font-medium">{t('tokens.fields.scopes')}</legend>
              {SCOPES.map((scope) => (
                <label key={scope} className="flex items-start gap-2 text-sm">
                  <Checkbox
                    className="mt-0.5"
                    checked={scopes.includes(scope)}
                    onCheckedChange={(checked) => {
                      setScopes((current) => toggled(current, scope, checked))
                    }}
                  />
                  <span>
                    <span className="font-mono">{scope}</span>{' '}
                    <span className="text-muted-foreground">
                      {t(`tokens.scopes.${SCOPE_KEYS[scope]}`)}
                    </span>
                  </span>
                </label>
              ))}
            </fieldset>
            <Label htmlFor={`${id}-expiry`}>{t('tokens.fields.expiry')}</Label>
            <NativeSelect
              id={`${id}-expiry`}
              className="h-9"
              value={expiry}
              onChange={(e) => {
                setExpiry(e.target.value as ExpiryChoice)
              }}
            >
              {EXPIRY_CHOICES.map((choice) => (
                <option key={choice} value={choice}>
                  {t(`tokens.expiry.${choice}`)}
                </option>
              ))}
            </NativeSelect>
            {expiry === 'never' ? (
              <p className="flex items-start gap-2 rounded-lg bg-warning/10 px-3 py-2 text-sm text-warning">
                <TriangleAlert aria-hidden className="mt-0.5 size-4 shrink-0" />
                {t('tokens.expiry.neverWarning')}
              </p>
            ) : null}
            {mutation.error ? <ErrorAlert error={mutation.error} /> : null}
            <DialogFooter>
              <Button type="submit" disabled={!ready || mutation.isPending}>
                {t('tokens.create.submit')}
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>
      <SecretRevealDialog
        open={mutation.data !== undefined}
        title={t('tokens.reveal.title', { name: mutation.data?.token.name ?? '' })}
        description={t('tokens.reveal.description')}
        secret={mutation.data?.secret ?? ''}
        onDone={() => {
          mutation.reset()
        }}
      />
    </>
  )
}
