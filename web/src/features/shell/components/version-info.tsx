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
    <p className={cn('flex text-xs text-muted-foreground', className)}>
      <a
        href={info.sourceUrl}
        target="_blank"
        rel="noopener noreferrer"
        title={t('shell.sourceCode')}
        className="truncate underline-offset-4 hover:text-foreground hover:underline"
      >
        v{info.version}
      </a>
    </p>
  )
}
