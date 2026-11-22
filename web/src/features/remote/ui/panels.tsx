import { CircleAlert, LoaderCircle } from 'lucide-react'
import { useId, useState } from 'react'
import type { ReactNode, SubmitEvent } from 'react'
import { useTranslation } from 'react-i18next'

import { SectionCard } from '@/components/common/section-card'
import { Button } from '@/components/ui/button'
import { TextField } from '@/features/auth/components/text-field'

/** A card in the middle of the remote stage: the steps before the screen shows. */
function Step({
  title,
  description,
  children,
  footer,
}: {
  title: string
  description?: string | undefined
  children?: ReactNode
  footer?: ReactNode
}) {
  return (
    <SectionCard
      title={title}
      footer={footer}
      {...(description === undefined ? {} : { description })}
    >
      {children}
    </SectionCard>
  )
}

/** Something is under way; `onCancel` gives up on it. */
export function ProgressPanel({ text, onCancel }: { text: string; onCancel?: () => void }) {
  const { t } = useTranslation()
  return (
    <div className="flex flex-col items-center gap-4 rounded-xl border bg-card px-6 py-10 text-center text-card-foreground shadow-xs">
      <LoaderCircle aria-hidden className="size-6 animate-spin text-muted-foreground" />
      <p role="status" className="text-sm text-muted-foreground">
        {text}
      </p>
      {onCancel === undefined ? null : (
        <Button variant="outline" size="sm" onClick={onCancel}>
          {t('common.cancel')}
        </Button>
      )}
    </div>
  )
}

export function ApprovalPanel({ onCancel }: { onCancel: () => void }) {
  const { t } = useTranslation()
  return (
    <Step
      title={t('remote.approval.title')}
      description={t('remote.approval.description')}
      footer={
        <Button variant="outline" onClick={onCancel}>
          {t('common.cancel')}
        </Button>
      }
    >
      <p role="status" className="flex items-center gap-2 text-sm text-muted-foreground">
        <LoaderCircle aria-hidden className="size-4 animate-spin" />
        {t('remote.connecting')}
      </p>
    </Step>
  )
}

interface SecretPanelProps {
  kind: 'password' | 'twoFactor'
  /** The last attempt was wrong. */
  wrong: boolean
  onSubmit: (value: string) => void
  onCancel: () => void
}

/** The device password, or its two-factor code. */
export function SecretPanel({ kind, wrong, onSubmit, onCancel }: SecretPanelProps) {
  const { t } = useTranslation()
  const id = useId()
  const [value, setValue] = useState('')
  const submit = (event: SubmitEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (value === '') return
    onSubmit(value)
    setValue('')
  }
  return (
    <form onSubmit={submit}>
      <SectionCard
        title={t(`remote.${kind}.title`)}
        description={t(`remote.${kind}.description`)}
        footer={
          <>
            <Button type="submit" disabled={value === ''}>
              {t(`remote.${kind}.submit`)}
            </Button>
            <Button type="button" variant="outline" onClick={onCancel}>
              {t('common.cancel')}
            </Button>
          </>
        }
      >
        <div className="space-y-3">
          <TextField
            id={id}
            label={t(`remote.${kind}.label`)}
            type={kind === 'password' ? 'password' : 'text'}
            inputMode={kind === 'password' ? undefined : 'numeric'}
            autoComplete={kind === 'password' ? 'off' : 'one-time-code'}
            autoFocus
            value={value}
            aria-invalid={wrong || undefined}
            onChange={(event) => {
              setValue(event.target.value)
            }}
          />
          {wrong ? (
            <p role="alert" className="text-sm text-destructive">
              {t(`remote.${kind}.wrong`)}
            </p>
          ) : null}
        </div>
      </SectionCard>
    </form>
  )
}

/** Connecting failed or the session ended; the user may try again. */
export function ProblemPanel({
  title,
  message,
  onRetry,
  retryLabel,
}: {
  title: string
  message: string
  onRetry: () => void
  retryLabel: string
}) {
  return (
    <Step title={title} footer={<Button onClick={onRetry}>{retryLabel}</Button>}>
      <p role="alert" className="flex items-start gap-2 text-sm">
        <CircleAlert aria-hidden className="mt-0.5 size-4 shrink-0 text-destructive" />
        <span className="min-w-0 break-words">{message}</span>
      </p>
    </Step>
  )
}
