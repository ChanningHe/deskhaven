import type { Me, RoleView } from '@/api/generated/model'

export function meFor(role: RoleView): Me {
  return {
    id: 1,
    username: role,
    display_name: `${role} user`,
    email: null,
    role,
    must_change_password: false,
    totp_enabled: true,
    setup_required: null,
  }
}

export const OWNER_ME = meFor('owner')
export const ADMIN_ME = meFor('admin')
export const USER_ME = meFor('user')
