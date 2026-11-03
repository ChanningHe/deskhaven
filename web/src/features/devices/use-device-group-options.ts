import { useListDeviceGroups } from '@/api/generated/device-groups/device-groups'

/** Most device groups offered in a select (the API's page size limit). */
const MAX_OPTIONS = 200

/**
 * Every device group by name, for the group select and filter. A fleet with more than
 * 200 groups would need a search box; none is expected.
 */
export function useDeviceGroupOptions() {
  const query = useListDeviceGroups({ page_size: MAX_OPTIONS, sort: 'name' })
  return query.data?.items ?? []
}
