import { createFileRoute, redirect } from '@tanstack/react-router'

import { hasPermission, PAGE_PERMISSIONS } from '@/features/auth/permissions'
import { OverviewPage } from '@/features/overview/components/overview-page'

export const Route = createFileRoute('/_app/')({
  beforeLoad: ({ context }) => {
    // Plain users have no overview; their console is the self-service page.
    if (!hasPermission(context.me.role, PAGE_PERMISSIONS.overview)) {
      // eslint-disable-next-line @typescript-eslint/only-throw-error -- router control flow
      throw redirect({ to: '/me', replace: true })
    }
  },
  component: OverviewPage,
})
