import { execFileSync } from 'node:child_process'

import type { Page } from '@playwright/test'

import { device, expectDesktop, openFromDeviceList, submitPassword } from './support/console.ts'
import { tool } from './support/env.ts'
import type { ClientKey, DeviceInfo } from './support/env.ts'
import { test as base, expect } from './support/fixtures.ts'
import { deviceLog, qosTargets } from './support/peer-log.ts'
import { startProxy } from './support/proxy.ts'
import type { Shaping } from './support/proxy.ts'
import { flicker, noise, xterm } from './support/x11.ts'
import type { XWindow } from './support/x11.ts'

const SAMPLE_MS = Number(process.env.RD_E2E_PERF_SECONDS ?? '180') * 1000
const PROBE_EVERY_MS = 7_000
const PROBE_TIMEOUT_MS = 20_000
const MINUTE_MS = 60_000
const PROBE_AT = '+700+100'
const PROBE = { x: 0.62, y: 0.25 }
const SCREENS = {
  small: { cols: 40, rows: 10 },
  large: { cols: 100, rows: 58 },
} as const

type ConditionName = 'A' | 'B' | 'C' | 'Cb' | 'Cs'
interface Condition {
  shaping: Shaping
  screen: keyof typeof SCREENS
  quality?: 'best'
}
// A and B change a small area, so their fps is the pipeline's rather than the bandwidth's.
// Cb is C at Best quality, whose bitrate fills the link; Cs is a link below the default bitrate.
const CONDITIONS: Record<ConditionName, Condition> = {
  A: { shaping: {}, screen: 'small' },
  B: { shaping: { rttMs: 80 }, screen: 'small' },
  C: { shaping: { rttMs: 80, kbps: 2000 }, screen: 'large' },
  Cb: { shaping: { rttMs: 80, kbps: 2000 }, screen: 'large', quality: 'best' },
  Cs: { shaping: { rttMs: 80, kbps: 500 }, screen: 'large' },
}
const SETTINGS_KEY = 'deskhaven.remote.settings'
type Transport = 'Relay' | 'WebRTC'
const CASES: [ConditionName, ClientKey, Transport][] = [
  ['A', '1.4', 'Relay'],
  ['A', '1.5', 'Relay'],
  ['A', '1.5', 'WebRTC'],
  ['B', '1.4', 'Relay'],
  ['B', '1.5', 'Relay'],
  ['C', '1.4', 'Relay'],
  ['C', '1.5', 'Relay'],
  ['Cb', '1.4', 'Relay'],
  ['Cb', '1.5', 'Relay'],
  ['Cs', '1.5', 'Relay'],
]

const DEFAULT_QUALITY = 'balanced'
const STATS = [
  'fps',
  'kbps',
  'delayMs',
  'decodeMs',
  'decodeQueue',
  'keyRequests',
  'pointerMoves',
  'sentFps',
  'decodeFps',
]
type Sample = Record<string, number | null>
interface QualityTrace {
  activeAt: number | null
  changes: { atMs: number; value: string }[]
}

const test = base.extend<{ shaping: Shaping }>({
  shaping: [{}, { option: true }],
  baseURL: async ({ baseURL, shaping }, provide) => {
    if (baseURL === undefined) throw new Error('no base URL')
    if (shaping.rttMs === undefined && shaping.kbps === undefined) {
      await provide(baseURL)
      return
    }
    const url = new URL(baseURL)
    const stack = { host: url.hostname, port: Number(url.port) }
    const proxy = await startProxy(stack, stack, shaping)
    await provide(proxy.origin)
    await proxy.stop()
  },
})

function xdo(target: DeviceInfo, ...args: string[]) {
  execFileSync(tool('xdotool'), args, {
    env: { PATH: process.env.PATH ?? '', DISPLAY: target.display },
    timeout: 10_000,
  })
}

function pixelIs(page: Page, dark: boolean, timeoutMs: number): Promise<number> {
  return page.evaluate(
    ({ probe, wantDark, limit }) =>
      new Promise<number>((resolve, reject) => {
        const canvas = document.querySelector<HTMLCanvasElement>('canvas[aria-hidden]')
        const context = canvas?.getContext('2d')
        if (!canvas || !context) {
          reject(new Error('no video canvas'))
          return
        }
        const started = Date.now()
        const look = () => {
          const px = context.getImageData(
            Math.floor(canvas.width * probe.x),
            Math.floor(canvas.height * probe.y),
            1,
            1,
          ).data
          const isDark = (px[0] ?? 255) < 64 && (px[3] ?? 0) > 0
          if (isDark === wantDark) resolve(Date.now())
          else if (Date.now() - started > limit) reject(new Error('probe pixel never changed'))
          else requestAnimationFrame(look)
        }
        look()
      }),
    { probe: PROBE, wantDark: dark, limit: timeoutMs },
  )
}

async function latency(page: Page, target: DeviceInfo): Promise<number | null> {
  try {
    xdo(target, 'search', '--class', 'xterm', 'windowunmap')
    await pixelIs(page, false, PROBE_TIMEOUT_MS)
    const seen = pixelIs(page, true, PROBE_TIMEOUT_MS)
    const sent = Date.now()
    xdo(target, 'search', '--class', 'xterm', 'windowmap')
    return (await seen) - sent
  } catch {
    return null
  }
}

async function startSampling(page: Page) {
  await page.evaluate((stats) => {
    const samples: Record<string, number | null>[] = []
    Reflect.set(window, '__perfSamples', samples)
    setInterval(() => {
      const sample: Record<string, number | null> = { atMs: Date.now() }
      for (const stat of stats) {
        const raw = document.querySelector(`[data-stat="${stat}"]`)?.getAttribute('data-value')
        sample[stat] = raw === undefined || raw === null || raw === '' ? null : Number(raw)
      }
      samples.push(sample)
    }, 1000)
  }, STATS)
}

function traceQuality(page: Page) {
  return page.addInitScript(() => {
    const trace: QualityTrace = { activeAt: null, changes: [] }
    Reflect.set(window, '__qualityTrace', trace)
    setInterval(() => {
      const pills = [...document.querySelectorAll('header span.rounded-full')]
      if (trace.activeAt === null && pills.some((p) => p.textContent.includes('Connected'))) {
        trace.activeAt = Date.now()
      }
      const value = document.querySelector('[data-stat="quality"]')?.getAttribute('data-value')
      if (value && trace.changes.at(-1)?.value !== value) {
        trace.changes.push({ atMs: Date.now(), value })
      }
    }, 100)
  })
}

async function qualityReport(page: Page, want: string) {
  const trace = await page.evaluate(() => Reflect.get(window, '__qualityTrace') as QualityTrace)
  const since = trace.activeAt ?? 0
  const reached = trace.changes.find((c) => c.value === want)
  return {
    want,
    reachedMs: reached && trace.activeAt !== null ? reached.atMs - since : null,
    changes: trace.changes.map((c) => ({ atMs: c.atMs - since, value: c.value })),
  }
}

function samplesOf(page: Page): Promise<Sample[]> {
  return page.evaluate(
    () => Reflect.get(window, '__perfSamples') as Record<string, number | null>[],
  )
}

function percentile(values: number[], p: number): number | null {
  if (values.length === 0) return null
  const sorted = [...values].sort((a, b) => a - b)
  const rank = Math.max(0, Math.ceil((p / 100) * sorted.length) - 1)
  return sorted[rank] ?? null
}

function numbers(values: (number | null | undefined)[]): number[] {
  return values.filter((v): v is number => typeof v === 'number' && Number.isFinite(v))
}

for (const [name, key, transport] of CASES) {
  const condition = CONDITIONS[name]
  test.describe(() => {
    test.use({ shaping: condition.shaping })
    test(`@slow perf ${name}: ${key} over ${transport}`, async ({ page }) => {
      test.setTimeout(SAMPLE_MS + 3 * MINUTE_MS)
      const target = device(key)
      test.skip(target === undefined, `no ${key} client in this run`)
      if (target === undefined) return
      if (condition.quality) {
        await page.addInitScript(
          ([key, quality]) => {
            localStorage.setItem(key, JSON.stringify({ v: 1, settings: { quality } }))
          },
          [SETTINGS_KEY, condition.quality] as const,
        )
      }
      if (transport === 'Relay') {
        await page.addInitScript(() => {
          Reflect.deleteProperty(window, 'RTCPeerConnection')
        })
      }

      await traceQuality(page)

      await openFromDeviceList(page, target.id)
      await submitPassword(page)
      const connected = page.locator('header span.rounded-full', { hasText: 'Connected' })
      await expect(connected).toContainText(transport)
      await page.getByRole('button', { name: 'Display' }).click()
      await page.getByRole('menuitemcheckbox', { name: 'Statistics' }).click()
      await expectDesktop(page, transport)

      const size = SCREENS[condition.screen]
      const windows: XWindow[] = [
        noise(target, '+0+0', size.cols, size.rows),
        xterm(target, PROBE_AT),
      ]
      try {
        await pixelIs(page, true, 30_000)
        await startSampling(page)
        const probes: { atMs: number; ms: number | null }[] = []
        const begin = Date.now()
        while (Date.now() - begin < SAMPLE_MS) {
          const atMs = Date.now() - begin
          probes.push({ atMs, ms: await latency(page, target) })
          await expect(connected).toContainText(transport)
          const wait = atMs + PROBE_EVERY_MS - (Date.now() - begin)
          if (wait > 0) await page.waitForTimeout(wait)
        }
        await expect(connected).toContainText(transport)
        const samples = await samplesOf(page)

        const column = (stat: string) => numbers(samples.map((s) => s[stat]))
        const fps = column('fps')
        const measured = numbers(probes.map((p) => p.ms))
        const within = (from: number, to: number) =>
          numbers(probes.filter((p) => p.atMs >= from && p.atMs < to).map((p) => p.ms))
        const report = {
          condition: name,
          key,
          transport,
          shaping: condition.shaping,
          screen: condition.screen,
          seconds: SAMPLE_MS / 1000,
          fps: { p50: percentile(fps, 50), p05: percentile(fps, 5), n: fps.length },
          latencyMs: {
            p50: percentile(measured, 50),
            p95: percentile(measured, 95),
            firstMinuteP95: percentile(within(0, MINUTE_MS), 95),
            lastMinuteP95: percentile(within(SAMPLE_MS - MINUTE_MS, SAMPLE_MS), 95),
            n: measured.length,
            missed: probes.length - measured.length,
          },
          keyRequests: column('keyRequests').at(-1) ?? null,
          kbpsP50: percentile(column('kbps'), 50),
          decodeMsP50: percentile(column('decodeMs'), 50),
          delayMsP50: percentile(column('delayMs'), 50),
          pointerMovesP50: percentile(column('pointerMoves'), 50),
          quality: await qualityReport(page, condition.quality ?? DEFAULT_QUALITY),
          probes,
          samples,
        }
        process.stdout.write(`PERF ${JSON.stringify(report)}\n`)
        await test
          .info()
          .attach('perf.json', { body: JSON.stringify(report), contentType: 'application/json' })
        expect(fps.length).toBeGreaterThan(0)
        expect(measured.length).toBeGreaterThan(0)
      } finally {
        for (const win of windows) win.stop()
      }
    })
  })
}

// F60: the user asks for 60 fps and the device always has a new, cheap frame. Shows whether the
// requested fps reaches the device and whether what arrives keeps up with the device's target.
const F60 = {
  userFps: 60,
  quality: 'balanced',
  warmUpMs: 30_000,
  sampleMs: 60_000,
  sentWithinMs: 20_000,
  keepsUpWith: 0.9,
  screen: { cols: 60, rows: 20 },
} as const

for (const transport of ['Relay', 'WebRTC'] as const) {
  test(`@slow perf F60: 1.5 over ${transport}`, async ({ page }) => {
    test.setTimeout(F60.warmUpMs + F60.sampleMs + 2 * MINUTE_MS)
    const target = device('1.5')
    test.skip(target === undefined, 'no 1.5 client in this run')
    if (target === undefined) return
    await page.addInitScript(
      ([key, settings]) => {
        localStorage.setItem(key, JSON.stringify({ v: 1, settings }))
      },
      [SETTINGS_KEY, { quality: F60.quality, fps: F60.userFps }] as const,
    )
    if (transport === 'Relay') {
      await page.addInitScript(() => {
        Reflect.deleteProperty(window, 'RTCPeerConnection')
      })
    }
    await traceQuality(page)

    await openFromDeviceList(page, target.id)
    await submitPassword(page)
    const connected = page.locator('header span.rounded-full', { hasText: 'Connected' })
    await expect(connected).toContainText(transport)
    await page.getByRole('button', { name: 'Display' }).click()
    await page.getByRole('menuitemcheckbox', { name: 'Statistics' }).click()
    await expectDesktop(page, transport)

    const colors = flicker(target, '+0+0', F60.screen.cols, F60.screen.rows)
    try {
      await startSampling(page)
      await page.waitForTimeout(F60.warmUpMs + F60.sampleMs)
      await expect(connected).toContainText(transport)
      const samples = await samplesOf(page)
      const trace = await page.evaluate(() => Reflect.get(window, '__qualityTrace') as QualityTrace)
      const activeAt = trace.activeAt ?? samples[0]?.atMs ?? 0
      const from = (samples[0]?.atMs ?? activeAt) + F60.warmUpMs
      const kept = samples.filter((s) => (s.atMs ?? 0) >= from)
      const column = (stat: string) => numbers(kept.map((s) => s[stat]))
      const fps = column('fps')
      const reached = samples.find((s) => s.sentFps === F60.userFps)
      const sentAtMs = reached ? (reached.atMs ?? activeAt) - activeAt : null
      const sentHeld =
        reached !== undefined &&
        samples.slice(samples.indexOf(reached)).every((s) => s.sentFps === F60.userFps)
      const log = deviceLog('1.5', target)
      const qos = log === null ? [] : qosTargets(log).filter((q) => q.atMs >= activeAt)
      const targets = qos.filter((q) => q.atMs >= from)
      const peerTarget = percentile(
        targets.map((q) => q.fps),
        50,
      )
      const report = {
        condition: 'F60',
        key: '1.5',
        transport,
        warmUpSeconds: F60.warmUpMs / 1000,
        seconds: F60.sampleMs / 1000,
        fps: { p50: percentile(fps, 50), p05: percentile(fps, 5), n: fps.length },
        sentFps: { reachedMs: sentAtMs, held: sentHeld, last: samples.at(-1)?.sentFps ?? null },
        decodeFpsP50: percentile(column('decodeFps'), 50),
        decodeMsP50: percentile(column('decodeMs'), 50),
        kbpsP50: percentile(column('kbps'), 50),
        peerTargetFps: {
          p50: peerTarget,
          min: targets.length === 0 ? null : Math.min(...targets.map((q) => q.fps)),
          n: targets.length,
          logged: log !== null,
        },
        samples,
        peerTrace: qos.map((q) => ({ ...q, atMs: q.atMs - activeAt })),
      }
      process.stdout.write(`PERF ${JSON.stringify(report)}\n`)
      await test
        .info()
        .attach('perf.json', { body: JSON.stringify(report), contentType: 'application/json' })
      expect(fps.length).toBeGreaterThan(0)
      expect(sentAtMs).not.toBeNull()
      expect(sentAtMs ?? Infinity).toBeLessThanOrEqual(F60.sentWithinMs)
      expect(sentHeld).toBe(true)
      if (peerTarget !== null) {
        expect(percentile(fps, 50) ?? 0).toBeGreaterThanOrEqual(F60.keepsUpWith * peerTarget)
      }
    } finally {
      colors.stop()
    }
  })
}

// Q500: custom quality 500 over an unshaped link with the whole screen changing, so the bitrate
// is the quality's. 4300 kbps is twice the old ceiling here, custom 100 at 1280×800 (2148 kbps).
const Q500 = {
  quality: { custom: 500 },
  warmUpMs: 30_000,
  sampleMs: 60_000,
  minKbps: 4300,
  screen: { cols: 214, rows: 62 },
} as const

for (const transport of ['Relay', 'WebRTC'] as const) {
  test(`@slow perf Q500: 1.5 over ${transport}`, async ({ page }) => {
    test.setTimeout(Q500.warmUpMs + Q500.sampleMs + 2 * MINUTE_MS)
    const target = device('1.5')
    test.skip(target === undefined, 'no 1.5 client in this run')
    if (target === undefined) return
    await page.addInitScript(
      ([key, settings]) => {
        localStorage.setItem(key, JSON.stringify({ v: 1, settings }))
      },
      [SETTINGS_KEY, { quality: Q500.quality }] as const,
    )
    if (transport === 'Relay') {
      await page.addInitScript(() => {
        Reflect.deleteProperty(window, 'RTCPeerConnection')
      })
    }
    await traceQuality(page)

    await openFromDeviceList(page, target.id)
    await submitPassword(page)
    const connected = page.locator('header span.rounded-full', { hasText: 'Connected' })
    await expect(connected).toContainText(transport)
    await page.getByRole('button', { name: 'Display' }).click()
    await page.getByRole('menuitemcheckbox', { name: 'Statistics' }).click()
    await expectDesktop(page, transport)

    const screen = noise(target, '+0+0', Q500.screen.cols, Q500.screen.rows)
    try {
      await startSampling(page)
      await page.waitForTimeout(Q500.warmUpMs + Q500.sampleMs)
      await expect(connected).toContainText(transport)
      const samples = await samplesOf(page)
      const from = (samples[0]?.atMs ?? 0) + Q500.warmUpMs
      const kept = samples.filter((s) => (s.atMs ?? 0) >= from)
      const column = (stat: string) => numbers(kept.map((s) => s[stat]))
      const kbps = percentile(column('kbps'), 50)
      const report = {
        condition: 'Q500',
        key: '1.5',
        transport,
        warmUpSeconds: Q500.warmUpMs / 1000,
        seconds: Q500.sampleMs / 1000,
        kbpsP50: kbps,
        kbpsP05: percentile(column('kbps'), 5),
        fps: { p50: percentile(column('fps'), 50), p05: percentile(column('fps'), 5) },
        delayMsP50: percentile(column('delayMs'), 50),
        decodeMsP50: percentile(column('decodeMs'), 50),
        quality: await qualityReport(page, String(Q500.quality.custom)),
        samples,
      }
      process.stdout.write(`PERF ${JSON.stringify(report)}\n`)
      await test
        .info()
        .attach('perf.json', { body: JSON.stringify(report), contentType: 'application/json' })
      expect(kept.length).toBeGreaterThan(0)
      expect(kbps ?? 0).toBeGreaterThanOrEqual(Q500.minKbps)
    } finally {
      screen.stop()
    }
  })
}
