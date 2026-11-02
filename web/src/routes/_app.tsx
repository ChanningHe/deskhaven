import { createFileRoute } from '@tanstack/react-router'

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
// shell, and so do unknown console URLs (see `_app/$.tsx`).
export const Route = createFileRoute('/_app')({
  component: AppShell,
  notFoundComponent: ShellNotFound,
})
