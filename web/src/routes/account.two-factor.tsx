import { createFileRoute } from '@tanstack/react-router'

import { TotpEnrolmentPage } from '@/features/auth/components/totp-enrolment-page'
import { requireSetupStep } from '@/features/auth/session'

// Account setup step outside the console shell: only for a session that must
// enrol the second factor `auth.tfa.enforce` requires.
export const Route = createFileRoute('/account/two-factor')({
  beforeLoad: async ({ context, location }) => ({
    me: await requireSetupStep(context.queryClient, location.href, 'totp_enrolment'),
  }),
  component: TotpEnrolmentPage,
})
