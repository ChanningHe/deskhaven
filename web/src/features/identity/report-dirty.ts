import { useEffect } from 'react'

export type DirtyReporter = (dirty: boolean) => void

export function useReportDirty(dirty: boolean, report: DirtyReporter) {
  useEffect(() => {
    report(dirty)
    return () => {
      report(false)
    }
  }, [dirty, report])
}
