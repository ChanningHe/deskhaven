import { LoaderCircle } from 'lucide-react'
import { useId, useState } from 'react'
import type { ReactNode, SubmitEvent } from 'react'
import { useTranslation } from 'react-i18next'

import { NativeSelect } from '@/components/common/native-select'
import { SectionCard } from '@/components/common/section-card'
import { Button } from '@/components/ui/button'
import { Label } from '@/components/ui/label'
import { TextField } from '@/features/auth/components/text-field'

import type { WindowsSessionChoice } from '../session/windows-sessions'

export function Step({
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
  wrong: boolean
  onSubmit: (value: string) => void
  onCancel: () => void
}

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

export function WindowsSessionPanel({
  choice,
  onChoose,
  onCancel,
}: {
  choice: WindowsSessionChoice
  onChoose: (sid: number) => void
  onCancel: () => void
}) {
  const { t } = useTranslation()
  const id = useId()
  const offered = choice.sessions.some((s) => s.sid === choice.current)
  const [sid, setSid] = useState(offered ? choice.current : (choice.sessions[0]?.sid ?? 0))
  const submit = (event: SubmitEvent<HTMLFormElement>) => {
    event.preventDefault()
    onChoose(sid)
  }
  return (
    <form onSubmit={submit}>
      <SectionCard
        title={t('remote.windowsSession.title')}
        description={t('remote.windowsSession.description')}
        footer={
          <>
            <Button type="submit">{t('remote.windowsSession.submit')}</Button>
            <Button type="button" variant="outline" onClick={onCancel}>
              {t('common.cancel')}
            </Button>
          </>
        }
      >
        <div className="space-y-2">
          <Label htmlFor={id}>{t('remote.windowsSession.label')}</Label>
          <NativeSelect
            id={id}
            className="w-full"
            value={sid}
            onChange={(event) => {
              setSid(Number(event.target.value))
            }}
          >
            {choice.sessions.map((s) => (
              <option key={s.sid} value={s.sid}>
                {s.sid === choice.current
                  ? t('remote.windowsSession.current', { name: s.name })
                  : s.name}
              </option>
            ))}
          </NativeSelect>
          {sid === choice.current ? null : (
            <p className="text-sm text-muted-foreground">{t('remote.windowsSession.switchNote')}</p>
          )}
        </div>
      </SectionCard>
    </form>
  )
}
