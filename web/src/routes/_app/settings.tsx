import { createFileRoute } from '@tanstack/react-router'

import { SettingsPage } from '@/features/settings/components/settings-page'
import { requirePagePermission } from '@/features/auth/permissions'

export const Route = createFileRoute('/_app/settings')({
  beforeLoad: ({ context }) => {
    requirePagePermission(context.me.role, 'settings')
  },
  component: SettingsPage,
})
