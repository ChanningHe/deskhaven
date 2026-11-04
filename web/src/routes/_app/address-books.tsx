import { createFileRoute } from '@tanstack/react-router'

import { requirePagePermission } from '@/features/auth/permissions'
import { parseAddressBooksSearch } from '@/features/addressbooks/addressbooks-search'
import { AddressBooksPage } from '@/features/addressbooks/components/address-books-page'

export const Route = createFileRoute('/_app/address-books')({
  validateSearch: parseAddressBooksSearch,
  beforeLoad: ({ context }) => {
    requirePagePermission(context.me.role, 'addressBooks')
  },
  component: AddressBooksPage,
})
