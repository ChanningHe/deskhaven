import { Check, Copy } from 'lucide-react'
import { Fragment, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { copyToClipboard } from '@/lib/clipboard'

type CopyState = 'idle' | 'copied' | 'failed'

// Long keys and variable names wrap after a `.` or `__` before breaking inside a word.
const SEGMENT_END = /(?<=\.|__)/

export function CopyCode({ value }: { value: string }) {
  const { t } = useTranslation()
  const codeRef = useRef<HTMLElement>(null)
  const [state, setState] = useState<CopyState>('idle')
  const copy = async () => {
    const copied = await copyToClipboard(value)
    setState(copied ? 'copied' : 'failed')
    if (!copied && codeRef.current !== null)
      window.getSelection()?.selectAllChildren(codeRef.current)
  }
  const tip = {
    idle: t('settings.row.copy', { value }),
    copied: t('secret.copied'),
    failed: t('secret.copyFailed'),
  }[state]
  return (
    <span className="min-w-0">
      <code ref={codeRef} className="wrap-anywhere select-all">
        {value.split(SEGMENT_END).map((segment, index) => (
          <Fragment key={index}>
            {index > 0 ? <wbr /> : null}
            {segment}
          </Fragment>
        ))}
      </code>
      <Tooltip>
        <TooltipTrigger
          render={
            <Button
              type="button"
              variant="ghost"
              size="icon-xs"
              aria-label={t('settings.row.copy', { value })}
              className="ml-0.5 size-5 align-middle text-muted-foreground hover:text-foreground"
              onClick={() => void copy()}
            />
          }
        >
          {state === 'copied' ? <Check aria-hidden /> : <Copy aria-hidden />}
        </TooltipTrigger>
        <TooltipContent>{tip}</TooltipContent>
      </Tooltip>
    </span>
  )
}
