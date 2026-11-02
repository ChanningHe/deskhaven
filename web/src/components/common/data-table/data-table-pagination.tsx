import { ChevronLeft, ChevronRight } from 'lucide-react'
import { useId } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'

import { PAGE_SIZES, pageCount } from './table-search'

interface DataTablePaginationProps {
  page: number
  pageSize: number
  total: number
  onPageChange: (page: number) => void
  onPageSizeChange: (pageSize: number) => void
}

export function DataTablePagination({
  page,
  pageSize,
  total,
  onPageChange,
  onPageSizeChange,
}: DataTablePaginationProps) {
  const { t } = useTranslation()
  const pageSizeId = useId()
  const pages = pageCount(total, pageSize)

  return (
    <nav
      aria-label={t('dataTable.pagination')}
      className="flex flex-wrap items-center justify-between gap-3 text-sm text-muted-foreground"
    >
      <p aria-live="polite">{t('dataTable.pageSummary', { page, pages, total })}</p>
      <div className="flex items-center gap-3">
        <div className="flex items-center gap-2">
          <label htmlFor={pageSizeId}>{t('dataTable.pageSize')}</label>
          <select
            id={pageSizeId}
            value={pageSize}
            onChange={(event) => {
              onPageSizeChange(Number(event.target.value))
            }}
            className="h-8 rounded-lg border border-input bg-background px-2 text-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
          >
            {PAGE_SIZES.map((size) => (
              <option key={size} value={size}>
                {size}
              </option>
            ))}
          </select>
        </div>
        <div className="flex items-center gap-1">
          <Button
            variant="outline"
            size="icon"
            aria-label={t('dataTable.previousPage')}
            disabled={page <= 1}
            onClick={() => {
              onPageChange(page - 1)
            }}
          >
            <ChevronLeft aria-hidden />
          </Button>
          <Button
            variant="outline"
            size="icon"
            aria-label={t('dataTable.nextPage')}
            disabled={page >= pages}
            onClick={() => {
              onPageChange(page + 1)
            }}
          >
            <ChevronRight aria-hidden />
          </Button>
        </div>
      </div>
    </nav>
  )
}
