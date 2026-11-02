import { createFileRoute } from '@tanstack/react-router'

import { GroupsPage } from '@/features/groups/components/groups-page'
import { requirePagePermission } from '@/features/auth/permissions'

export const Route = createFileRoute('/_app/groups')({
  beforeLoad: ({ context }) => {
    requirePagePermission(context.me.role, 'groups')
  },
  component: GroupsPage,
})
