import { screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { renderWithProviders } from '@/test/render-with-providers'

import { DeviceStatusBadge } from './device-status-badge'

function markerOf(label: string): string {
  const marker = screen.getByText(label).querySelector('[aria-hidden]')
  if (marker === null) throw new Error(`no marker for ${label}`)
  return marker.className
}

describe('DeviceStatusBadge', () => {
  it('gives online, offline and disabled each its word and a marker of its own', () => {
    renderWithProviders(
      <>
        <DeviceStatusBadge device={{ online: true, status: 'active' }} />
        <DeviceStatusBadge device={{ online: false, status: 'active' }} />
        <DeviceStatusBadge device={{ online: true, status: 'disabled' }} />
      </>,
    )

    const markers = ['在线', '离线', '已禁用'].map(markerOf)
    expect(new Set(markers).size).toBe(3)
  })

  it('marks a disabled device as stopped by someone, not as an error', () => {
    renderWithProviders(<DeviceStatusBadge device={{ online: false, status: 'disabled' }} />)

    expect(markerOf('已禁用')).not.toMatch(/destructive/)
  })
})
