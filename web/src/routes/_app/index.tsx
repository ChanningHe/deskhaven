import { createFileRoute } from '@tanstack/react-router'

import { OverviewPage } from '@/features/overview/components/overview-page'

export const Route = createFileRoute('/_app/')({ component: OverviewPage })
