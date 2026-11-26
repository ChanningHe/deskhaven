import { Trans, useTranslation } from 'react-i18next'

import {
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
} from '@/components/ui/dropdown-menu'

import { EXPERIMENTAL_CODECS, VIDEO_CODECS, type VideoCodec } from '../../handshake/login'
import type { Session } from '../../session/session'
import { CODEC_NAMES } from '../../video/codec'
import { useStore } from '../use-remote'

export function CodecSubmenu({
  session,
  codec,
  onChange,
}: {
  session: Session
  codec: VideoCodec
  onChange: (codec: VideoCodec) => void
}) {
  const { t } = useTranslation()
  const { support, receiving } = useStore(session.codecs)
  return (
    <DropdownMenuSub>
      <DropdownMenuSubTrigger>
        <span className="flex flex-1 justify-between gap-3">
          {t('remote.toolbar.codec')}
          <span className="text-muted-foreground">{CODEC_NAMES[codec]}</span>
        </span>
      </DropdownMenuSubTrigger>
      <DropdownMenuSubContent>
        <DropdownMenuRadioGroup
          value={codec}
          onValueChange={(value: unknown) => {
            const picked = VIDEO_CODECS.find((c) => c === value)
            if (picked !== undefined && picked !== codec && support[picked] === 'ok') {
              onChange(picked)
            }
          }}
        >
          {VIDEO_CODECS.map((option) => {
            const blocked = support[option]
            const name = t(`remote.toolbar.codecs.${option}`)
            return (
              <DropdownMenuRadioItem
                key={option}
                value={option}
                disabled={blocked !== 'ok'}
                closeOnClick
              >
                {blocked === 'ok' ? (
                  name
                ) : (
                  <span>
                    <Trans
                      i18nKey="remote.toolbar.codecOption"
                      values={{ name, reason: t(`remote.toolbar.codecBlocked.${blocked}`) }}
                      components={{ reason: <span className="text-muted-foreground" /> }}
                    />
                  </span>
                )}
                {EXPERIMENTAL_CODECS.includes(option) ? (
                  <>
                    {' '}
                    <span className="ml-auto rounded-sm bg-muted px-1.5 py-px text-[10px] leading-4 font-medium text-muted-foreground">
                      {t('remote.toolbar.codecExperimental')}
                    </span>
                  </>
                ) : null}
              </DropdownMenuRadioItem>
            )
          })}
        </DropdownMenuRadioGroup>
        {receiving === null || receiving === codec ? null : (
          <p className="px-1.5 pt-0.5 pb-1 text-xs text-muted-foreground">
            {t('remote.toolbar.codecReceiving', { name: CODEC_NAMES[receiving] })}
          </p>
        )}
      </DropdownMenuSubContent>
    </DropdownMenuSub>
  )
}
