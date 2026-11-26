export const PLATFORM_WINDOWS = 'Windows'
export const PLATFORM_LINUX = 'Linux'
export const PLATFORM_MACOS = 'Mac OS'
export const PLATFORM_ANDROID = 'Android'

const VERSION_PART = /^\d+$/

function part(text: string | undefined): number {
  return text !== undefined && VERSION_PART.test(text) ? Number(text) : 0
}

/** hbb_common `get_version_number`: "1.2.3" is 1002030, "1.2.3-4" is 1002034, anything unparsable 0. */
export function versionNumber(version: string): number {
  const [release, build] = version.split('-')
  let n = 0
  let last = 0
  for (const x of (release ?? '').split('.')) {
    last = part(x)
    n = n * 1000 + last
  }
  return n - last + last * 10 + part(build)
}

export function atLeast(version: string, minimum: string): boolean {
  return versionNumber(version) >= versionNumber(minimum)
}
