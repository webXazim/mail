import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { MailBodyFrame } from './MailBodyFrame'

describe('MailBodyFrame', () => {
  it('uses a transparent, content-sized sandbox instead of a fixed white canvas', () => {
    render(<MailBodyFrame html="<p>Hello world</p>" subject="Test" />)

    const frame = screen.getByTitle('Message body — Test') as HTMLIFrameElement
    expect(frame).toHaveAttribute('sandbox', 'allow-same-origin')
    expect(frame.srcdoc).toContain('background:transparent!important')
    expect(frame.style.background).toBe('transparent')
    expect(frame.style.height).toBe('48px')
    expect(frame.style.height).not.toBe('420px')

    fireEvent.load(frame)
    expect(frame.style.overflow).toBe('hidden')
  })
})
