import { render, screen, within } from '@testing-library/react'
import { createRef } from 'react'
import { describe, expect, it } from 'vitest'

import { SectionCard } from './section-card'

describe('SectionCard', () => {
  it('is a region named by its level 2 heading, with the description under it', () => {
    render(
      <SectionCard title="LDAP directory" description="Sign in with directory accounts.">
        <p>body</p>
      </SectionCard>,
    )

    const region = screen.getByRole('region', { name: 'LDAP directory' })
    expect(within(region).getByRole('heading', { level: 2 })).toHaveTextContent('LDAP directory')
    expect(within(region).getByText('Sign in with directory accounts.')).toBeInTheDocument()
    expect(within(region).getByText('body')).toBeInTheDocument()
  })

  it('renders only the slots it is given', () => {
    render(
      <SectionCard title="Bare">
        <p>body</p>
      </SectionCard>,
    )

    const region = screen.getByRole('region', { name: 'Bare' })
    expect(region.querySelector('[data-slot="section-card-status"]')).toBeNull()
    expect(region.querySelector('[data-slot="section-card-actions"]')).toBeNull()
    expect(region.querySelector('[data-slot="section-card-footer"]')).toBeNull()
    expect(region).toHaveTextContent(/^Barebody$/)
  })

  it('renders status, actions and footer when given', () => {
    render(
      <SectionCard
        title="Full"
        status={<span>Enabled</span>}
        actions={<button type="button">Add</button>}
        footer={<button type="submit">Save</button>}
      >
        <p>body</p>
      </SectionCard>,
    )

    const region = screen.getByRole('region', { name: 'Full' })
    const status = region.querySelector('[data-slot="section-card-status"]')
    const actions = region.querySelector('[data-slot="section-card-actions"]')
    const footer = region.querySelector('[data-slot="section-card-footer"]')
    expect(status).toHaveTextContent('Enabled')
    expect(within(actions as HTMLElement).getByRole('button', { name: 'Add' })).toBeVisible()
    expect(within(footer as HTMLElement).getByRole('button', { name: 'Save' })).toBeVisible()
  })

  it('keeps the region name to the title when a status badge sits next to it', () => {
    render(
      <SectionCard title="Named" status={<span>Disabled</span>}>
        <p>body</p>
      </SectionCard>,
    )

    expect(screen.getByRole('region', { name: 'Named' })).toBeInTheDocument()
  })

  it('makes the heading a focus target only when a title ref is asked for', () => {
    const titleRef = createRef<HTMLHeadingElement>()
    render(
      <>
        <SectionCard title="Target" titleRef={titleRef}>
          <p>a</p>
        </SectionCard>
        <SectionCard title="Plain">
          <p>b</p>
        </SectionCard>
      </>,
    )

    const target = screen.getByRole('heading', { name: 'Target' })
    expect(titleRef.current).toBe(target)
    expect(target).toHaveAttribute('tabindex', '-1')
    expect(screen.getByRole('heading', { name: 'Plain' })).not.toHaveAttribute('tabindex')
  })

  it('gives each card its own heading id', () => {
    render(
      <>
        <SectionCard title="One">
          <p>a</p>
        </SectionCard>
        <SectionCard title="Two">
          <p>b</p>
        </SectionCard>
      </>,
    )

    expect(screen.getByRole('region', { name: 'One' })).toBeInTheDocument()
    expect(screen.getByRole('region', { name: 'Two' })).toBeInTheDocument()
  })
})
