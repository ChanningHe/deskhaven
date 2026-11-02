import { useCallback, useState } from 'react'
import type { ReactNode } from 'react'
import { useTranslation } from 'react-i18next'

import { useResetUserPassword } from '@/api/generated/users/users'
import type { UserView } from '@/api/generated/model'
import { SecretRevealDialog } from '@/components/common/secret-reveal-dialog'

import { SECRET_MUTATION } from '../secret-mutation'

interface ResetPasswordReveal {
  /** Resets `user`'s password; the temporary one is then shown by `dialog`. */
  resetPassword: (user: UserView) => Promise<unknown>
  /** The reveal; render it where it outlives the drawer (the page). */
  dialog: ReactNode
}

/**
 * Password reset with its show-once reveal, owned by the page rather than the drawer: the
 * drawer closes with the URL (Back, Escape), and must not take the only copy of the
 * temporary password with it.
 */
export function useResetPasswordReveal(): ResetPasswordReveal {
  const { t } = useTranslation()
  const mutation = useResetUserPassword(SECRET_MUTATION)
  const [username, setUsername] = useState('')
  const { mutateAsync } = mutation
  const resetPassword = useCallback(
    (user: UserView) => {
      setUsername(user.username)
      return mutateAsync({ id: user.id })
    },
    [mutateAsync],
  )
  const dialog = (
    <SecretRevealDialog
      open={mutation.data !== undefined}
      title={t('users.actions.temporaryTitle', { username })}
      description={t('users.actions.temporaryDescription')}
      secret={mutation.data?.temporary_password ?? ''}
      onDone={() => {
        mutation.reset()
      }}
    />
  )
  return { resetPassword, dialog }
}
