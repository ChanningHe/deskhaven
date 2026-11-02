import { createFileRoute } from '@tanstack/react-router'

import { AuditPage } from '@/features/audit/components/audit-page'
import { requirePagePermission } from '@/features/auth/permissions'

export const Route = createFileRoute('/_app/audit')({
  beforeLoad: ({ context }) => {
    requirePagePermission(context.me.role, 'audit')
  },
  component: AuditPage,
})
