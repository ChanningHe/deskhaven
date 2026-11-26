import { useRouter } from '@tanstack/react-router'
import { useEffect, useState } from 'react'
import type { RefObject } from 'react'
import { useTranslation } from 'react-i18next'

interface RouteAnnouncerProps {
  mainRef: RefObject<HTMLElement | null>
}

export function RouteAnnouncer({ mainRef }: RouteAnnouncerProps) {
  const { t } = useTranslation()
  const router = useRouter()
  const [announcement, setAnnouncement] = useState('')

  useEffect(
    () =>
      router.subscribe('onRendered', (event) => {
        const main = mainRef.current
        if (!main) return
        const heading = main.querySelector('h1')?.textContent ?? ''
        const appName = t('app.name')
        document.title = heading ? `${heading} · ${appName}` : appName

        // Initial load: the browser already focuses the document start.
        if (!event.fromLocation || !event.pathChanged) return
        main.focus({ preventScroll: true })
        setAnnouncement(heading)
      }),
    [router, mainRef, t],
  )

  return (
    <div role="status" aria-live="polite" aria-atomic="true" className="sr-only">
      {announcement}
    </div>
  )
}
