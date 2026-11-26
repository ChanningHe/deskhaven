import { createFileRoute } from '@tanstack/react-router'

import type { OidcAnswer } from '@/features/auth/components/oidc-callback-page'
import { OidcCallbackPage } from '@/features/auth/components/oidc-callback-page'

function text(value: unknown): string | undefined {
  return typeof value === 'string' && value !== '' ? value : undefined
}

// The OIDC callback hands a console login on here; not nested under /login.
export const Route = createFileRoute('/login_/oidc')({
  validateSearch: (search: Record<string, unknown>): OidcAnswer => ({
    state: text(search.state),
    code: text(search.code),
    error: text(search.error),
  }),
  component: function OidcCallbackRoute() {
    return <OidcCallbackPage answer={Route.useSearch()} />
  },
})
