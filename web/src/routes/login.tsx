import { createFileRoute } from '@tanstack/react-router'

import { LoginPage } from '@/features/auth/components/login-page'
import { safeRedirectTarget } from '@/features/auth/session'

interface LoginSearch {
  /** Same-origin path to return to after signing in (validated). */
  redirect?: string | undefined
}

export const Route = createFileRoute('/login')({
  validateSearch: (search: Record<string, unknown>): LoginSearch => {
    // Always set the key: the router keeps unvalidated keys from the raw URL otherwise.
    return { redirect: safeRedirectTarget(search.redirect) }
  },
  component: LoginPage,
})
