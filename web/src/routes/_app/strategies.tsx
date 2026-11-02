import { createFileRoute } from '@tanstack/react-router'

import { StrategiesPage } from '@/features/strategies/components/strategies-page'

export const Route = createFileRoute('/_app/strategies')({ component: StrategiesPage })
