import { keepPreviousData } from '@tanstack/react-query'
import { Link, useNavigate, useSearch } from '@tanstack/react-router'
import { Monitor } from 'lucide-react'
import { useCallback, useMemo } from 'react'
import { useTranslation } from 'react-i18next'

import { useListDevices } from '@/api/generated/devices/devices'
import { applyTableSearch, DataTable, toListParams } from '@/components/common/data-table'
import type { SearchChangeOptions } from '@/components/common/data-table'
import { EmptyState } from '@/components/common/empty-state'
import { PageHeader } from '@/components/common/page-header'
import { buttonVariants } from '@/components/ui/button'

import { DEVICE_SORTABLE, devicesListParams, hasDeviceFilters } from '../devices-search'
import type { DevicesSearch } from '../devices-search'
import { deviceColumns } from './device-columns'
import { DeviceDrawer } from './device-drawer'
import { DeviceFilters } from './device-filters'

/** Online status changes within seconds; the list follows (08 §2). */
const LIST_REFRESH_MS = 15_000

export function DevicesPage() {
  const { t, i18n } = useTranslation()
  const search = useSearch({ from: '/_app/devices' })
  const navigate = useNavigate({ from: '/devices' })
  // The previous page stays on screen while the next one loads (no flash of the skeleton).
  const query = useListDevices(devicesListParams(search, toListParams(search)), {
    query: { placeholderData: keepPreviousData, refetchInterval: LIST_REFRESH_MS },
  })

  const onSearchChange = useCallback(
    (patch: Partial<DevicesSearch>, options?: SearchChangeOptions) => {
      void navigate({
        search: (previous: DevicesSearch) => applyTableSearch(previous, patch),
        replace: options?.replace ?? false,
      })
    },
    [navigate],
  )
  // The drawer is not a filter: opening it keeps the page. Opening adds a history entry;
  // closing (Escape, close button, after a delete) replaces it, so Back never reopens it.
  const openDevice = useCallback(
    (device: number) => {
      void navigate({ search: (previous: DevicesSearch) => ({ ...previous, device }) })
    },
    [navigate],
  )
  const closeDevice = useCallback(() => {
    void navigate({
      search: (previous: DevicesSearch) => ({ ...previous, device: undefined }),
      replace: true,
    })
  }, [navigate])
  const columns = useMemo(
    () => deviceColumns(t, i18n.language, openDevice),
    [t, i18n.language, openDevice],
  )

  return (
    <div className="space-y-6">
      <PageHeader title={t('nav.devices')} description={t('pages.devices.description')} />
      <DataTable
        id="devices"
        label={t('devices.tableLabel')}
        columns={columns}
        sortable={DEVICE_SORTABLE}
        data={query.data}
        getRowId={(device) => String(device.id)}
        search={search}
        onSearchChange={onSearchChange}
        error={query.error ?? undefined}
        onRetry={() => void query.refetch()}
        searchPlaceholder={t('devices.search')}
        filters={<DeviceFilters search={search} onChange={onSearchChange} />}
        isFiltered={hasDeviceFilters(search)}
        onClearFilters={() => {
          onSearchChange({
            q: undefined,
            online: undefined,
            device_group: undefined,
            status: undefined,
          })
        }}
        emptyState={
          <EmptyState
            icon={Monitor}
            title={t('pages.devices.emptyTitle')}
            description={t('pages.devices.emptyDescription')}
            action={
              // A link styled as a button keeps the link role (Base UI's Button
              // would stamp role="button" onto the <a>).
              <Link to="/client-setup" className={buttonVariants({ variant: 'outline' })}>
                {t('pages.devices.emptyAction')}
              </Link>
            }
          />
        }
      />
      <DeviceDrawer deviceId={search.device} onClose={closeDevice} />
    </div>
  )
}
