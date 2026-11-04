import { useNavigate, useSearch } from '@tanstack/react-router'
import { useTranslation } from 'react-i18next'

import { useGetAddressBook } from '@/api/generated/address-books/address-books'
import { ErrorAlert } from '@/components/common/error-alert'
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from '@/components/ui/sheet'
import { Tabs, TabsList, TabsPanel, TabsTab } from '@/components/ui/tabs'
import { httpStatusOf } from '@/lib/http-error'

import { BOOK_TABS, DEFAULT_BOOK_TAB } from '../addressbooks-search'
import type { AddressBooksSearch, BookTab } from '../addressbooks-search'
import { BookDetails } from './book-details'
import { BookEntries } from './book-entries'
import { BookShares } from './book-shares'
import { BookTags } from './book-tags'

const HTTP_NOT_FOUND = 404

interface BookDrawerProps {
  guid: string | undefined
  onClose: () => void
}

/** Side drawer of one shared book: entries, tags, grants and the book's own details. */
export function BookDrawer({ guid, onClose }: BookDrawerProps) {
  const { t } = useTranslation()
  return (
    <Sheet
      open={guid !== undefined}
      onOpenChange={(open) => {
        if (!open) onClose()
      }}
    >
      <SheetContent closeLabel={t('common.close')} className="w-full overflow-y-auto sm:max-w-3xl">
        {guid === undefined ? null : <DrawerBody guid={guid} onDeleted={onClose} />}
      </SheetContent>
    </Sheet>
  )
}

function DrawerBody({ guid, onDeleted }: { guid: string; onDeleted: () => void }) {
  const { t } = useTranslation()
  const search = useSearch({ from: '/_app/address-books' })
  const navigate = useNavigate({ from: '/address-books' })
  const query = useGetAddressBook(guid)
  if (query.data === undefined) {
    return (
      <SheetHeader>
        <SheetTitle>{t('addressBooks.drawer.title')}</SheetTitle>
        {query.error ? (
          <ErrorAlert error={query.error} onRetry={() => void query.refetch()} />
        ) : (
          <SheetDescription>{t('common.loading')}</SheetDescription>
        )}
      </SheetHeader>
    )
  }
  const book = query.data
  // A later refetch failed: what follows is the last good copy (or the book is gone).
  const failure = query.isError ? query.error : undefined
  const gone = httpStatusOf(failure) === HTTP_NOT_FOUND
  const tab: BookTab = search.tab ?? DEFAULT_BOOK_TAB
  return (
    <>
      <SheetHeader>
        <SheetTitle>{book.name}</SheetTitle>
        <SheetDescription>
          {book.owner === null
            ? t('addressBooks.drawer.managedByAdmins')
            : t('addressBooks.drawer.ownedBy', { owner: book.owner })}
        </SheetDescription>
      </SheetHeader>
      {gone ? (
        <p role="alert" className="mx-4 rounded-lg bg-muted px-3 py-2 text-sm">
          {t('addressBooks.drawer.gone')}
        </p>
      ) : (
        <div className="grid gap-4 px-4 pb-6">
          {failure === undefined ? null : (
            <ErrorAlert
              error={failure}
              title={t('addressBooks.drawer.stale')}
              onRetry={() => void query.refetch()}
            />
          )}
          <Tabs
            value={tab}
            onValueChange={(next: BookTab) => {
              void navigate({
                search: (previous: AddressBooksSearch) => ({
                  ...previous,
                  tab: next === 'entries' ? undefined : next,
                }),
                replace: true,
              })
            }}
          >
            <TabsList aria-label={t('addressBooks.drawer.tabsLabel')}>
              {BOOK_TABS.map((name) => (
                <TabsTab key={name} value={name}>
                  {t(`addressBooks.drawer.tabs.${name}`)}
                </TabsTab>
              ))}
            </TabsList>
            <TabsPanel value="entries">
              <BookEntries book={book} />
            </TabsPanel>
            <TabsPanel value="tags">
              <BookTags guid={guid} />
            </TabsPanel>
            <TabsPanel value="shares">
              <BookShares guid={guid} />
            </TabsPanel>
            <TabsPanel value="details">
              <BookDetails key={book.guid} book={book} onDeleted={onDeleted} />
            </TabsPanel>
          </Tabs>
        </div>
      )}
    </>
  )
}
