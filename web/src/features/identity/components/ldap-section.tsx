import { ShieldAlert } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import { useListLdapDirectories } from '@/api/generated/identity/identity'
import type { LdapDirectoryView } from '@/api/generated/model'
import { ErrorAlert } from '@/components/common/error-alert'
import { SourceBadge } from '@/components/common/source-badge'

import { directoryDraft } from '../ldap-form'
import { DirectoryFields } from './directory-fields'
import { DirectoryForm } from './directory-form'
import { Section } from './form-controls'
import { LdapDiagnostics } from './ldap-diagnostics'

const ignore = () => undefined

/** The directory P0 uses: the enabled one, else the first. */
function shownDirectory(directories: readonly LdapDirectoryView[]): LdapDirectoryView | undefined {
  return directories.find((d) => d.enabled) ?? directories[0]
}

/**
 * The LDAP directory. Only the owner may change it or run tests (06 §3.6); admins see it
 * read-only. A directory from the configuration file is read-only for everyone.
 */
export function LdapSection({ isOwner }: { isOwner: boolean }) {
  const { t } = useTranslation()
  const query = useListLdapDirectories()
  const directory = query.data === undefined ? undefined : shownDirectory(query.data)
  const managed = directory?.managed_by === 'config'

  return (
    <Section
      id="identity-ldap"
      title={t('identity.ldap.title')}
      description={t('identity.ldap.description')}
    >
      {isOwner ? null : (
        <p className="flex items-start gap-2 text-sm text-muted-foreground">
          <ShieldAlert aria-hidden className="mt-0.5 size-4 shrink-0" />
          {t('identity.ldap.ownerOnly')}
        </p>
      )}
      {query.error ? <ErrorAlert error={query.error} onRetry={() => void query.refetch()} /> : null}
      {query.data === undefined ? null : (
        <LdapBody directory={directory} isOwner={isOwner} managed={managed} />
      )}
    </Section>
  )
}

interface LdapBodyProps {
  directory: LdapDirectoryView | undefined
  isOwner: boolean
  managed: boolean
}

function LdapBody({ directory, isOwner, managed }: LdapBodyProps) {
  const { t } = useTranslation()
  if (isOwner && !managed) {
    return <DirectoryForm key={directory?.id ?? 'new'} directory={directory} />
  }
  if (directory === undefined) {
    return <p className="text-sm text-muted-foreground">{t('identity.ldap.empty')}</p>
  }
  return (
    <div className="grid gap-6">
      {managed ? (
        <p className="flex items-center gap-2 text-xs text-muted-foreground">
          <SourceBadge source={{ kind: 'file' }} />
          {t('identity.ldap.readOnly')}
        </p>
      ) : null}
      <DirectoryFields
        draft={directoryDraft(directory)}
        set={ignore}
        stored={{ hasPassword: directory.has_bind_password }}
        needsPassword={false}
        disabled
      />
      {isOwner ? <LdapDiagnostics target={{ directory_id: directory.id }} /> : null}
    </div>
  )
}
