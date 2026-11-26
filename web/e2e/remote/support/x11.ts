// Started after the session is up, so they sit on top of the client's own windows.
import { execFileSync, spawn, spawnSync } from 'node:child_process'
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

// Plain xterm pastes PRIMARY on Shift+Insert and nothing on Ctrl+V; the client writes CLIPBOARD.
const CLIPBOARD_PASTE = [
  'XTerm*VT100.translations: #override \\n',
  'Ctrl <Key>V: insert-selection(CLIPBOARD) \\n',
  'Ctrl Shift <Key>V: insert-selection(CLIPBOARD) \\n',
  'Shift <Key>Insert: insert-selection(CLIPBOARD)',
].join('')

export function xterm(device: DeviceInfo, at = '+0+0', pasteClipboard = false): XWindow {
  return run(device, tool('xterm'), [
    ...(pasteClipboard ? ['-xrm', CLIPBOARD_PASTE] : []),
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

const KEY_NAMES = [
  'XTerm*VT100.translations: #override \\n',
  'Mod1 <Key>Tab: string("alt-tab\\n") \\n',
  '<Key>Print: string("print-screen\\n") \\n',
  '<Key>Super_L: string("win\\n")',
].join('')

export function keyProbe(device: DeviceInfo, file: string): XWindow {
  return run(device, tool('xterm'), [
    '-xrm',
    KEY_NAMES,
    '-geometry',
    '80x24+0+0',
    '-bg',
    'black',
    '-fg',
    'white',
    '-e',
    '/bin/sh',
    '-c',
    `cat > ${file}`,
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

// Well over 60 changes a second (a capture equal to the last one sends nothing) and cheap to
// encode, so the bitrate never caps the frame rate. With `sleep 0.008` a 60 fps target got ~52.
const FLICKER =
  'i=0; while :; do printf "\\033[4%dm\\033[2J" $((i % 6 + 1)); i=$((i + 1)); sleep 0.004; done'

export function flicker(device: DeviceInfo, at: string, cols: number, rows: number): XWindow {
  return run(device, tool('xterm'), [
    '-geometry',
    `${String(cols)}x${String(rows)}${at}`,
    '-name',
    'rd-flicker',
    '-class',
    'RdFlicker',
    '-e',
    '/bin/sh',
    '-c',
    FLICKER,
  ])
}

export function xclock(device: DeviceInfo): XWindow {
  return run(device, tool('xclock'), [
    '-digital',
    '-update',
    '1',
    '-strftime',
    '%H:%M:%S',
    '-geometry',
    '600x200+0+500',
    '-face',
    'monospace:size=48',
  ])
}

/** No window manager: a click moves no focus, and the client's connection window keeps it. */
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

export function movePointer(device: DeviceInfo, x: number, y: number) {
  execFileSync(tool('xdotool'), ['mousemove', String(x), String(y)], {
    env: { PATH: process.env.PATH ?? '', DISPLAY: device.display },
    timeout: 10_000,
  })
}

/** Where the device's X pointer is: only input the device received moves it. */
export function pointerAt(device: DeviceInfo): { x: number; y: number } {
  const out = execFileSync(tool('xdotool'), ['getmouselocation', '--shell'], {
    env: { PATH: process.env.PATH ?? '', DISPLAY: device.display },
    encoding: 'utf8',
    timeout: 10_000,
  })
  const read = (name: string) => Number(new RegExp(`^${name}=(\\d+)$`, 'm').exec(out)?.[1])
  return { x: read('X'), y: read('Y') }
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

/** Xvfb has one mode, so the client would list one resolution. */
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

const CLIPBOARD = ['-selection', 'clipboard']

export function setClipboard(device: DeviceInfo, text: string): void {
  execFileSync(tool('xclip'), [...CLIPBOARD, '-in'], {
    env: { PATH: process.env.PATH ?? '', DISPLAY: device.display },
    input: text,
    stdio: ['pipe', 'ignore', 'ignore'],
    timeout: 10_000,
  })
}

export function clipboardOf(device: DeviceInfo): string | null {
  const read = spawnSync(tool('xclip'), [...CLIPBOARD, '-out'], {
    env: { PATH: process.env.PATH ?? '', DISPLAY: device.display },
    encoding: 'utf8',
    timeout: 10_000,
  })
  if (read.error) throw read.error
  return read.status === 0 ? read.stdout : null
}
