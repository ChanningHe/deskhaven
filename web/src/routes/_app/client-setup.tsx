import { createFileRoute } from '@tanstack/react-router'

import { ClientSetupPage } from '@/features/client-setup/components/client-setup-page'

export const Route = createFileRoute('/_app/client-setup')({ component: ClientSetupPage })
