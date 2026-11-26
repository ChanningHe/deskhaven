// Mirrored from the admin API so the console only offers what the server will accept.

import type { Me, RoleView } from '@/api/generated/model'

export function canManage(me: Me, targetRole: RoleView): boolean {
  return me.role === 'owner' || (me.role === 'admin' && targetRole !== 'owner')
}

export function assignableRoles(me: Me): RoleView[] {
  return me.role === 'owner' ? ['user', 'admin', 'owner'] : ['user', 'admin']
}

export function canDelete(me: Me, target: { id: number; role: RoleView }): boolean {
  return target.id !== me.id && canManage(me, target.role)
}
