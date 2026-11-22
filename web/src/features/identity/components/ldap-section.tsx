import { ShieldAlert } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import { useListLdapDirectories } from '@/api/generated/identity/identity'
import type { LdapDirectoryView } from '@/api/generated/model'
import { ErrorAlert } from '@/components/common/error-alert'
import { SectionCard } from '@/components/common/section-card'
import { SourceBadge } from '@/components/common/source-badge'
import { Badge } from '@/components/ui/badge'

import { directoryDraft, shownDirectory } from '../ldap-form'
import type { DirtyReporter } from '../report-dirty'
import { DirectoryFields } from './directory-fields'
import { DirectoryForm } from './directory-form'
import { LdapDiagnostics } from './ldap-diagnostics'

const ignore = () => undefined

/**
 * The LDAP directory. Only the owner may change it or run tests (06 §3.6); admins see it
 * read-only. A directory from the configuration file is read-only for everyone.
 */
interface LdapSectionProps {
  isOwner: boolean
  onDirtyChange: DirtyReporter
}

export function LdapSection({ isOwner, onDirtyChange }: LdapSectionProps) {
  const { t } = useTranslation()
  const query = useListLdapDirectories()
  const directory = query.data === undefined ? undefined : shownDirectory(query.data)
  const managed = directory?.managed_by === 'config'
  const status =
    query.data === undefined || query.error ? undefined : <SavedState directory={directory} />
  const alert = query.error ? (
    <ErrorAlert error={query.error} onRetry={() => void query.refetch()} />
  ) : null

  if (query.data !== undefined && isOwner && !managed) {
    return (
      <DirectoryForm
        key={directory?.id ?? 'new'}
        directory={directory}
        status={status}
        alert={alert}
        onDirtyChange={onDirtyChange}
      />
    )
  }
  return (
    <>
      <SectionCard
        title={t('identity.ldap.title')}
        description={t('identity.ldap.description')}
        status={status}
      >
        <div className="grid gap-6">
          {isOwner ? null : (
            <p className="flex items-start gap-2 text-sm text-muted-foreground">
              <ShieldAlert aria-hidden className="mt-0.5 size-4 shrink-0" />
              {t('identity.ldap.ownerOnly')}
            </p>
          )}
          {alert}
          {query.data === undefined ? null : (
            <ReadOnlyDirectory directory={directory} managed={managed} />
          )}
        </div>
      </SectionCard>
      {isOwner && directory !== undefined ? (
        <LdapDiagnostics target={{ directory_id: directory.id }} />
      ) : null}
    </>
  )
}

/** The stored directory's state, never the form's draft. */
function SavedState({ directory }: { directory: LdapDirectoryView | undefined }) {
  const { t } = useTranslation()
  if (directory === undefined) {
    return <Badge variant="outline">{t('identity.ldap.status.unconfigured')}</Badge>
  }
  return (
    <>
      <Badge variant={directory.enabled ? 'success' : 'secondary'}>
        {directory.enabled ? t('identity.ldap.status.enabled') : t('identity.ldap.status.disabled')}
      </Badge>
      {directory.managed_by === 'config' ? <SourceBadge source={{ kind: 'file' }} /> : null}
    </>
  )
}

interface ReadOnlyDirectoryProps {
  directory: LdapDirectoryView | undefined
  managed: boolean
}

function ReadOnlyDirectory({ directory, managed }: ReadOnlyDirectoryProps) {
  const { t } = useTranslation()
  if (directory === undefined) {
    return <p className="text-sm text-muted-foreground">{t('identity.ldap.empty')}</p>
  }
  return (
    <>
      {managed ? (
        <p className="text-xs text-muted-foreground">{t('identity.ldap.readOnly')}</p>
      ) : null}
      <DirectoryFields
        draft={directoryDraft(directory)}
        set={ignore}
        stored={{ hasPassword: directory.has_bind_password }}
        needsPassword={false}
        disabled
      />
    </>
  )
}
