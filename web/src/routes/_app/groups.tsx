import { createFileRoute } from '@tanstack/react-router'

import { GroupsPage } from '@/features/groups/components/groups-page'

export const Route = createFileRoute('/_app/groups')({ component: GroupsPage })
