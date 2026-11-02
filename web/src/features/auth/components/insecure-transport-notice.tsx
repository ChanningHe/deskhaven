import { ShieldAlert } from 'lucide-react'
import { useTranslation } from 'react-i18next'

/**
 * Why sign-in cannot work here: the session cookies are `Secure`, and browsers
 * drop them on a plain-HTTP page. Shown when the server answers
 * `insecure_transport` (or the page itself is not a secure context).
 */
export function InsecureTransportNotice() {
  const { t } = useTranslation()
  return (
    <div
      role="alert"
      className="space-y-2 rounded-lg border border-destructive/30 bg-destructive/5 px-3 py-3 text-sm"
    >
      <p className="flex items-center gap-2 font-medium text-destructive">
        <ShieldAlert aria-hidden className="size-4 shrink-0" />
        {t('insecure.title')}
      </p>
      <p className="text-muted-foreground">{t('insecure.description')}</p>
      <ul className="list-disc space-y-1 pl-5 text-muted-foreground">
        <li>{t('insecure.proxy')}</li>
        <li>
          {t('insecure.tunnel')}
          <code className="mt-1 block rounded bg-muted px-2 py-1 font-mono text-xs break-all text-foreground">
            ssh -L 21114:127.0.0.1:21114 user@server
          </code>
        </li>
      </ul>
    </div>
  )
}
