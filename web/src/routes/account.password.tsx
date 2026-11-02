import { createFileRoute } from '@tanstack/react-router'

import { ChangePasswordPage } from '@/features/auth/components/change-password-page'
import { requireSetupStep } from '@/features/auth/session'

// Account setup step outside the console shell: only for a session that must
// replace a temporary password (the server allows nothing else meanwhile).
export const Route = createFileRoute('/account/password')({
  beforeLoad: async ({ context, location }) => ({
    me: await requireSetupStep(context.queryClient, location.href, 'password_change'),
  }),
  component: ChangePasswordPage,
})
