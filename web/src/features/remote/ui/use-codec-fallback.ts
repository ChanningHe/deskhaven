import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'

import type { VideoCodec } from '../handshake/login'
import type { Session } from '../session/session'
import { CODEC_NAMES } from '../video/codec'

interface Fallback {
  codec: VideoCodec
  why: 'failed' | 'ignored'
  back: VideoCodec
}

export function useCodecFallback(session: Session, showMs: number): string | null {
  const { t } = useTranslation()
  const [fallback, setFallback] = useState<Fallback | null>(null)
  useEffect(() => {
    let last = session.codecs.get().failedAt
    return session.codecs.subscribe(() => {
      const view = session.codecs.get()
      if (view.failedAt === last) return
      last = view.failedAt
      if (view.failed === undefined) return
      const why = view.support[view.failed] === 'ignored' ? 'ignored' : 'failed'
      setFallback({ codec: view.failed, why, back: view.requested })
    })
  }, [session])
  useEffect(() => {
    if (fallback === null) return
    const timer = setTimeout(() => {
      setFallback(null)
    }, showMs)
    return () => {
      clearTimeout(timer)
    }
  }, [fallback, showMs])
  if (fallback === null) return null
  return t(`remote.codecFallback.${fallback.why}`, {
    name: CODEC_NAMES[fallback.codec],
    back: CODEC_NAMES[fallback.back],
  })
}
