import { keepPreviousData, useQueryClient } from '@tanstack/react-query'
import { useNavigate, useSearch } from '@tanstack/react-router'
import { KeyRound } from 'lucide-react'
import { useCallback, useMemo, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { revokeApiToken, useListApiTokens } from '@/api/generated/api-tokens/api-tokens'
import type { ApiTokenView } from '@/api/generated/model'
import { ConfirmDialog } from '@/components/common/confirm-dialog'
import { applyTableSearch, DataTable, toListParams } from '@/components/common/data-table'
import type { SearchChangeOptions } from '@/components/common/data-table'
import { EmptyState } from '@/components/common/empty-state'
import { PageHeader } from '@/components/common/page-header'
import { useMe } from '@/features/auth/session'

import { invalidateTokens } from '../invalidate'
import { TOKEN_SORTABLE } from '../tokens-search'
import type { TokensSearch } from '../tokens-search'
import { CreateTokenButton } from './create-token-dialog'
import { tokenColumns } from './token-columns'

export function TokensPage() {
  const { t, i18n } = useTranslation()
  const me = useMe()
  const queryClient = useQueryClient()
  const search = useSearch({ from: '/_app/tokens' })
  const navigate = useNavigate({ from: '/tokens' })
  const query = useListApiTokens(toListParams(search), {
    query: { placeholderData: keepPreviousData },
  })
  // `revoking` outlives `confirming`: the dialog keeps its texts while it animates closed.
  const [revoking, setRevoking] = useState<ApiTokenView | undefined>(undefined)
  const [confirming, setConfirming] = useState(false)

  const onSearchChange = useCallback(
    (patch: Partial<TokensSearch>, options?: SearchChangeOptions) => {
      void navigate({
        search: (previous: TokensSearch) => applyTableSearch(previous, patch),
        replace: options?.replace ?? false,
      })
    },
    [navigate],
  )
  const askRevoke = useCallback((token: ApiTokenView) => {
    setRevoking(token)
    setConfirming(true)
  }, [])
  const columns = useMemo(
    () => tokenColumns(t, i18n.language, me, askRevoke),
    [t, i18n.language, me, askRevoke],
  )

  return (
    <div className="space-y-6">
      <PageHeader
        title={t('nav.tokens')}
        description={t('pages.tokens.description')}
        actions={<CreateTokenButton />}
      />
      <DataTable
        id="tokens"
        label={t('nav.tokens')}
        columns={columns}
        sortable={TOKEN_SORTABLE}
        data={query.data}
        getRowId={(token) => String(token.id)}
        search={search}
        onSearchChange={onSearchChange}
        error={query.error ?? undefined}
        onRetry={() => void query.refetch()}
        searchPlaceholder={t('tokens.search')}
        onClearFilters={() => {
          onSearchChange({ q: undefined })
        }}
        emptyState={
          <EmptyState
            icon={KeyRound}
            title={t('pages.tokens.emptyTitle')}
            description={t('pages.tokens.emptyDescription')}
          />
        }
      />
      <ConfirmDialog
        open={confirming}
        onOpenChange={setConfirming}
        title={t('tokens.revoke.title')}
        description={t('tokens.revoke.description', { name: revoking?.name ?? '' })}
        confirmLabel={t('tokens.revoke.confirm')}
        onConfirm={async () => {
          if (revoking === undefined) return
          await revokeApiToken(revoking.id)
          await invalidateTokens(queryClient)
        }}
      />
    </div>
  )
}
