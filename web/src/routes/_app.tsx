import { createFileRoute } from '@tanstack/react-router'

import { AppShell } from '@/features/shell/components/app-shell'

// Pathless layout: every console page renders inside the sidebar + top bar shell.
export const Route = createFileRoute('/_app')({ component: AppShell })
