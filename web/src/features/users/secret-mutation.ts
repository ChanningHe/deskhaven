/**
 * Options for mutations whose response is a show-once secret (temporary passwords): the
 * mutation cache drops the response as soon as nothing observes it (the dialog showing it
 * was closed or unmounted), instead of keeping the plaintext for the default five minutes.
 */
export const SECRET_MUTATION = { mutation: { gcTime: 0 } } as const
