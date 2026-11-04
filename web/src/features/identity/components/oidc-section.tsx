import { useQueryClient } from '@tanstack/react-query'
import { KeyRound, Pencil, Plus, Trash2 } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import {
  deleteOidcProvider,
  getListOidcProviderConfigsQueryKey,
  useListOidcProviderConfigs,
} from '@/api/generated/identity/identity'
import type { OidcProviderView } from '@/api/generated/model'
import { ConfirmDialog } from '@/components/common/confirm-dialog'
import { EmptyState } from '@/components/common/empty-state'
import { ErrorAlert } from '@/components/common/error-alert'
import { SourceBadge } from '@/components/common/source-badge'
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

import { Section } from './form-controls'
import { ProviderDialog } from './provider-dialog'
import type { ProviderTarget } from './provider-dialog'

/** OIDC providers: the callback URL to register, the list, create / edit / delete. */
export function OidcSection() {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const query = useListOidcProviderConfigs()
  const [editing, setEditing] = useState<ProviderTarget | undefined>(undefined)
  // `deleting` outlives `confirming`: the dialog keeps its texts while it animates closed.
  const [deleting, setDeleting] = useState<OidcProviderView | undefined>(undefined)
  const [confirming, setConfirming] = useState(false)
  // The dialog edits the polled copy, so a concurrent change rebases the draft.
  const shown =
    editing?.kind === 'stored'
      ? {
          kind: 'stored' as const,
          provider: query.data?.items.find((p) => p.id === editing.provider.id) ?? editing.provider,
        }
      : editing

  return (
    <Section
      id="identity-oidc"
      title={t('identity.oidc.title')}
      description={t('identity.oidc.description')}
      actions={
        <Button
          onClick={() => {
            setEditing({ kind: 'new' })
          }}
        >
          <Plus aria-hidden />
          {t('identity.oidc.add')}
        </Button>
      }
    >
      {query.error ? <ErrorAlert error={query.error} onRetry={() => void query.refetch()} /> : null}
      {query.data === undefined ? null : (
        <>
          <CopyField
            label={t('identity.oidc.callbackUrl')}
            value={query.data.callback_url ?? ''}
            hint={
              query.data.callback_url == null
                ? t('identity.oidc.callbackMissing')
                : t('identity.oidc.callbackHint')
            }
          />
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
              onEdit={(provider) => {
                setEditing({ kind: 'stored', provider })
              }}
              onDelete={(provider) => {
                setDeleting(provider)
                setConfirming(true)
              }}
            />
          )}
        </>
      )}
      <ProviderDialog
        target={shown}
        onClose={() => {
          setEditing(undefined)
        }}
      />
      <ConfirmDialog
        open={confirming}
        onOpenChange={setConfirming}
        title={t('identity.oidc.confirmDelete.title')}
        description={t('identity.oidc.confirmDelete.description', { name: deleting?.name ?? '' })}
        confirmLabel={t('identity.oidc.confirmDelete.confirm')}
        confirmText={deleting?.name ?? ''}
        onConfirm={async () => {
          if (deleting === undefined) return
          await deleteOidcProvider(deleting.id)
          await queryClient.invalidateQueries({ queryKey: getListOidcProviderConfigsQueryKey() })
        }}
      />
    </Section>
  )
}

interface ProviderTableProps {
  providers: readonly OidcProviderView[]
  onEdit: (provider: OidcProviderView) => void
  onDelete: (provider: OidcProviderView) => void
}

function ProviderTable({ providers, onEdit, onDelete }: ProviderTableProps) {
  const { t } = useTranslation()
  return (
    <Table aria-label={t('identity.oidc.tableLabel')}>
      <TableHeader>
        <TableRow>
          <TableHead>{t('identity.oidc.columns.name')}</TableHead>
          <TableHead>{t('identity.oidc.columns.issuer')}</TableHead>
          <TableHead>{t('identity.oidc.columns.status')}</TableHead>
          <TableHead>{t('identity.oidc.columns.source')}</TableHead>
          <TableHead className="text-right">{t('identity.oidc.columns.actions')}</TableHead>
        </TableRow>
      </TableHeader>
      <TableBody>
        {providers.map((provider) => (
          <TableRow key={provider.id}>
            <TableCell>
              <div className="font-medium">{provider.display_name}</div>
              <div className="font-mono text-xs text-muted-foreground">{provider.name}</div>
            </TableCell>
            <TableCell className="font-mono text-xs break-all">{provider.issuer}</TableCell>
            <TableCell>
              <Badge variant={provider.enabled ? 'default' : 'secondary'}>
                {provider.enabled ? t('identity.oidc.enabled') : t('identity.oidc.disabled')}
              </Badge>
            </TableCell>
            <TableCell>
              {provider.managed_by === 'config' ? <SourceBadge source={{ kind: 'file' }} /> : null}
            </TableCell>
            <TableCell className="text-right whitespace-nowrap">
              <Button
                variant="ghost"
                size="icon-sm"
                aria-label={t('identity.oidc.edit', { name: provider.name })}
                onClick={() => {
                  onEdit(provider)
                }}
              >
                <Pencil aria-hidden />
              </Button>
              {provider.managed_by === 'config' ? null : (
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
  )
}
