import { Check, Copy, TriangleAlert } from 'lucide-react'
import { useRef, useState } from 'react'
import type { ReactNode } from 'react'
import { useTranslation } from 'react-i18next'

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

export interface SecretRevealDialogProps {
  open: boolean
  title: string
  description?: ReactNode
  /** The secret, shown exactly once (API token, recovery codes, setup link). */
  secret: string
  /**
   * Called when the user confirmed they saved it; the only way to close. Drop
   * the secret from memory here: it must not be shown again.
   */
  onDone: () => void
}

type CopyState = 'idle' | 'copied' | 'failed'

async function copyToClipboard(text: string): Promise<boolean> {
  // The Clipboard API exists only in secure contexts (https or localhost).
  if (typeof navigator.clipboard === 'undefined') return false
  try {
    await navigator.clipboard.writeText(text)
    return true
  } catch {
    return false
  }
}

function SecretBody({
  title,
  secret,
  onDone,
}: Omit<SecretRevealDialogProps, 'open' | 'description'>) {
  const { t } = useTranslation()
  const fieldRef = useRef<HTMLTextAreaElement>(null)
  const [copyState, setCopyState] = useState<CopyState>('idle')
  const [acknowledged, setAcknowledged] = useState(false)
  const rows = Math.min(secret.split('\n').length, 12)

  const copy = async () => {
    const copied = await copyToClipboard(secret)
    setCopyState(copied ? 'copied' : 'failed')
    // Without clipboard access, focus (which selects) the text to copy by hand.
    if (!copied) fieldRef.current?.focus()
  }

  return (
    <>
      <div className="grid gap-2">
        <textarea
          ref={fieldRef}
          readOnly
          aria-label={title}
          value={secret}
          rows={rows}
          spellCheck={false}
          onFocus={(event) => {
            event.currentTarget.select()
          }}
          className="w-full resize-none rounded-lg border bg-muted px-3 py-2 font-mono text-sm break-all text-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
        />
        <div className="flex items-center gap-2">
          <Button variant="outline" size="sm" onClick={() => void copy()}>
            {copyState === 'copied' ? <Check aria-hidden /> : <Copy aria-hidden />}
            {t('secret.copy')}
          </Button>
          <p role="status" className="text-xs text-muted-foreground">
            {copyState === 'copied' ? t('secret.copied') : null}
            {copyState === 'failed' ? t('secret.copyFailed') : null}
          </p>
        </div>
      </div>
      <label className="flex items-center gap-2 text-sm">
        <Checkbox
          checked={acknowledged}
          onCheckedChange={(checked) => {
            setAcknowledged(checked)
          }}
        />
        {t('secret.acknowledge')}
      </label>
      <DialogFooter>
        <Button disabled={!acknowledged} onClick={onDone}>
          {t('secret.done')}
        </Button>
      </DialogFooter>
    </>
  )
}

/**
 * Shows a secret once (08 §4): copy button, and closing requires confirming it
 * was saved. Escape and outside clicks do not close it, so the secret cannot be
 * lost by accident.
 */
export function SecretRevealDialog({
  open,
  title,
  description,
  secret,
  onDone,
}: SecretRevealDialogProps) {
  const { t } = useTranslation()
  return (
    <Dialog open={open} disablePointerDismissal>
      <DialogContent showCloseButton={false} className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          {description === undefined ? null : <DialogDescription>{description}</DialogDescription>}
        </DialogHeader>
        <p className="flex items-start gap-2 rounded-lg bg-warning/10 px-3 py-2 text-sm text-warning">
          <TriangleAlert aria-hidden className="mt-0.5 size-4 shrink-0" />
          {t('secret.warning')}
        </p>
        <SecretBody title={title} secret={secret} onDone={onDone} />
      </DialogContent>
    </Dialog>
  )
}
