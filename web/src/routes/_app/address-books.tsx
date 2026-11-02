import { createFileRoute } from '@tanstack/react-router'

import { AddressBooksPage } from '@/features/addressbooks/components/address-books-page'
import { requirePagePermission } from '@/features/auth/permissions'

export const Route = createFileRoute('/_app/address-books')({
  beforeLoad: ({ context }) => {
    requirePagePermission(context.me.role, 'addressBooks')
  },
  component: AddressBooksPage,
})
