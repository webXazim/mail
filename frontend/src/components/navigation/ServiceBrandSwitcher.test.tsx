import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { MemoryRouter, useNavigate, type NavigateFunction } from 'react-router-dom'
import { ServiceBrandSwitcher } from './ServiceBrandSwitcher'

// SVG geometry is verified in a browser; jsdom cannot initialize the morph engine.
vi.mock('../../brand/engine/cs-morph-logo.js', () => ({}))
vi.mock('../../brand/engine/cs-logo-engine.js', () => ({}))

beforeEach(() => {
  vi.stubGlobal('matchMedia', vi.fn(() => ({ addEventListener: vi.fn(), removeEventListener: vi.fn() })))
})
afterEach(() => { cleanup(); vi.unstubAllGlobals() })

const services = ['mail', 'mailer', 'docs', 'connect', 'notes', 'keylang'] as const
const names = { mail: 'CS Mail', mailer: 'CS Mailer', docs: 'CS Docs', connect: 'CS Connect', notes: 'CS Notes', keylang: 'CS KeyLang' }

describe('service-to-service logo navigation', () => {
  it('closes on pathname and query changes and stays closed after navigating back', () => {
    let navigate: NavigateFunction
    function NavigationFixture() {
      navigate = useNavigate()
      return <ServiceBrandSwitcher activeService="mail" />
    }
    render(<MemoryRouter initialEntries={['/mail/inbox']}><NavigationFixture /></MemoryRouter>)
    const openMenu = () => fireEvent.click(screen.getByRole('button', { name: 'CS Mail. Switch service' }))
    openMenu()
    expect(screen.getByRole('menu')).toBeTruthy()
    act(() => { void navigate('/mail/sent') })
    expect(screen.queryByRole('menu')).toBeNull()
    openMenu()
    act(() => { void navigate('/mail/sent?filter=unread') })
    expect(screen.queryByRole('menu')).toBeNull()
    act(() => { void navigate(-1) })
    expect(screen.queryByRole('menu')).toBeNull()
    openMenu()
    expect(screen.getByRole('menu')).toBeTruthy()
  })

  for (const current of services) {
    it(`offers every other service from ${names[current]}`, () => {
      render(<MemoryRouter><ServiceBrandSwitcher activeService={current} /></MemoryRouter>)
      const trigger = screen.getByRole('button', { name: `${names[current]}. Switch service` })
      fireEvent.click(trigger)
      const items = screen.getAllByRole('menuitem')
      expect(items).toHaveLength(6)
      for (const destination of services) {
        if (destination === current) {
          expect(screen.queryByRole('menuitem', { name: names[destination] })).toBeNull()
          continue
        }
        const item = screen.getByRole('menuitem', { name: names[destination] })
        expect(item.querySelector('img')?.getAttribute('src')).toBeTruthy()
        expect(item.getAttribute('href')).toBe(`https://${destination}.crescentsphere.com`)
      }
      expect(screen.getByRole('menuitem', { name: 'CrescentSphere' }).getAttribute('href')).toBe('https://crescentsphere.com')
      const selectedIndex = items.indexOf(document.activeElement as HTMLElement)
      fireEvent.keyDown(document.activeElement!, { key: 'ArrowRight' })
      expect(document.activeElement).toBe(items[(selectedIndex + 1) % items.length])
      fireEvent.keyDown(document.activeElement!, { key: 'ArrowLeft' })
      expect(document.activeElement).toBe(items[selectedIndex])
      fireEvent.keyDown(document.activeElement!, { key: 'Escape' })
      expect(screen.queryByRole('menu')).toBeNull()
      expect(document.activeElement).toBe(trigger)
    })
  }

  it('closes the panel when its main logo is clicked again', () => {
    render(<MemoryRouter initialEntries={['/mail/inbox']}><ServiceBrandSwitcher activeService="mail" /></MemoryRouter>)
    const trigger = screen.getByRole('button', { name: 'CS Mail. Switch service' })
    fireEvent.click(trigger)
    fireEvent.click(trigger)
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
