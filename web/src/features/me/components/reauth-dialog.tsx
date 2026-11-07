import { useState } from 'react'
import type { SubmitEvent } from 'react'
import { useTranslation } from 'react-i18next'

import { ErrorAlert } from '@/components/common/error-alert'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { TextField } from '@/features/auth/components/text-field'

interface ReauthDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  title: string
  description: string
  submitLabel: string
  /** Also ask for an authenticator or recovery code (default); without it the code is ''. */
  askCode?: boolean
  /** Performs the action with the typed password and code; a rejection is shown and keeps the dialog open. */
  onSubmit: (currentPassword: string, code: string) => Promise<unknown>
}

// Rendered inside the popup, which unmounts on close: every opening starts empty.
function ReauthForm({
  submitLabel,
  askCode = true,
  onSubmit,
  onDone,
}: Pick<ReauthDialogProps, 'submitLabel' | 'askCode' | 'onSubmit'> & { onDone: () => void }) {
  const { t } = useTranslation()
  const [password, setPassword] = useState('')
  const [code, setCode] = useState('')
  const [pending, setPending] = useState(false)
  const [error, setError] = useState<unknown>(undefined)

  const submit = async (event: SubmitEvent<HTMLFormElement>) => {
    event.preventDefault()
    // The dialog may be rendered inside a form: its submit must not reach it.
    event.stopPropagation()
    if (pending) return
    setPending(true)
    setError(undefined)
    try {
      await onSubmit(password, code.trim())
      onDone()
    } catch (caught) {
      setError(caught)
      setPassword('')
      setCode('')
    } finally {
      setPending(false)
    }
  }

  return (
    <form className="grid gap-4" onSubmit={(event) => void submit(event)}>
      <TextField
        id="me-reauth-password"
        label={t('accountSetup.currentPassword')}
        type="password"
        autoComplete="current-password"
        required
        value={password}
        onChange={(event) => {
          setPassword(event.target.value)
        }}
      />
      {askCode ? (
        <TextField
          id="me-reauth-code"
          label={t('me.tfa.proofCode')}
          autoComplete="one-time-code"
          spellCheck={false}
          required
          value={code}
          onChange={(event) => {
            setCode(event.target.value)
          }}
        />
      ) : null}
      {error === undefined ? null : <ErrorAlert error={error} />}
      <DialogFooter>
        <Button type="submit" disabled={pending} aria-busy={pending}>
          {submitLabel}
        </Button>
      </DialogFooter>
    </form>
  )
}

/** Asks for the current password again before a sensitive account change (06 §3.3). */
export function ReauthDialog({
  open,
  onOpenChange,
  title,
  description,
  ...form
}: ReauthDialogProps) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>{description}</DialogDescription>
        </DialogHeader>
        <ReauthForm
          {...form}
          onDone={() => {
            onOpenChange(false)
          }}
        />
      </DialogContent>
    </Dialog>
  )
}
