import { QueryClientProvider, useQuery } from '@tanstack/react-query'
import {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  Outlet,
  RouterProvider,
  useNavigate,
} from '@tanstack/react-router'
import { render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'

import { createQueryClient } from '@/app/query-client'

import {
  applyTableSearch,
  createDataTableColumnHelper,
  DataTable,
  parseTableSearch,
  toListParams,
} from '.'
import type { ListPage, ListParams, TableSearch } from '.'
import { columnVisibilityStorageKey } from './use-column-visibility'

interface Device {
  id: string
  name: string
  os: string
}

const DEVICES: Device[] = Array.from({ length: 45 }, (_, index) => ({
  id: `d${index + 1}`,
  name: `Device ${String(index + 1).padStart(2, '0')}`,
  os: index % 2 === 0 ? 'Linux' : 'Windows',
}))

const column = createDataTableColumnHelper<Device>()
const COLUMNS = column.columns([
  column.accessor('name', { header: 'Name', meta: { label: 'Name' } }),
  column.accessor('os', { header: 'OS', meta: { label: 'OS' } }),
])
const SORTABLE = ['name'] as const
const deviceName = (device: Device) => device.name

/** A fake list endpoint over `devices`: filter, sort, paginate. */
function listDevices(
  devices: readonly Device[],
  { page, page_size, sort, q }: ListParams,
): ListPage<Device> {
  const matching = devices.filter((device) => q === undefined || device.name.includes(q))
  const sorted = sort === '-name' ? [...matching].reverse() : matching
  const start = (page - 1) * page_size
  return { items: sorted.slice(start, start + page_size), total: matching.length, page, page_size }
}

interface RenderTableOptions {
  empty?: boolean
  /** Extra history entries before `url` (the last entry is the current one). */
  before?: string[]
  /** The server's rows; tests may change them and refetch. */
  rows?: { current: readonly Device[] }
}

function renderTable(
  url: string,
  { empty = false, before = [], rows = { current: DEVICES } }: RenderTableOptions = {},
) {
  const fetchList = vi.fn((params: ListParams) =>
    Promise.resolve(
      empty ? { items: [], total: 0, page: 1, page_size: 20 } : listDevices(rows.current, params),
    ),
  )
  const onBulk = vi.fn()
  const queryClient = createQueryClient()

  function DevicesTable() {
    const search = devicesRoute.useSearch()
    const navigate = useNavigate()
    const params = toListParams(search)
    const query = useQuery({ queryKey: ['devices', params], queryFn: () => fetchList(params) })
    const onSearchChange = (patch: TableSearch, options?: { replace?: boolean }) => {
      // The test router is not the registered app router, so its paths are untyped here.
      void navigate({
        search: ((previous: TableSearch) => applyTableSearch(previous, patch)) as never,
        replace: options?.replace ?? false,
      })
    }
    return (
      <DataTable
        id="devices"
        label="Devices"
        columns={COLUMNS}
        sortable={SORTABLE}
        data={query.data}
        getRowId={(device) => device.id}
        getRowLabel={deviceName}
        search={search}
        onSearchChange={onSearchChange}
        searchPlaceholder="Search devices"
        bulkActions={(ids) => (
          <button
            type="button"
            onClick={() => {
              onBulk(ids)
            }}
          >
            Delete selected
          </button>
        )}
        emptyState={<p>No devices yet</p>}
      />
    )
  }

  const rootRoute = createRootRoute({ component: Outlet })
  const homeRoute = createRoute({
    getParentRoute: () => rootRoute,
    path: '/',
    component: () => <h1>Home</h1>,
  })
  const devicesRoute = createRoute({
    getParentRoute: () => rootRoute,
    path: '/devices',
    validateSearch: (raw: Record<string, unknown>) => parseTableSearch(raw, { sortable: SORTABLE }),
    component: DevicesTable,
  })
  const entries = [...before, url]
  const router = createRouter({
    routeTree: rootRoute.addChildren([homeRoute, devicesRoute]),
    history: createMemoryHistory({ initialEntries: entries, initialIndex: entries.length - 1 }),
  })
  render(
    <QueryClientProvider client={queryClient}>
      <RouterProvider router={router} />
    </QueryClientProvider>,
  )
  return { router, fetchList, onBulk, queryClient, user: userEvent.setup() }
}

function rowNames() {
  const table = screen.getByRole('table', { name: 'Devices' })
  return within(table)
    .getAllByRole('row')
    .slice(1)
    .map((row) => within(row).getAllByRole('cell')[1]?.textContent)
}

function rowCheckbox(index: number): HTMLElement {
  const checkbox = screen.getAllByRole('checkbox', { name: /^选择 Device / })[index]
  if (!checkbox) throw new Error(`no row checkbox #${index}`)
  return checkbox
}

describe('DataTable URL sync', () => {
  it('loads the state from the URL and requests exactly that page', async () => {
    const { fetchList } = renderTable('/devices?page=2&page_size=10&sort=-name&q=Device')

    expect(await screen.findByText('Device 35')).toBeInTheDocument()
    expect(fetchList).toHaveBeenLastCalledWith({
      page: 2,
      page_size: 10,
      sort: '-name',
      q: 'Device',
    })
    expect(screen.getByRole('searchbox', { name: '搜索' })).toHaveValue('Device')
  })

  it('writes the search box to `q` after a pause and resets the page', async () => {
    const { router, user } = renderTable('/devices?page=2')
    await screen.findByText('Device 21')

    await user.type(screen.getByRole('searchbox', { name: '搜索' }), 'Device 4')

    await waitFor(() => {
      expect(router.state.location.search).toEqual({ q: 'Device 4' })
    })
    await waitFor(() => {
      expect(rowNames()).toEqual([40, 41, 42, 43, 44, 45].map((n) => `Device ${n}`))
    })
  })

  it('keeps what is being typed when the URL catches up (trailing space)', async () => {
    const { router, user } = renderTable('/devices')
    await screen.findByText('Device 01')
    const box = screen.getByRole('searchbox', { name: '搜索' })

    await user.type(box, 'Device ')
    await waitFor(() => {
      expect(router.state.location.search).toEqual({ q: 'Device' })
    })
    expect(box).toHaveValue('Device ')
    await user.type(box, '4')

    expect(box).toHaveValue('Device 4')
    await waitFor(() => {
      expect(router.state.location.search).toEqual({ q: 'Device 4' })
    })
  })

  it('follows the URL when it changes underneath (Back)', async () => {
    const { router, user } = renderTable('/devices')
    await screen.findByText('Device 01')
    const box = screen.getByRole('searchbox', { name: '搜索' })
    await user.type(box, 'Device 4{Enter}')
    await waitFor(() => {
      expect(router.state.location.search).toEqual({ q: 'Device 4' })
    })

    router.history.back()

    await waitFor(() => {
      expect(box).toHaveValue('')
    })
  })

  it('corrects an out-of-range page in place, so Back is not trapped', async () => {
    const { router } = renderTable('/devices?page=5', { before: ['/'] })

    await waitFor(() => {
      expect(router.state.location.search).toEqual({ page: 3 })
    })
    await screen.findByText('Device 41')
    // The correction replaced ?page=5 instead of pushing ?page=3 on top of it.
    expect(router.history.length).toBe(2)

    router.history.back()

    expect(await screen.findByRole('heading', { name: 'Home' })).toBeInTheDocument()
    expect(router.state.location.pathname).toBe('/')
  })
})

describe('DataTable columns and selection', () => {
  it('hides columns from the column picker and remembers the choice', async () => {
    const { user } = renderTable('/devices')
    await screen.findByText('Device 01')

    await user.click(screen.getByRole('button', { name: '列' }))
    await user.click(await screen.findByRole('menuitemcheckbox', { name: 'OS' }))

    await waitFor(() => {
      expect(screen.queryByRole('columnheader', { name: 'OS' })).not.toBeInTheDocument()
    })
    expect(window.localStorage.getItem(columnVisibilityStorageKey('devices'))).toBe('{"os":false}')
  })

  it('selects rows for bulk actions and clears the selection on a new page', async () => {
    const { user, onBulk } = renderTable('/devices')
    await screen.findByText('Device 01')

    await user.click(rowCheckbox(0))
    await user.click(rowCheckbox(2))
    await user.click(screen.getByRole('button', { name: 'Delete selected' }))
    expect(onBulk).toHaveBeenCalledWith(['d1', 'd3'])

    await user.click(screen.getByRole('checkbox', { name: '选择本页全部行' }))
    await user.click(screen.getByRole('button', { name: 'Delete selected' }))
    expect(onBulk).toHaveBeenLastCalledWith(DEVICES.slice(0, 20).map((device) => device.id))

    await user.click(screen.getByRole('button', { name: '下一页' }))
    await screen.findByText('Device 21')
    expect(screen.queryByRole('button', { name: 'Delete selected' })).not.toBeInTheDocument()
  })

  it('drops selected rows that disappear after a refetch', async () => {
    const rows = { current: DEVICES }
    const { user, onBulk, queryClient } = renderTable('/devices', { rows })
    await screen.findByText('Device 01')
    await user.click(rowCheckbox(0))
    await user.click(rowCheckbox(2))

    // Someone else deletes Device 01; the list refetches on the same page.
    rows.current = DEVICES.filter((device) => device.id !== 'd1')
    await queryClient.invalidateQueries({ queryKey: ['devices'] })

    await waitFor(() => {
      expect(screen.queryByText('Device 01')).not.toBeInTheDocument()
    })
    await user.click(screen.getByRole('button', { name: 'Delete selected' }))
    expect(onBulk).toHaveBeenLastCalledWith(['d3'])
  })
})
