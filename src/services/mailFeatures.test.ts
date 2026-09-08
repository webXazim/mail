import { beforeEach, describe, expect, it } from 'vitest'
import { foldersApi } from './folders'
import { rulesApi } from './rules'

beforeEach(() => localStorage.clear())

describe('foldersApi', () => {
  it('starts with a seeded Projects folder', () => {
    expect(foldersApi.list().some(folder => folder.name === 'Projects')).toBe(true)
  })

  it('adds, renames and removes folders', () => {
    foldersApi.add('Legal')
    const legal = foldersApi.byName('Legal')
    expect(legal).toBeDefined()
    if (!legal) return
    foldersApi.rename(legal.id, 'Contracts')
    expect(foldersApi.byId(legal.id)?.name).toBe('Contracts')
    foldersApi.remove(legal.id)
    expect(foldersApi.byId(legal.id)).toBeUndefined()
  })

  it('ignores blank names', () => {
    const before = foldersApi.list().length
    expect(foldersApi.add('   ').length).toBe(before)
  })
})

describe('rulesApi', () => {
  it('manages a rule lifecycle', () => {
    const rule = { id: 'rule-test', name: 'Big client', enabled: true, conditions: [{ field: 'from' as const, value: 'boss@harbor.co' }], actions: [{ kind: 'archive' as const }] }
    rulesApi.add(rule)
    expect(rulesApi.list().some(item => item.id === 'rule-test')).toBe(true)
    rulesApi.toggle('rule-test', false)
    expect(rulesApi.list().find(item => item.id === 'rule-test')?.enabled).toBe(false)
    rulesApi.remove('rule-test')
    expect(rulesApi.list().some(item => item.id === 'rule-test')).toBe(false)
  })
})