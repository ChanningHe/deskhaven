// The 2FA enforcement policy: the `auth.tfa.enforce` setting and its choices.

export const TFA_KEY = 'auth.tfa.enforce'
export const TFA_CHOICES = ['off', 'admins', 'all'] as const
export type TfaChoice = (typeof TFA_CHOICES)[number]

export function isTfaChoice(value: unknown): value is TfaChoice {
  return TFA_CHOICES.some((choice) => choice === value)
}
