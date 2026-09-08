import { useRef, useState } from 'react'
import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it } from 'vitest'
import { useFocusTrap } from './useFocusTrap'

function Dialog({ close }: { close: () => void }) {
  const ref = useRef<HTMLElement>(null)
  useFocusTrap(ref)
  return (
    <section ref={ref} role="dialog">
      <button>First</button>
      <button>Second</button>
      <button onClick={close}>Close</button>
    </section>
  )
}

function Harness() {
  const [open, setOpen] = useState(false)
  return (
    <div>
      <button onClick={() => setOpen(true)}>Open</button>
      {open && <Dialog close={() => setOpen(false)} />}
    </div>
  )
}

describe('useFocusTrap', () => {
  it('focuses the first focusable element when the dialog opens', async () => {
    render(<Harness />)
    const user = userEvent.setup()
    await user.click(screen.getByRole('button', { name: 'Open' }))
    expect(screen.getByRole('dialog')).toBeTruthy()
    expect(document.activeElement).toHaveTextContent('First')
  })

  it('wraps focus forward from the last to the first element', async () => {
    render(<Harness />)
    const user = userEvent.setup()
    await user.click(screen.getByRole('button', { name: 'Open' }))
    const first = screen.getByRole('button', { name: 'First' })
    const last = screen.getByRole('button', { name: 'Close' })
    first?.focus()
    await user.tab()
    await user.tab()
    expect(document.activeElement).toBe(last)
    await user.tab()
    expect(document.activeElement).toBe(first)
  })

  it('restores focus to the element that opened the dialog on close', async () => {
    render(<Harness />)
    const user = userEvent.setup()
    const opener = screen.getByRole('button', { name: 'Open' })
    await user.click(opener)
    await user.click(screen.getByRole('button', { name: 'Close' }))
    expect(document.activeElement).toBe(opener)
  })
})