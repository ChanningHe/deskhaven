import { createFileRoute } from '@tanstack/react-router'

import { GroupsPage } from '@/features/groups/components/groups-page'
import { parseGroupsSearch } from '@/features/groups/groups-search'
import { requirePagePermission } from '@/features/auth/permissions'

export const Route = createFileRoute('/_app/groups')({
  validateSearch: parseGroupsSearch,
  beforeLoad: ({ context }) => {
    requirePagePermission(context.me.role, 'groups')
  },
  component: GroupsPage,
})
