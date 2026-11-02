import type { Updater, ColumnVisibilityState } from '@tanstack/react-table'
import { useCallback, useState } from 'react'

import { readStorage, writeStorage } from '@/lib/storage'

// Column visibility is a per-browser view preference, not part of a shareable
// URL, so it is persisted in localStorage per table.

export function columnVisibilityStorageKey(tableId: string): string {
  return `deskhaven.table.${tableId}.columns`
}

function readVisibility(tableId: string): ColumnVisibilityState {
  const raw = readStorage(columnVisibilityStorageKey(tableId))
  if (raw === null) return {}
  try {
    const parsed: unknown = JSON.parse(raw)
    if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) return {}
    return Object.fromEntries(
      Object.entries(parsed).filter((entry): entry is [string, boolean] => {
        return typeof entry[1] === 'boolean'
      }),
    )
  } catch {
    return {}
  }
}

export function useColumnVisibility(
  tableId: string,
): [ColumnVisibilityState, (updater: Updater<ColumnVisibilityState>) => void] {
  const [visibility, setVisibility] = useState(() => readVisibility(tableId))

  const update = useCallback(
    (updater: Updater<ColumnVisibilityState>) => {
      setVisibility((previous) => {
        const next = typeof updater === 'function' ? updater(previous) : updater
        writeStorage(columnVisibilityStorageKey(tableId), JSON.stringify(next))
        return next
      })
    },
    [tableId],
  )

  return [visibility, update]
}
