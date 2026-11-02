import { Link } from '@tanstack/react-router'
import { Compass } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import { EmptyState } from '@/components/common/empty-state'
import { Button } from '@/components/ui/button'

export function NotFound() {
  const { t } = useTranslation()
  return (
    <main className="mx-auto flex min-h-svh max-w-lg items-center px-4">
      <div className="w-full">
        <EmptyState
          icon={Compass}
          title={t('notFound.title')}
          description={t('notFound.description')}
          action={
            <Button variant="outline" nativeButton={false} render={<Link to="/" />}>
              {t('notFound.back')}
            </Button>
          }
        />
      </div>
    </main>
  )
}
