import { KeyRound, Pencil, Plus, Trash2 } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import { useListOidcProviderConfigs } from '@/api/generated/identity/identity'
import type { OidcProviderView } from '@/api/generated/model'
import { EmptyState } from '@/components/common/empty-state'
import { ErrorAlert } from '@/components/common/error-alert'
import { SectionCard } from '@/components/common/section-card'
import { SourceBadge } from '@/components/common/source-badge'
import { enabledVariant } from '@/components/common/status-variant'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table'
import { CopyField } from '@/features/client-setup/components/copy-field'

import { useProviderDialogs } from './use-provider-dialogs'

/**
 * OIDC providers: the callback URL to register, the list, create / edit / delete. Writes are
 * the owner's (a provider can mint administrators); admins see the providers read-only.
 */
export function OidcSection({ isOwner }: { isOwner: boolean }) {
  const { t } = useTranslation()
  const query = useListOidcProviderConfigs()
  const { edit, remove, dialogs } = useProviderDialogs(query.data?.items, !isOwner)

  return (
    <SectionCard
      title={t('identity.oidc.title')}
      description={t('identity.oidc.description')}
      actions={
        isOwner ? (
          <Button
            onClick={() => {
              edit({ kind: 'new' })
            }}
          >
            <Plus aria-hidden />
            {t('identity.oidc.add')}
          </Button>
        ) : undefined
      }
    >
      <div className="space-y-6">
        {isOwner ? null : (
          <p className="text-sm text-muted-foreground">{t('identity.oidc.ownerOnly')}</p>
        )}
        {query.error ? (
          <ErrorAlert error={query.error} onRetry={() => void query.refetch()} />
        ) : null}
        {query.data === undefined ? null : (
          <>
            <div className="max-w-2xl">
              <CopyField
                label={t('identity.oidc.callbackUrl')}
                value={query.data.callback_url ?? ''}
                hint={
                  query.data.callback_url == null
                    ? t('identity.oidc.callbackMissing')
                    : t('identity.oidc.callbackHint')
                }
              />
            </div>
            {query.data.items.length === 0 ? (
              <EmptyState
                icon={KeyRound}
                headingLevel={3}
                title={t('identity.oidc.emptyTitle')}
                description={t('identity.oidc.emptyDescription')}
              />
            ) : (
              <ProviderTable
                providers={query.data.items}
                isOwner={isOwner}
                onEdit={(provider) => {
                  edit({ kind: 'stored', provider })
                }}
                onDelete={remove}
              />
            )}
          </>
        )}
      </div>
      {dialogs}
    </SectionCard>
  )
}

interface ProviderTableProps {
  providers: readonly OidcProviderView[]
  /** Admins only view providers: no delete, "view" instead of "edit". */
  isOwner: boolean
  onEdit: (provider: OidcProviderView) => void
  onDelete: (provider: OidcProviderView) => void
}

const HEAD = 'h-9 text-xs text-muted-foreground first:pl-3 last:pr-3'

function ProviderTable({ providers, isOwner, onEdit, onDelete }: ProviderTableProps) {
  const { t } = useTranslation()
  return (
    <div className="overflow-hidden rounded-lg border">
      <Table aria-label={t('identity.oidc.tableLabel')}>
        <TableHeader className="bg-muted/50">
          <TableRow className="hover:bg-transparent">
            <TableHead className={HEAD}>{t('identity.oidc.columns.name')}</TableHead>
            <TableHead className={HEAD}>{t('identity.oidc.columns.issuer')}</TableHead>
            <TableHead className={HEAD}>{t('identity.oidc.columns.status')}</TableHead>
            <TableHead className={HEAD}>{t('identity.oidc.columns.source')}</TableHead>
            <TableHead className={`${HEAD} text-right`}>
              {t('identity.oidc.columns.actions')}
            </TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {providers.map((provider) => (
            <TableRow key={provider.id}>
              <TableCell className="pl-3">
                <div className="font-medium">{provider.display_name}</div>
                <div className="font-mono text-xs text-muted-foreground">{provider.name}</div>
              </TableCell>
              <TableCell className="font-mono text-xs break-all">{provider.issuer}</TableCell>
              <TableCell>
                <Badge variant={enabledVariant(provider.enabled)}>
                  {provider.enabled
                    ? t('identity.ldap.status.enabled')
                    : t('identity.ldap.status.disabled')}
                </Badge>
              </TableCell>
              <TableCell>
                {provider.managed_by === 'config' ? (
                  <SourceBadge source={{ kind: 'file' }} />
                ) : null}
              </TableCell>
              <TableCell className="pr-3 text-right whitespace-nowrap">
                <Button
                  variant="ghost"
                  size="icon-sm"
                  aria-label={t(isOwner ? 'identity.oidc.edit' : 'identity.oidc.view', {
                    name: provider.name,
                  })}
                  onClick={() => {
                    onEdit(provider)
                  }}
                >
                  <Pencil aria-hidden />
                </Button>
                {!isOwner || provider.managed_by === 'config' ? null : (
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    aria-label={t('identity.oidc.delete', { name: provider.name })}
                    onClick={() => {
                      onDelete(provider)
                    }}
                  >
                    <Trash2 aria-hidden />
                  </Button>
                )}
              </TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>
    </div>
  )
}
