import type { ReactNode } from 'react'

import { RemoteBar, type BarFrame, type BarStatus } from './remote-bar'

export function Waiting({
  frame,
  status,
  children,
}: {
  frame: BarFrame
  status: BarStatus
  children: ReactNode
}) {
  return (
    <>
      <RemoteBar {...frame} status={status} />
      <div className="flex min-h-0 flex-1 overflow-y-auto bg-muted/40 px-4 py-8 sm:pb-[12vh] dark:bg-background">
        <div className="m-auto w-full max-w-md">{children}</div>
      </div>
    </>
  )
}
