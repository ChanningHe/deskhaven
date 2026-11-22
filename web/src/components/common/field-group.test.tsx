import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { FieldGroup } from './field-group'

describe('FieldGroup', () => {
  it('titles the group with a level 3 heading and its description', () => {
    render(
      <FieldGroup title="Connection" description="Where the directory server is.">
        <input aria-label="Host" />
      </FieldGroup>,
    )

    expect(screen.getByRole('heading', { level: 3, name: 'Connection' })).toBeInTheDocument()
    expect(screen.getByText('Where the directory server is.')).toBeInTheDocument()
    expect(screen.getByRole('textbox', { name: 'Host' })).toBeInTheDocument()
  })

  it('leaves the description out when none is given', () => {
    const { container } = render(
      <FieldGroup title="Mapping">
        <input aria-label="Attribute" />
      </FieldGroup>,
    )

    expect(screen.getByRole('heading', { level: 3, name: 'Mapping' })).toBeInTheDocument()
    expect(container.querySelector('p')).toBeNull()
  })
})
