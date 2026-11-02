import { createFileRoute } from '@tanstack/react-router'

import { UsersPage } from '@/features/users/components/users-page'
import { requirePagePermission } from '@/features/auth/permissions'

export const Route = createFileRoute('/_app/users')({
  beforeLoad: ({ context }) => {
    requirePagePermission(context.me.role, 'users')
  },
  component: UsersPage,
})
