// Windows a test puts on a device's X display. Started after the session is up, so they
// are on top of whatever the client itself has put on screen (its connection window).
import { execFileSync, spawn } from 'node:child_process'
import type { ChildProcess } from 'node:child_process'

import { tool } from './env.ts'
import type { DeviceInfo } from './env.ts'

export interface XWindow {
  stop: () => void
}

function run(device: DeviceInfo, command: string, args: string[]): XWindow {
  const child: ChildProcess = spawn(command, args, {
    cwd: device.scratchDir,
    env: { PATH: process.env.PATH ?? '', DISPLAY: device.display, SHELL: '/bin/sh' },
    stdio: 'ignore',
  })
  child.on('error', (error) => {
    process.stderr.write(`${command}: ${error.message}\n`)
  })
  return {
    stop: () => {
      child.kill('SIGKILL')
    },
  }
}

/** A black xterm (at `at`, default the top left) running `sh`, in the run's temp dir. */
export function xterm(device: DeviceInfo, at = '+0+0'): XWindow {
  return run(device, tool('xterm'), [
    '-geometry',
    `80x24${at}`,
    '-bg',
    'black',
    '-fg',
    'white',
    '-e',
    '/bin/sh',
  ])
}

// Class RdNoise keeps the probe's `--class xterm` search from matching this window.
export function noise(device: DeviceInfo, at: string, cols: number, rows: number): XWindow {
  return run(device, tool('xterm'), [
    '-geometry',
    `${String(cols)}x${String(rows)}${at}`,
    '-name',
    'rd-noise',
    '-class',
    'RdNoise',
    '-bg',
    'black',
    '-fg',
    'white',
    '-e',
    'od',
    '-An',
    '-tx1',
    '/dev/urandom',
  ])
}

/** A clock that redraws every second, at the bottom left. */
export function xclock(device: DeviceInfo): XWindow {
  return run(device, tool('xclock'), [
    '-digital',
    '-update',
    '1',
    '-geometry',
    '600x200+0+500',
    '-fa',
    'monospace',
    '-fs',
    '48',
  ])
}

/**
 * Gives the keyboard focus to the newest window of `windowClass`. There is no window manager
 * on the display, so nothing moves the focus when a window is clicked, and the client's
 * connection window (1.5) keeps it.
 */
export function focus(device: DeviceInfo, windowClass: string) {
  execFileSync(
    tool('xdotool'),
    ['search', '--onlyvisible', '--class', windowClass, 'windowfocus'],
    {
      env: { PATH: process.env.PATH ?? '', DISPLAY: device.display },
      timeout: 10_000,
    },
  )
}

function xrandr(device: DeviceInfo, args: string[]): string {
  return execFileSync(tool('xrandr'), args, {
    env: { PATH: process.env.PATH ?? '', DISPLAY: device.display },
    encoding: 'utf8',
    timeout: 10_000,
  })
}

function outputOf(device: DeviceInfo): string {
  const line = xrandr(device, ['-q'])
    .split('\n')
    .find((l) => l.includes(' connected'))
  const output = line?.split(' ')[0]
  if (output === undefined) throw new Error('the display has no output')
  return output
}

/**
 * Xvfb has one mode, so its client lists one resolution. This gives the output another one,
 * smaller than the screen. The client finds `xrandr` through its PATH (see stack.ts).
 */
export function addMode(device: DeviceInfo, width: number, height: number): void {
  const name = `${width}x${height}`
  const output = outputOf(device)
  if (xrandr(device, ['-q']).includes(` ${name} `)) return
  const timing = [
    width,
    width + 8,
    width + 16,
    width + 24,
    height,
    height + 2,
    height + 4,
    height + 6,
  ]
  xrandr(device, ['--newmode', name, '60.00', ...timing.map(String)])
  xrandr(device, ['--output', output, '--addmode', output, name])
}

export function setMode(device: DeviceInfo, width: number, height: number): void {
  xrandr(device, ['--output', outputOf(device), '--mode', `${width}x${height}`])
}
