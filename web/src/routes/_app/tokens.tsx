import { createFileRoute } from '@tanstack/react-router'

import { TokensPage } from '@/features/tokens/components/tokens-page'
import { parseTokensSearch } from '@/features/tokens/tokens-search'
import { requirePagePermission } from '@/features/auth/permissions'

export const Route = createFileRoute('/_app/tokens')({
  validateSearch: parseTokensSearch,
  beforeLoad: ({ context }) => {
    requirePagePermission(context.me.role, 'tokens')
  },
  component: TokensPage,
})
