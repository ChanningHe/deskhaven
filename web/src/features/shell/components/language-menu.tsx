import { Languages } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import {
  currentLanguage,
  isLanguage,
  LANGUAGE_NAMES,
  setLanguage,
  SUPPORTED_LANGUAGES,
} from '@/i18n'

export function LanguageMenu() {
  // useTranslation re-renders this component whenever the language changes.
  const { t } = useTranslation()
  const current = currentLanguage()

  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        render={<Button variant="ghost" size="icon" aria-label={t('language.label')} />}
      >
        <Languages aria-hidden />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-40">
        <DropdownMenuGroup>
          <DropdownMenuLabel>{t('language.label')}</DropdownMenuLabel>
          <DropdownMenuRadioGroup
            value={current}
            onValueChange={(value: unknown) => {
              if (isLanguage(value)) void setLanguage(value)
            }}
          >
            {SUPPORTED_LANGUAGES.map((language) => (
              <DropdownMenuRadioItem key={language} value={language} lang={language} closeOnClick>
                {LANGUAGE_NAMES[language]}
              </DropdownMenuRadioItem>
            ))}
          </DropdownMenuRadioGroup>
        </DropdownMenuGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
