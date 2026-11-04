import { createFileRoute } from '@tanstack/react-router'

import { parseAuditSearch } from '@/features/audit/audit-search'
import { AuditPage } from '@/features/audit/components/audit-page'
import { requirePagePermission } from '@/features/auth/permissions'

export const Route = createFileRoute('/_app/audit')({
  validateSearch: parseAuditSearch,
  beforeLoad: ({ context }) => {
    requirePagePermission(context.me.role, 'audit')
  },
  component: AuditPage,
})
