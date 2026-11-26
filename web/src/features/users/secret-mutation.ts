/** The mutation cache drops a show-once secret as soon as nothing observes it, not after five minutes. */
export const SECRET_MUTATION = { mutation: { gcTime: 0 } } as const
