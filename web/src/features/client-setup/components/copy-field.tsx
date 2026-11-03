import { Check, Copy } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { copyToClipboard } from '@/lib/clipboard'

interface CopyFieldProps {
  label: string
  value: string
  /** Shown under the field (what it is for, why it is empty). */
  hint?: string | undefined
  /** A text area for long values (strings, commands). */
  multiline?: boolean
}

type CopyState = 'idle' | 'copied' | 'failed'

/** A read-only value with a copy button; selects itself on focus for manual copying. */
export function CopyField({ label, value, hint, multiline = false }: CopyFieldProps) {
  const { t } = useTranslation()
  // The outcome belongs to the value that was copied: a different value starts afresh.
  const [outcome, setOutcome] = useState<{ value: string; state: CopyState }>({
    value,
    state: 'idle',
  })
  const state: CopyState = outcome.value === value ? outcome.state : 'idle'
  const copy = async () => {
    setOutcome({ value, state: (await copyToClipboard(value)) ? 'copied' : 'failed' })
  }
  const field = {
    readOnly: true,
    value,
    'aria-label': label,
    spellCheck: false,
    onFocus: (event: { currentTarget: { select: () => void } }) => {
      event.currentTarget.select()
    },
  }
  return (
    <div className="grid gap-1.5">
      <p className="text-sm font-medium">{label}</p>
      <div className="flex items-start gap-2">
        {multiline ? (
          <textarea
            {...field}
            rows={3}
            className="w-full resize-none rounded-lg border bg-muted px-3 py-2 font-mono text-sm break-all focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
          />
        ) : (
          <Input {...field} className="font-mono" />
        )}
        <Button
          variant="outline"
          size="sm"
          disabled={value === ''}
          aria-label={t('pages.clientSetup.copy', { label })}
          onClick={() => void copy()}
        >
          {state === 'copied' ? <Check aria-hidden /> : <Copy aria-hidden />}
          {t('secret.copy')}
        </Button>
      </div>
      <p role="status" className="text-xs text-muted-foreground">
        {state === 'copied' ? t('secret.copied') : null}
        {state === 'failed' ? t('secret.copyFailed') : null}
        {state === 'idle' ? hint : null}
      </p>
    </div>
  )
}
