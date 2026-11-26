import { ErrorAlert } from '@/components/common/error-alert'

import { isInsecureTransportError } from '../transport'
import { InsecureTransportNotice } from './insecure-transport-notice'

export function AuthError({ error }: { error: unknown }) {
  if (error === null || error === undefined) return null
  if (isInsecureTransportError(error)) return <InsecureTransportNotice />
  return <ErrorAlert error={error} />
}
