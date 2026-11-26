import { createFileRoute } from '@tanstack/react-router'

import { ChangePasswordPage } from '@/features/auth/components/change-password-page'
import { requireSetupStep } from '@/features/auth/session'

export const Route = createFileRoute('/account/password')({
  beforeLoad: async ({ context, location }) => ({
    me: await requireSetupStep(context.queryClient, location.href, 'password_change'),
  }),
  component: ChangePasswordPage,
})
