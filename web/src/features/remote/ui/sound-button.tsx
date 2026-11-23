import { Volume2, VolumeX } from 'lucide-react'
import { useEffect } from 'react'
import { useTranslation } from 'react-i18next'

import { soundSwitch, type Speaker } from '../audio/speaker'
import type { Session } from '../session/session'
import { IconButton } from './toolbar-controls'
import { useStore } from './use-remote'

export function SoundButton({
  session,
  speaker,
  enabled,
  onEnabled,
}: {
  session: Session
  speaker: Speaker
  enabled: boolean
  onEnabled: (on: boolean) => void
}) {
  const { t } = useTranslation()
  const view = useStore(speaker.view)
  const { audio: permitted } = useStore(session.permissions)
  const audio = useStore(session.audio.state)
  const { output } = view

  useEffect(() => {
    session.audio.play(permitted ? output : null)
    return () => {
      session.audio.play(null)
    }
  }, [session, output, permitted])

  const state = soundSwitch(view, { enabled, permitted, audio })
  const disabled = state === 'probing' || state === 'unsupported' || state === 'denied'
  const onClick = () => {
    if (state === 'off' || state === 'suspended') speaker.start()
    if (state === 'off') onEnabled(true)
    if (state === 'on' || state === 'failed') onEnabled(false)
  }
  return (
    <IconButton
      label={t(`remote.sound.${state}`)}
      hint={t(`remote.sound.${state}Hint`)}
      disabled={disabled}
      focusableWhenDisabled
      className="aria-disabled:opacity-50"
      onClick={onClick}
    >
      {state === 'on' ? <Volume2 aria-hidden /> : <VolumeX aria-hidden />}
    </IconButton>
  )
}
