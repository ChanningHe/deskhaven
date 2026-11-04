// Names for the numeric codes the RustDesk clients send (03 §2.3, §2.6). A code this
// console does not know (a newer client) is shown as the number, never hidden.

import type { TFunction } from 'i18next'

const CONN_TYPE = {
  0: 'audit.connType.remoteControl',
  1: 'audit.connType.fileTransfer',
  2: 'audit.connType.portForward',
  3: 'audit.connType.camera',
  4: 'audit.connType.terminal',
} as const

const PRIMARY_AUTH = {
  1: 'audit.primaryAuth.click',
  2: 'audit.primaryAuth.temporaryPassword',
  3: 'audit.primaryAuth.permanentPassword',
  4: 'audit.primaryAuth.switchSides',
} as const

const TWO_FACTOR = {
  1: 'audit.twoFactor.totp',
  2: 'audit.twoFactor.trustedDevice',
} as const

const ALARM_TYPE = {
  0: 'audit.alarmType.ipAllowList',
  1: 'audit.alarmType.manyAttempts',
  2: 'audit.alarmType.burstAttempts',
  6: 'audit.alarmType.ipv6Prefix',
  7: 'audit.alarmType.terminalLogin',
  8: 'audit.alarmType.terminalLoginFailed',
  9: 'audit.alarmType.scopeViolation',
  10: 'audit.alarmType.idAllowList',
} as const

/** Codes with a name, for the filter dropdowns. */
export const CONN_TYPE_CODES = Object.keys(CONN_TYPE).map(Number)
export const ALARM_TYPE_CODES = Object.keys(ALARM_TYPE).map(Number)

function known<T extends object>(names: T, code: number): code is keyof T & number {
  return Object.hasOwn(names, code)
}

export function connTypeLabel(t: TFunction, code: number): string {
  return known(CONN_TYPE, code) ? t(CONN_TYPE[code]) : t('audit.unknownCode', { code })
}

export function primaryAuthLabel(t: TFunction, code: number): string {
  return known(PRIMARY_AUTH, code) ? t(PRIMARY_AUTH[code]) : t('audit.unknownCode', { code })
}

export function twoFactorLabel(t: TFunction, code: number): string {
  return known(TWO_FACTOR, code) ? t(TWO_FACTOR[code]) : t('audit.unknownCode', { code })
}

export function alarmTypeLabel(t: TFunction, code: number): string {
  return known(ALARM_TYPE, code) ? t(ALARM_TYPE[code]) : t('audit.unknownCode', { code })
}

const DIRECTION = {
  send: 'audit.direction.send',
  receive: 'audit.direction.receive',
} as const

const METHOD = {
  password: 'audit.method.password',
  totp: 'audit.method.totp',
  email_code: 'audit.method.email_code',
  oidc: 'audit.method.oidc',
  ldap: 'audit.method.ldap',
  recovery: 'audit.method.recovery',
} as const

const CLIENT = {
  rustdesk: 'audit.client.rustdesk',
  console: 'audit.client.console',
  api: 'audit.client.api',
} as const

const ACTOR_KIND = {
  user: 'audit.actorKind.user',
  api_token: 'audit.actorKind.api_token',
  cli: 'audit.actorKind.cli',
  system: 'audit.actorKind.system',
} as const

const RESULT = {
  success: 'audit.result.success',
  invalid_credentials: 'audit.result.invalid_credentials',
  disabled: 'audit.result.disabled',
  unverified: 'audit.result.unverified',
  locked: 'audit.result.locked',
  tfa_required: 'audit.result.tfa_required',
  tfa_setup_required: 'audit.result.tfa_setup_required',
  invalid_code: 'audit.result.invalid_code',
  invalid_challenge: 'audit.result.invalid_challenge',
  oidc_denied: 'audit.result.oidc_denied',
  oidc_unavailable: 'audit.result.oidc_unavailable',
  oidc_invalid_token: 'audit.result.oidc_invalid_token',
  no_linked_account: 'audit.result.no_linked_account',
  provisioning_failed: 'audit.result.provisioning_failed',
} as const

function knownName<T extends object>(names: T, name: string): name is keyof T & string {
  return Object.hasOwn(names, name)
}

// The server stores these names as text; one this console does not know is shown as it is.
export const directionLabel = (t: TFunction, name: string): string =>
  knownName(DIRECTION, name) ? t(DIRECTION[name]) : name
export const methodLabel = (t: TFunction, name: string): string =>
  knownName(METHOD, name) ? t(METHOD[name]) : name
export const clientLabel = (t: TFunction, name: string): string =>
  knownName(CLIENT, name) ? t(CLIENT[name]) : name
export const actorKindLabel = (t: TFunction, name: string): string =>
  knownName(ACTOR_KIND, name) ? t(ACTOR_KIND[name]) : name
export const resultLabel = (t: TFunction, name: string): string =>
  knownName(RESULT, name) ? t(RESULT[name]) : name
