import { useId, useState } from 'react'
import type { ReactNode } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import { Label } from '@/components/ui/label'
import { copyToClipboard } from '@/lib/clipboard'

const TEXTAREA_CLASS =
  'w-full rounded-lg border border-input bg-background px-2.5 py-1.5 font-mono text-sm outline-none focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50'

function Strip({ label, children }: { label: string; children: ReactNode }) {
  return (
    <section
      aria-label={label}
      className="absolute top-2 right-2 z-10 w-[min(24rem,calc(100%-1rem))] space-y-2 rounded-xl border bg-popover p-3 text-popover-foreground shadow-lg"
    >
      {children}
    </section>
  )
}

export function PastePanel({
  onSend,
  onClose,
}: {
  onSend: (text: string) => boolean
  onClose: () => void
}) {
  const { t } = useTranslation()
  const id = useId()
  const [text, setText] = useState('')
  const [result, setResult] = useState<'sent' | 'tooLong' | null>(null)
  return (
    <Strip label={t('remote.clipboard.pasteTitle')}>
      <p className="text-sm text-muted-foreground">{t('remote.clipboard.pasteHint')}</p>
      <Label htmlFor={id}>{t('remote.clipboard.pasteLabel')}</Label>
      <textarea
        id={id}
        rows={3}
        value={text}
        onChange={(event) => {
          setText(event.target.value)
          setResult(null)
        }}
        className={TEXTAREA_CLASS}
      />
      <div className="flex flex-wrap items-center gap-2">
        <Button
          size="sm"
          disabled={text === ''}
          onClick={() => {
            setResult(onSend(text) ? 'sent' : 'tooLong')
          }}
        >
          {t('remote.clipboard.send')}
        </Button>
        <Button size="sm" variant="outline" onClick={onClose}>
          {t('remote.clipboard.close')}
        </Button>
        <p role="status" className="text-sm text-muted-foreground">
          {result === null ? null : t(`remote.clipboard.${result}`)}
        </p>
      </div>
    </Strip>
  )
}

/** What the peer copied; copying it to the local clipboard is the user's call. */
export function RemoteClipboardPanel({ text, onClose }: { text: string; onClose: () => void }) {
  const { t } = useTranslation()
  const id = useId()
  const [copied, setCopied] = useState<boolean | null>(null)
  return (
    <Strip label={t('remote.clipboard.remoteTitle')}>
      <Label htmlFor={id}>{t('remote.clipboard.remoteTitle')}</Label>
      <textarea id={id} rows={3} readOnly value={text} className={TEXTAREA_CLASS} />
      <div className="flex flex-wrap items-center gap-2">
        <Button
          size="sm"
          onClick={() => {
            void copyToClipboard(text).then(setCopied)
          }}
        >
          {t('remote.clipboard.copy')}
        </Button>
        <Button size="sm" variant="outline" onClick={onClose}>
          {t('remote.clipboard.close')}
        </Button>
        <p role="status" className="text-sm text-muted-foreground">
          {copied === null
            ? null
            : t(copied ? 'remote.clipboard.copied' : 'remote.clipboard.copyFailed')}
        </p>
      </div>
    </Strip>
  )
}
