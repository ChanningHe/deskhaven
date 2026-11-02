import { createFileRoute } from '@tanstack/react-router'

import { AddressBooksPage } from '@/features/addressbooks/components/address-books-page'

export const Route = createFileRoute('/_app/address-books')({ component: AddressBooksPage })
