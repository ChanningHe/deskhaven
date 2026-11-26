export const HTTP_UNAUTHORIZED = 401

export class HttpError extends Error {
  override readonly name: string = 'HttpError'
  readonly status: number

  constructor(status: number, message = `HTTP ${status}`) {
    super(message)
    this.status = status
  }
}

export function httpStatusOf(error: unknown): number | undefined {
  if (typeof error !== 'object' || error === null || !('status' in error)) return undefined
  return typeof error.status === 'number' ? error.status : undefined
}

export function isClientError(status: number): boolean {
  return status >= 400 && status < 500
}
