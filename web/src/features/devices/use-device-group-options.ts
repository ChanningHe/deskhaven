import { useListDeviceGroups } from '@/api/generated/device-groups/device-groups'

/** The API's page size limit. */
const MAX_OPTIONS = 200

/** More than 200 groups would need a search box; none is expected. */
export function useDeviceGroupOptions() {
  const query = useListDeviceGroups({ page_size: MAX_OPTIONS, sort: 'name' })
  return query.data?.items ?? []
}
