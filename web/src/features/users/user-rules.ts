// Who may manage whom (06 §4.1), mirrored from the admin API so the console
// only offers what the server will accept. The server enforces every rule.

import type { Me, RoleView } from '@/api/generated/model'

/** Whether `me` may change, reset or delete an account of `targetRole`. */
export function canManage(me: Me, targetRole: RoleView): boolean {
  return me.role === 'owner' || (me.role === 'admin' && targetRole !== 'owner')
}

/** Roles `me` may give an account. Only owners grant the owner role. */
export function assignableRoles(me: Me): RoleView[] {
  return me.role === 'owner' ? ['user', 'admin', 'owner'] : ['user', 'admin']
}

/** Nobody deletes their own account. */
export function canDelete(me: Me, target: { id: number; role: RoleView }): boolean {
  return target.id !== me.id && canManage(me, target.role)
}
