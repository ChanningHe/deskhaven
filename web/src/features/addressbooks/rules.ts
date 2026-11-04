/** Share rules: 1 read-only (and the stored passwords), 2 read-write, 3 full control. */
export const RULES = [1, 2, 3] as const

export type Rule = (typeof RULES)[number]

/** The rule a new grant starts with: the least that is useful. */
export const DEFAULT_RULE: Rule = 1
