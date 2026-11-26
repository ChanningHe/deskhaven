import { CircleAlert } from 'lucide-react'
import type { ReactNode } from 'react'

import { Button } from '@/components/ui/button'

import { Step } from './panels'

export function ProblemPanel({
  title,
  message,
  onRetry,
  retryLabel,
  leave,
}: {
  title: string
  message: string
  onRetry: () => void
  retryLabel: string
  leave?: ReactNode
}) {
  return (
    <Step
      title={title}
      footer={
        <>
          <Button onClick={onRetry}>{retryLabel}</Button>
          {leave}
        </>
      }
    >
      <p role="alert" className="flex items-start gap-2 text-sm">
        <CircleAlert aria-hidden className="mt-0.5 size-4 shrink-0 text-destructive" />
        <span className="min-w-0 break-words">{message}</span>
      </p>
    </Step>
  )
}
