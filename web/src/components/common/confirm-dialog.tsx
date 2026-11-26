import { useId, useRef, useState } from 'react'
import type { ReactNode, SubmitEvent } from 'react'
import { useTranslation } from 'react-i18next'

import { ErrorAlert } from '@/components/common/error-alert'
import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'

export interface ConfirmDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  title: string
  description?: ReactNode
  confirmLabel: string
  confirmText?: string
  destructive?: boolean
  onConfirm: () => unknown
}

type Body = Omit<ConfirmDialogProps, 'open' | 'onOpenChange' | 'title' | 'description'> & {
  onDone: () => void
  onPendingChange: (pending: boolean) => void
}

// Rendered inside the popup, which unmounts on close: every opening starts empty.
function ConfirmBody({
  confirmLabel,
  confirmText,
  destructive = true,
  onConfirm,
  onDone,
  onPendingChange,
}: Body) {
  const { t } = useTranslation()
  const inputId = useId()
  const [typed, setTyped] = useState('')
  const [pending, setPending] = useState(false)
  const [error, setError] = useState<unknown>(undefined)
  const matches = confirmText === undefined || typed === confirmText

  const submit = async (event: SubmitEvent<HTMLFormElement>) => {
    event.preventDefault()
    // React bubbles synthetic events through portals: never let this submit reach a form
    // the dialog happens to be rendered in.
    event.stopPropagation()
    if (!matches || pending) return
    setPending(true)
    onPendingChange(true)
    setError(undefined)
    try {
      await onConfirm()
      onDone()
    } catch (caught) {
      setError(caught)
    } finally {
      setPending(false)
      onPendingChange(false)
    }
  }

  return (
    <form className="grid gap-4" onSubmit={(event) => void submit(event)}>
      {confirmText === undefined ? null : (
        <div className="grid gap-2">
          <Label htmlFor={inputId} className="font-normal">
            {t('confirm.typeToConfirm', { name: confirmText })}
          </Label>
          <Input
            id={inputId}
            value={typed}
            autoComplete="off"
            spellCheck={false}
            disabled={pending}
            onChange={(event) => {
              setTyped(event.target.value)
            }}
          />
        </div>
      )}
      {error === undefined ? null : <ErrorAlert error={error} />}
      <AlertDialogFooter>
        <AlertDialogCancel disabled={pending}>{t('common.cancel')}</AlertDialogCancel>
        <Button
          type="submit"
          variant={destructive ? 'destructive' : 'default'}
          disabled={!matches || pending}
          aria-busy={pending}
        >
          {confirmLabel}
        </Button>
      </AlertDialogFooter>
    </form>
  )
}

/** Focus starts on the name field or Cancel, never on the destructive button. */
export function ConfirmDialog({
  open,
  onOpenChange,
  title,
  description,
  ...body
}: ConfirmDialogProps) {
  const pendingRef = useRef(false)
  return (
    <AlertDialog
      open={open}
      onOpenChange={(next) => {
        // A running action cannot be dismissed; its outcome decides.
        if (!next && pendingRef.current) return
        onOpenChange(next)
      }}
    >
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{title}</AlertDialogTitle>
          {description === undefined ? null : (
            <AlertDialogDescription>{description}</AlertDialogDescription>
          )}
        </AlertDialogHeader>
        <ConfirmBody
          {...body}
          onDone={() => {
            onOpenChange(false)
          }}
          onPendingChange={(pending) => {
            pendingRef.current = pending
          }}
        />
      </AlertDialogContent>
    </AlertDialog>
  )
}
