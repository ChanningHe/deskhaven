import { Link } from '@tanstack/react-router'
import { ArrowLeft } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import { buttonVariants } from '@/components/ui/button'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'

/** Back to the console; leaving the page ends the session. */
export function BackLink() {
  const { t } = useTranslation()
  const label = t('remote.back')
  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <Link
            to="/"
            aria-label={label}
            className={buttonVariants({ variant: 'ghost', size: 'icon' })}
          />
        }
      >
        <ArrowLeft aria-hidden />
      </TooltipTrigger>
      <TooltipContent side="bottom">{label}</TooltipContent>
    </Tooltip>
  )
}
