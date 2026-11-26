import type { ClientVersions } from '@/api/generated/model'

export type BarKind = 'version' | 'other' | 'unknown'

export interface Bar {
  key: string
  kind: BarKind
  version: string
  count: number
  /** Width in percent of the plot: the largest bar is 100. */
  width: number
}

type BarRow = Omit<Bar, 'width'>

export function versionBars(versions: ClientVersions): Bar[] {
  const listed: BarRow[] = versions.versions.map(({ version, count }) => ({
    key: `v:${version}`,
    kind: 'version',
    version,
    count,
  }))
  const rest: BarRow[] = [
    { key: 'other', kind: 'other', version: '', count: versions.other },
    { key: 'unknown', kind: 'unknown', version: '', count: versions.unknown },
  ]
  const rows = [...listed, ...rest.filter((row) => row.count > 0)]
  const largest = Math.max(1, ...rows.map((row) => row.count))
  return rows.map((row) => ({ ...row, width: (row.count / largest) * 100 }))
}
