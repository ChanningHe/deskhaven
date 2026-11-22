// Entry point of the browser decode check (web/e2e/codec/): bundled on its own and loaded into a
// real browser page. The app never imports it.
import { isCodecSupported } from './codec'
import { createWebCodecsDecoder } from './decoder'
import { fixtureKeyFrame } from './fixture-frame'
import { VideoPipeline } from './pipeline'

export interface DecodeCheckResult {
  supported: boolean
  codedWidth: number
  displayWidth: number
  displayHeight: number
  visibleWidth: number
  acks: number
  keyRequests: number
}

const DECODE_TIMEOUT_MS = 10_000

/** Pushes the captured VP9 key frame through the pipeline on WebCodecs and reports the frame. */
export async function decodeCapturedKeyFrame(): Promise<DecodeCheckResult> {
  const supported = await isCodecSupported('vp09.00.10.08')
  const counts = { acks: 0, keyRequests: 0 }
  const frame = await new Promise<VideoFrame>((resolve, reject) => {
    const timer = setTimeout(() => {
      reject(new Error('no frame decoded'))
    }, DECODE_TIMEOUT_MS)
    const pipeline = new VideoPipeline({
      createDecoder: createWebCodecsDecoder,
      onFrame: (decoded) => {
        clearTimeout(timer)
        pipeline.close()
        resolve(decoded)
      },
      ack: () => counts.acks++,
      requestKeyFrame: () => counts.keyRequests++,
      onFatal: (error) => {
        clearTimeout(timer)
        reject(error)
      },
    })
    pipeline.push(fixtureKeyFrame())
  })
  const result = {
    supported,
    codedWidth: frame.codedWidth,
    displayWidth: frame.displayWidth,
    displayHeight: frame.displayHeight,
    visibleWidth: frame.visibleRect?.width ?? 0,
    ...counts,
  }
  frame.close()
  return result
}
