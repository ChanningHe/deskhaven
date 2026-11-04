import { describe, expect, it } from 'vitest'

import { endField, startField, windowEnd, windowStart } from './audit-range'

describe('audit time window', () => {
  it('runs from the start of the first day to the start of the day after the last', () => {
    const from = windowStart('2026-03-10')
    const to = windowEnd('2026-03-10')

    expect(from).toBe(new Date(2026, 2, 10).getTime())
    expect(to).toBe(new Date(2026, 2, 11).getTime())
  })

  it('rolls the end over month and year boundaries', () => {
    expect(windowEnd('2026-12-31')).toBe(new Date(2027, 0, 1).getTime())
    expect(windowEnd('2026-02-28')).toBe(new Date(2026, 2, 1).getTime())
  })

  it('shows the window back as the same dates', () => {
    const from = windowStart('2026-03-10')
    const to = windowEnd('2026-03-12')

    expect(startField(from)).toBe('2026-03-10')
    expect(endField(to)).toBe('2026-03-12')
    expect(startField(undefined)).toBe('')
    expect(endField(undefined)).toBe('')
  })

  it('rejects what is not a real date', () => {
    for (const bad of ['', '2026-3-1', '2026-02-31', '2026-13-01', 'tomorrow', '2026-03-10x']) {
      expect(windowStart(bad), bad).toBeUndefined()
      expect(windowEnd(bad), bad).toBeUndefined()
    }
  })
})
