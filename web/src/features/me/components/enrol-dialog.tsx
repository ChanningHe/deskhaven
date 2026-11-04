import { useState } from 'react'
import type { SubmitEvent } from 'react'
import { useTranslation } from 'react-i18next'

import type { TotpConfirm, TotpEnrolment } from '@/api/generated/model'
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
import { QrCode } from '@/features/auth/components/qr-code'
import { TextField } from '@/features/auth/components/text-field'
import { groupedSecret } from '@/features/auth/secret-format'

interface EnrolDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  /** The pending seed; `undefined` while it is being created (or failed: see `beginError`). */
  enrolment: TotpEnrolment | undefined
  beginError: unknown
  onRetry: () => void
  /** Confirms with a code; a rejection is shown and keeps the dialog open. */
  onConfirm: (body: TotpConfirm) => Promise<unknown>
}

// Rendered inside the popup, which unmounts on close: every opening starts empty.
function ConfirmForm({ onConfirm }: Pick<EnrolDialogProps, 'onConfirm'>) {
  const { t } = useTranslation()
  const [password, setPassword] = useState('')
  const [code, setCode] = useState('')
  const [pending, setPending] = useState(false)
  const [error, setError] = useState<unknown>(undefined)

  const submit = async (event: SubmitEvent<HTMLFormElement>) => {
    event.preventDefault()
    event.stopPropagation()
    if (pending) return
    setPending(true)
    setError(undefined)
    try {
      // The current password re-authenticates: a stolen session cannot bind another
      // authenticator.
      await onConfirm({ current_password: password, code: code.trim() })
    } catch (caught) {
      setError(caught)
      setCode('')
    } finally {
      setPending(false)
    }
  }

  return (
    <form className="grid gap-4" onSubmit={(event) => void submit(event)}>
      <TextField
        id="me-totp-password"
        label={t('accountSetup.currentPassword')}
        type="password"
        autoComplete="current-password"
        required
        value={password}
        onChange={(event) => {
          setPassword(event.target.value)
        }}
      />
      <TextField
        id="me-totp-code"
        label={t('accountSetup.code')}
        autoComplete="one-time-code"
        inputMode="numeric"
        required
        value={code}
        onChange={(event) => {
          setCode(event.target.value)
        }}
      />
      {error === undefined ? null : <ErrorAlert error={error} />}
      <DialogFooter>
        <Button type="submit" disabled={pending} aria-busy={pending}>
          {t('accountSetup.enable')}
        </Button>
      </DialogFooter>
    </form>
  )
}

/** Scan (or type) the new seed, then confirm with the current password and a code. */
export function EnrolDialog({
  open,
  onOpenChange,
  enrolment,
  beginError,
  onRetry,
  onConfirm,
}: EnrolDialogProps) {
  const { t } = useTranslation()
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t('me.tfa.enrolTitle')}</DialogTitle>
          <DialogDescription>{t('me.tfa.enrolDescription')}</DialogDescription>
        </DialogHeader>
        {beginError === undefined || beginError === null ? null : (
          <ErrorAlert error={beginError} onRetry={onRetry} />
        )}
        {enrolment === undefined ? null : (
          <div className="grid gap-4">
            <div className="flex flex-col items-center gap-3">
              <QrCode qr={enrolment.qr} label={t('accountSetup.qrLabel')} />
              <p className="text-sm text-muted-foreground">{t('accountSetup.manual')}</p>
              <code className="rounded bg-muted px-2 py-1 font-mono text-sm break-all">
                {groupedSecret(enrolment.secret)}
              </code>
            </div>
            <ConfirmForm onConfirm={onConfirm} />
          </div>
        )}
      </DialogContent>
    </Dialog>
  )
}
