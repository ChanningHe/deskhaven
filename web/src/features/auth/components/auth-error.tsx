import { ErrorAlert } from '@/components/common/error-alert'

import { isInsecureTransportError } from '../transport'
import { InsecureTransportNotice } from './insecure-transport-notice'

/** A sign-in error: the plain-HTTP explanation, or the translated error. */
export function AuthError({ error }: { error: unknown }) {
  if (error === null || error === undefined) return null
  if (isInsecureTransportError(error)) return <InsecureTransportNotice />
  return <ErrorAlert error={error} />
}
