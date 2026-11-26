import { createFileRoute } from '@tanstack/react-router'

import { SetupPage } from '@/features/setup/components/setup-page'

export const Route = createFileRoute('/setup')({ component: SetupPage })
