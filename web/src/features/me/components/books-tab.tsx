import { useTranslation } from 'react-i18next'

import { useListMyAddressBooks } from '@/api/generated/me/me'
import { ErrorAlert } from '@/components/common/error-alert'
import { Badge } from '@/components/ui/badge'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table'

import { Section } from './section'

/** The label key of a rule number: 1 read-only, 2 read-write, 3 full control. */
function ruleKey(rule: number) {
  if (rule >= 3) return 'fullControl'
  return rule === 2 ? 'readWrite' : 'readOnly'
}

/** The personal book and the shared books the user holds a rule on (edited in the client). */
export function BooksTab() {
  const { t } = useTranslation()
  const query = useListMyAddressBooks()
  const books = query.data?.items ?? []
  const shown = books.filter((book) => book.kind === 'shared').length
  const sharedTotal = query.data?.shared_total ?? 0

  return (
    <Section title={t('me.books.title')} description={t('me.books.description')}>
      {query.isError ? (
        <ErrorAlert
          error={query.error}
          title={t('me.books.loadFailed')}
          onRetry={() => void query.refetch()}
        />
      ) : null}
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>{t('me.books.columns.name')}</TableHead>
            <TableHead>{t('me.books.columns.kind')}</TableHead>
            <TableHead>{t('me.books.columns.owner')}</TableHead>
            <TableHead>{t('me.books.columns.rule')}</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {books.map((book) => (
            <TableRow key={book.guid}>
              <TableCell className="font-medium">{book.name}</TableCell>
              <TableCell>
                <Badge variant="outline">{t(`me.books.kind.${book.kind}`)}</Badge>
              </TableCell>
              <TableCell>
                {book.kind === 'personal' ? '—' : (book.owner ?? t('me.books.adminManaged'))}
              </TableCell>
              <TableCell>{t(`me.books.rule.${ruleKey(book.rule)}`)}</TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>
      {sharedTotal > shown ? (
        <p className="text-xs text-muted-foreground">
          {t('me.books.truncated', { shown, total: sharedTotal })}
        </p>
      ) : null}
      <p className="text-xs text-muted-foreground">{t('me.books.editInClient')}</p>
    </Section>
  )
}
