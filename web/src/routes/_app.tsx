import { createFileRoute } from '@tanstack/react-router'

import { requireSession } from '@/features/auth/session'
import { AppShell } from '@/features/shell/components/app-shell'
import { NotFound } from '@/features/shell/components/not-found'

// A route's notFoundComponent replaces its component, so the shell is
// rendered explicitly around the not-found content to keep the navigation.
function ShellNotFound() {
  return (
    <AppShell>
      <NotFound />
    </AppShell>
  )
}

// Pathless layout: every console page renders inside the sidebar + top bar
// shell, and so do unknown console URLs (see `_app/$.tsx`). The guard loads the
// session (or redirects to /login) and hands `me` to every child route, which
// checks its own page permission in `beforeLoad`.
export const Route = createFileRoute('/_app')({
  beforeLoad: async ({ context, location }) => ({
    me: await requireSession(context.queryClient, location.href),
  }),
  component: AppShell,
  notFoundComponent: ShellNotFound,
})
