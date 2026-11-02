// Error contract between the API layer and the rest of the app. The API
// mutator (generated client, once orval is wired up) throws HttpError for every
// non-2xx response; network failures surface as plain errors (no status).

export const HTTP_UNAUTHORIZED = 401

export class HttpError extends Error {
  override readonly name = 'HttpError'
  readonly status: number

  constructor(status: number, message = `HTTP ${status}`) {
    super(message)
    this.status = status
  }
}

/** HTTP status carried by an error, or undefined for network/unknown errors. */
export function httpStatusOf(error: unknown): number | undefined {
  if (typeof error !== 'object' || error === null || !('status' in error)) return undefined
  return typeof error.status === 'number' ? error.status : undefined
}

export function isClientError(status: number): boolean {
  return status >= 400 && status < 500
}
