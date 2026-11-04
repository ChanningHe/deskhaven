import type { ReactNode } from 'react'

/** The cell that opens a record's drawer: a button, so it is reachable by keyboard. */
export function OpenButton({ onOpen, children }: { onOpen: () => void; children: ReactNode }) {
  return (
    <button
      type="button"
      className="text-left font-medium underline-offset-4 hover:underline focus-visible:underline focus-visible:outline-none"
      onClick={onOpen}
    >
      {children}
    </button>
  )
}
