import { createFileRoute } from '@tanstack/react-router'

import { UsersPage } from '@/features/users/components/users-page'
import { parseUsersSearch } from '@/features/users/users-search'
import { requirePagePermission } from '@/features/auth/permissions'

export const Route = createFileRoute('/_app/users')({
  validateSearch: parseUsersSearch,
  beforeLoad: ({ context }) => {
    requirePagePermission(context.me.role, 'users')
  },
  component: UsersPage,
})
