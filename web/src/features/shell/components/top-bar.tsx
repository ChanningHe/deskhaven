import { LanguageMenu } from './language-menu'
import { MobileNav } from './mobile-nav'
import { SearchPlaceholder } from './search-placeholder'
import { ThemeMenu } from './theme-menu'
import { UserMenu } from './user-menu'

export function TopBar() {
  return (
    <header className="flex h-14 shrink-0 items-center gap-2 border-b bg-background px-3 sm:px-4">
      <MobileNav />
      <SearchPlaceholder />
      <div className="ml-auto flex items-center gap-1">
        <LanguageMenu />
        <ThemeMenu />
        <UserMenu />
      </div>
    </header>
  )
}
