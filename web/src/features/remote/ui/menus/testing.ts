import { screen } from '@testing-library/react'
import type { UserEvent } from '@testing-library/user-event'

/** Opens a toolbar menu by its trigger's name; the popup mounts a tick after the click. */
export async function openMenu(user: UserEvent, name: string): Promise<HTMLElement> {
  await user.click(screen.getByRole('button', { name }))
  return screen.findByRole('menu', { name })
}
