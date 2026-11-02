import { createFileRoute } from '@tanstack/react-router'

import { TokensPage } from '@/features/tokens/components/tokens-page'
import { requirePagePermission } from '@/features/auth/permissions'

export const Route = createFileRoute('/_app/tokens')({
  beforeLoad: ({ context }) => {
    requirePagePermission(context.me.role, 'tokens')
  },
  component: TokensPage,
})
