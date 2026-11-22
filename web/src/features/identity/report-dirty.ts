import { useEffect } from 'react'

export type DirtyReporter = (dirty: boolean) => void

/** Tells the page whether this form holds unsaved edits; an unmounted form holds none. */
export function useReportDirty(dirty: boolean, report: DirtyReporter) {
  useEffect(() => {
    report(dirty)
    return () => {
      report(false)
    }
  }, [dirty, report])
}
