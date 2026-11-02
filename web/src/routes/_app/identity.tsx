import { createFileRoute } from '@tanstack/react-router'

import { IdentityPage } from '@/features/auth/components/identity-page'

export const Route = createFileRoute('/_app/identity')({ component: IdentityPage })
