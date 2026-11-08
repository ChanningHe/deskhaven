import { Menu } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import { Sheet, SheetContent, SheetHeader, SheetTitle, SheetTrigger } from '@/components/ui/sheet'

import { Brand } from './brand'
import { SidebarNav } from './sidebar-nav'
import { VersionInfo } from './version-info'

/** Navigation drawer for small screens (below md). */
export function MobileNav() {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)

  return (
    <Sheet open={open} onOpenChange={setOpen}>
      <SheetTrigger
        render={
          <Button
            variant="ghost"
            size="icon"
            className="md:hidden"
            aria-label={t('shell.openNavigation')}
          />
        }
      >
        <Menu aria-hidden />
      </SheetTrigger>
      <SheetContent side="left" closeLabel={t('shell.close')} className="w-64 gap-0 p-0">
        <SheetHeader className="h-14 justify-center border-b px-3">
          <SheetTitle render={<div />}>
            <Brand />
          </SheetTitle>
        </SheetHeader>
        <SidebarNav
          collapsed={false}
          onNavigate={() => {
            setOpen(false)
          }}
        />
        <VersionInfo className="border-t px-3 py-3" />
      </SheetContent>
    </Sheet>
  )
}
