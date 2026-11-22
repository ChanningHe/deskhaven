import { createFileRoute } from '@tanstack/react-router'

import { IdentityPage } from '@/features/identity/components/identity-page'
import { parseIdentitySearch } from '@/features/identity/identity-search'
import { requirePagePermission } from '@/features/auth/permissions'

export const Route = createFileRoute('/_app/identity')({
  validateSearch: parseIdentitySearch,
  beforeLoad: ({ context }) => {
    requirePagePermission(context.me.role, 'identity')
  },
  component: IdentityPage,
})
