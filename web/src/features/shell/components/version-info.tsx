import { useTranslation } from 'react-i18next'

import { cn } from '@/lib/utils'

import { useServerVersion } from '../version'

interface VersionInfoProps {
  className?: string
}

/** The server version and the source link (AGPL-3.0 section 13); nothing until known. */
export function VersionInfo({ className }: VersionInfoProps) {
  const { t } = useTranslation()
  const info = useServerVersion()
  if (info === undefined) return null

  return (
    <p className={cn('flex flex-wrap gap-x-2 text-xs text-muted-foreground', className)}>
      <span>v{info.version}</span>
      <a
        href={info.sourceUrl}
        target="_blank"
        rel="noopener noreferrer"
        className="underline-offset-4 hover:text-foreground hover:underline"
      >
        {t('shell.sourceCode')}
      </a>
    </p>
  )
}
