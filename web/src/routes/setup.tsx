import { createFileRoute } from '@tanstack/react-router'

import { SetupPage } from '@/features/setup/components/setup-page'

// First-run setup: `/setup#token=…` from the server log (no session needed).
export const Route = createFileRoute('/setup')({ component: SetupPage })
