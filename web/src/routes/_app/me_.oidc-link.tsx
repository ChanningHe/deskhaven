import { createFileRoute } from '@tanstack/react-router'

import type { OidcAnswer } from '@/features/auth/components/oidc-callback-page'
import { OidcLinkPage } from '@/features/me/components/oidc-link-page'

function text(value: unknown): string | undefined {
  return typeof value === 'string' && value !== '' ? value : undefined
}

// The OIDC callback (`<public_url>/api/oidc/callback`) hands a link flow on to this page
// with the identity provider's query; not nested under /me (its own page, no tabs).
export const Route = createFileRoute('/_app/me_/oidc-link')({
  validateSearch: (search: Record<string, unknown>): OidcAnswer => ({
    state: text(search.state),
    code: text(search.code),
    error: text(search.error),
  }),
  component: function OidcLinkRoute() {
    return <OidcLinkPage answer={Route.useSearch()} />
  },
})
