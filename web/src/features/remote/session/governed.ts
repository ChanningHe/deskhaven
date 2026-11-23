import type { FpsGovernor } from '../video/fps-governor'
import type { VideoMeter } from '../video/pipeline'

export function governed(meter: VideoMeter, governor: FpsGovernor): VideoMeter {
  return {
    now: () => meter.now(),
    received: (bytes) => {
      meter.received(bytes)
    },
    decoded: (latencyMs, serviceMs) => {
      meter.decoded(latencyMs, serviceMs)
      governor.decoded(serviceMs)
    },
  }
}
