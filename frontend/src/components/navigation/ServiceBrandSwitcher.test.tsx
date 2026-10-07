import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { MemoryRouter } from 'react-router-dom'
import { ServiceBrandSwitcher } from './ServiceBrandSwitcher'

beforeEach(() => {
  vi.stubGlobal('matchMedia', vi.fn(() => ({ addEventListener: vi.fn(), removeEventListener: vi.fn() })))
})
afterEach(() => { cleanup(); vi.unstubAllGlobals() })

const services = ['mail', 'mailer', 'docs', 'connect', 'notes', 'keylang'] as const
const names = { mail: 'CS Mail', mailer: 'CS Mailer', docs: 'CS Docs', connect: 'CS Connect', notes: 'CS Notes', keylang: 'CS KeyLang' }

describe('service-to-service logo navigation', () => {
  for (const current of services) {
    it(`offers every other service from ${names[current]}`, () => {
      render(<MemoryRouter><ServiceBrandSwitcher activeService={current} /></MemoryRouter>)
      const trigger = screen.getByRole('button', { name: `${names[current]}. Switch service` })
      fireEvent.click(trigger)
      const items = screen.getAllByRole('menuitem')
      expect(items).toHaveLength(7)
      for (const destination of services) {
        const item = screen.getByRole('menuitem', { name: names[destination] })
        expect(item.querySelector('img')?.getAttribute('src')).toBeTruthy()
        if (destination === current) {
          expect(item.getAttribute('aria-current')).toBe('true')
          expect(document.activeElement).toBe(item)
        } else {
          expect(item.getAttribute('href')).toBe(`https://${destination}.crescentsphere.com`)
        }
      }
      expect(screen.getByRole('menuitem', { name: 'CrescentSphere' }).getAttribute('href')).toBe('https://crescentsphere.com')
      const selectedIndex = items.indexOf(document.activeElement as HTMLElement)
      fireEvent.keyDown(document.activeElement!, { key: 'ArrowDown' })
      expect(document.activeElement).toBe(items[(selectedIndex + 1) % items.length])
      fireEvent.keyDown(document.activeElement!, { key: 'Escape' })
      expect(screen.queryByRole('menu')).toBeNull()
      expect(document.activeElement).toBe(trigger)
    })
  }

  it('keeps the current workspace when its own logo is selected', () => {
    render(<MemoryRouter initialEntries={['/mail/inbox']}><ServiceBrandSwitcher activeService="mail" /></MemoryRouter>)
    const trigger = screen.getByRole('button', { name: 'CS Mail. Switch service' })
    fireEvent.click(trigger)
    fireEvent.click(screen.getByRole('menuitem', { name: 'CS Mail' }))
    expect(screen.queryByRole('menu')).toBeNull()
    expect(document.activeElement).toBe(trigger)
  })

  it('copies service theme colors into the floating panel and updates an open panel', async () => {
    render(<MemoryRouter><ServiceBrandSwitcher activeService="docs" /></MemoryRouter>)
    const trigger = screen.getByRole('button', { name: 'CS Docs. Switch service' })
    // Set resolved tokens directly: jsdom does not implement CSS variable inheritance.
    trigger.style.setProperty('--ui-v2-surface', '#ffffff')
    trigger.style.setProperty('--ui-v2-text', '#171817')
    fireEvent.click(trigger)
    const panel = screen.getByRole('menu')
    expect(panel.style.getPropertyValue('--service-menu-surface')).toBe('#ffffff')
    expect(panel.style.getPropertyValue('--service-menu-text')).toBe('#171817')
    trigger.style.setProperty('--ui-v2-surface', '#222623')
    trigger.style.setProperty('--ui-v2-text', '#f4f1e9')
    await waitFor(() => {
      expect(panel.style.getPropertyValue('--service-menu-surface')).toBe('#222623')
      expect(panel.style.getPropertyValue('--service-menu-text')).toBe('#f4f1e9')
    })
  })
})
