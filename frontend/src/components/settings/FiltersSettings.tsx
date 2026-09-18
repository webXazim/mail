import { useEffect, useState } from 'react'
import { Filter, Plus, Trash2, X } from 'lucide-react'
import {
  rulesApi,
  type FilterRule,
  type RuleAction,
  type RuleCondition,
} from '../../services/rules'
import { labelsApi } from '../../services/labels'
import { foldersApi } from '../../services/folders'

const conditionFields = [
  { id: 'from', label: 'From contains' },
  { id: 'to', label: 'To contains' },
  { id: 'subject', label: 'Subject contains' },
  { id: 'hasAttachment', label: 'Has attachment' },
  { id: 'size', label: 'Message size' },
  { id: 'date', label: 'Sent date' },
] as const
type ConditionField = (typeof conditionFields)[number]['id']

const actionOptions: { id: RuleAction['kind']; label: string }[] = [
  { id: 'label', label: 'Apply a label' },
  { id: 'move', label: 'Move to folder' },
  { id: 'archive', label: 'Archive' },
  { id: 'keep-in-inbox', label: 'Keep in Inbox' },
  { id: 'mark-read', label: 'Mark as read' },
  { id: 'mark-starred', label: 'Mark as starred' },
  { id: 'discard', label: 'Discard message' },
  { id: 'forward', label: 'Forward to' },
]

const buildCondition = (field: ConditionField, previous?: RuleCondition): RuleCondition => {
  if (field === 'hasAttachment') return { field }
  if (field === 'size') {
    const prev = previous && previous.field === 'size' ? previous : undefined
    return { field, op: prev?.op ?? 'larger', size: prev?.size ?? 100 }
  }
  if (field === 'date') {
    const prev = previous && previous.field === 'date' ? previous : undefined
    return { field, op: prev?.op ?? 'before', value: prev?.value ?? '' }
  }
  const prev = previous && previous.field === field ? previous : undefined
  return { field, value: prev?.value ?? '' }
}

const buildAction = (kind: RuleAction['kind'], previous?: RuleAction): RuleAction => {
  if (kind === 'label')
    return {
      kind,
      value: previous && 'value' in previous ? previous.value : (labelsApi.list()[0]?.name ?? ''),
    }
  if (kind === 'move')
    return {
      kind,
      value:
        previous && 'value' in previous ? previous.value : (foldersApi.list()[0]?.name ?? 'Inbox'),
    }
  if (kind === 'forward')
    return { kind, value: previous && 'value' in previous ? previous.value : '' }
  return { kind }
}

const describeCondition = (condition: RuleCondition): string => {
  if (condition.field === 'hasAttachment') return 'has an attachment'
  if (condition.field === 'size')
    return `${condition.field === 'size' && condition.op === 'larger' ? 'larger than' : 'smaller than'} ${condition.size} KB`
  if (condition.field === 'date')
    return `sent ${condition.op.replace('-', ' ')} ${condition.value || 'a date'}`
  return `${condition.field} contains “${condition.value}”`
}

const describeAction = (action: RuleAction): string => {
  if (action.kind === 'label' || action.kind === 'move' || action.kind === 'forward')
    return `${action.kind === 'forward' ? 'forward to' : action.kind}: ${action.value}`
  if (action.kind === 'keep-in-inbox') return 'keep in Inbox'
  if (action.kind === 'mark-read') return 'mark as read'
  if (action.kind === 'mark-starred') return 'mark as starred'
  if (action.kind === 'discard') return 'discard'
  return 'archive'
}

const emptyRule = (): FilterRule => ({
  id: '',
  name: '',
  enabled: true,
  conditions: [buildCondition('from')],
  actions: [buildAction('archive')],
})

export function FiltersSettings() {
  const [rules, setRules] = useState<FilterRule[]>(() => rulesApi.list())
  const [draft, setDraft] = useState<FilterRule | null>(null)
  const [notice, setNotice] = useState('')
  const [error, setError] = useState('')

  useEffect(() => {
    if (!notice) return
    const timer = window.setTimeout(() => setNotice(''), 2200)
    return () => window.clearTimeout(timer)
  }, [notice])

  const startNew = () => setDraft(emptyRule())
  const closeEditor = () => {
    setDraft(null)
    setError('')
  }

  const patchDraft = (patch: Partial<FilterRule>) =>
    setDraft((current) => (current ? { ...current, ...patch } : current))

  const updateCondition = (index: number, condition: RuleCondition) => {
    if (!draft) return
    const conditions = draft.conditions.map((item, itemIndex) =>
      itemIndex === index ? condition : item,
    )
    patchDraft({ conditions })
  }
  const updateAction = (index: number, action: RuleAction) => {
    if (!draft) return
    const actions = draft.actions.map((item, itemIndex) => (itemIndex === index ? action : item))
    patchDraft({ actions })
  }

  const valid = Boolean(
    draft &&
    draft.name.trim() &&
    draft.conditions.length > 0 &&
    draft.actions.length > 0 &&
    draft.conditions.every((condition) =>
      condition.field === 'hasAttachment'
        ? true
        : condition.field === 'size'
          ? condition.size > 0
          : condition.field === 'date'
            ? Boolean(condition.value)
            : Boolean(condition.value),
    ) &&
    draft.actions.every((action) =>
      action.kind === 'forward'
        ? /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(action.value)
        : action.kind === 'label' || action.kind === 'move'
          ? Boolean(action.value)
          : true,
    ),
  )

  const saveRule = () => {
    if (!draft || !valid) {
      setError('Give the rule a name and add at least one condition and one action.')
      return
    }
    const saved = draft.id
      ? rulesApi.update(draft)
      : rulesApi.add({ ...draft, id: `rule-${Date.now()}` })
    setRules(saved)
    setDraft(null)
    setError('')
    setNotice(draft.id ? 'Rule updated' : 'Rule created')
  }

  const removeRule = (id: string) => {
    setRules(rulesApi.remove(id))
    if (draft?.id === id) setDraft(null)
    setNotice('Rule removed')
  }

  const activeLabelOptions = labelsApi.list()
  const activeFolderOptions = [
    'Inbox',
    ...foldersApi.list().map((customFolder) => customFolder.name),
  ]

  return (
    <div>
      <div className="settings-section rule-section-head">
        <div>
          <h3>Incoming mail rules</h3>
          <p className="settings-hint">
            Rules run against new mail in the order shown. You can stack as many actions as you like
            on each rule.
          </p>
        </div>
        {!draft && (
          <button type="button" className="secondary-button" onClick={startNew}>
            <Plus size={15} />
            New rule
          </button>
        )}
      </div>

      {draft && (
        <div className="rule-card">
          <div className="rule-card__head">
            <strong>{draft.id ? 'Edit rule' : 'New rule'}</strong>
            <button
              type="button"
              className="icon-button"
              aria-label="Close rule editor"
              onClick={closeEditor}
            >
              <X size={15} />
            </button>
          </div>
          <label className="rule-name">
            Rule name
            <input
              value={draft.name}
              placeholder="e.g. Newsletter senders"
              aria-label="Rule name"
              onChange={(event) => patchDraft({ name: event.target.value })}
            />
          </label>

          <div className="rule-block">
            <h4>Match all of these conditions</h4>
            {draft.conditions.map((condition, index) => (
              <div className="rule-builder-row" key={index}>
                <select
                  aria-label="Condition field"
                  value={condition.field}
                  onChange={(event) =>
                    updateCondition(
                      index,
                      buildCondition(event.target.value as ConditionField, condition),
                    )
                  }
                >
                  {conditionFields.map((field) => (
                    <option key={field.id} value={field.id}>
                      {field.label}
                    </option>
                  ))}
                </select>
                {condition.field === 'hasAttachment' && (
                  <span className="rule-builder-static">yes</span>
                )}
                {condition.field === 'size' && (
                  <>
                    <select
                      aria-label="Size comparison"
                      value={condition.op}
                      onChange={(event) =>
                        updateCondition(index, {
                          ...condition,
                          op: event.target.value as 'larger' | 'smaller',
                        })
                      }
                    >
                      <option value="larger">Larger than</option>
                      <option value="smaller">Smaller than</option>
                    </select>
                    <input
                      type="number"
                      min={1}
                      aria-label="Size in KB"
                      value={condition.size}
                      onChange={(event) =>
                        updateCondition(index, {
                          ...condition,
                          size: Number(event.target.value) || 0,
                        })
                      }
                    />
                    <span className="rule-builder-static">KB</span>
                  </>
                )}
                {condition.field === 'date' && (
                  <>
                    <select
                      aria-label="Date comparison"
                      value={condition.op}
                      onChange={(event) =>
                        updateCondition(index, {
                          ...condition,
                          op: event.target.value as typeof condition.op,
                        })
                      }
                    >
                      <option value="before">Before</option>
                      <option value="after">After</option>
                      <option value="on">On</option>
                      <option value="on-or-after">On or after</option>
                      <option value="on-or-before">On or before</option>
                      <option value="not-on">Not on</option>
                    </select>
                    <input
                      type="date"
                      aria-label="Sent date"
                      value={condition.value}
                      onChange={(event) =>
                        updateCondition(index, { ...condition, value: event.target.value })
                      }
                    />
                  </>
                )}
                {condition.field === 'from' && (
                  <input
                    aria-label="From value"
                    value={
                      'value' in condition && typeof condition.value === 'string'
                        ? condition.value
                        : ''
                    }
                    placeholder="senders@company.com"
                    onChange={(event) =>
                      updateCondition(index, { ...condition, value: event.target.value })
                    }
                  />
                )}
                {condition.field === 'to' && (
                  <input
                    aria-label="To value"
                    value={
                      'value' in condition && typeof condition.value === 'string'
                        ? condition.value
                        : ''
                    }
                    placeholder="mailing-list@example.com"
                    onChange={(event) =>
                      updateCondition(index, { ...condition, value: event.target.value })
                    }
                  />
                )}
                {condition.field === 'subject' && (
                  <input
                    aria-label="Subject value"
                    value={
                      'value' in condition && typeof condition.value === 'string'
                        ? condition.value
                        : ''
                    }
                    placeholder="Annual report"
                    onChange={(event) =>
                      updateCondition(index, { ...condition, value: event.target.value })
                    }
                  />
                )}
                <button
                  type="button"
                  className="icon-button"
                  aria-label="Remove condition"
                  onClick={() =>
                    patchDraft({
                      conditions: draft.conditions.filter((_, itemIndex) => itemIndex !== index),
                    })
                  }
                >
                  <Trash2 size={15} />
                </button>
              </div>
            ))}
            <button
              type="button"
              className="text-button"
              onClick={() => {
                if (!draft) return
                patchDraft({ conditions: [...draft.conditions, buildCondition('from')] })
              }}
            >
              <Plus size={14} />
              Add condition
            </button>
          </div>

          <div className="rule-block">
            <h4>Then do the following</h4>
            {draft.actions.map((action, index) => (
              <div className="rule-builder-row" key={index}>
                <select
                  aria-label="Action kind"
                  value={action.kind}
                  onChange={(event) =>
                    updateAction(
                      index,
                      buildAction(event.target.value as RuleAction['kind'], action),
                    )
                  }
                >
                  {actionOptions.map((option) => (
                    <option key={option.id} value={option.id}>
                      {option.label}
                    </option>
                  ))}
                </select>
                {action.kind === 'label' && (
                  <select
                    aria-label="Label"
                    value={action.value}
                    onChange={(event) =>
                      updateAction(index, { ...action, value: event.target.value })
                    }
                  >
                    {activeLabelOptions.map((label) => (
                      <option key={label.name} value={label.name}>
                        {label.name}
                      </option>
                    ))}
                  </select>
                )}
                {action.kind === 'move' && (
                  <select
                    aria-label="Move to"
                    value={action.value}
                    onChange={(event) =>
                      updateAction(index, { ...action, value: event.target.value })
                    }
                  >
                    {activeFolderOptions.map((folder) => (
                      <option key={folder} value={folder}>
                        {folder}
                      </option>
                    ))}
                  </select>
                )}
                {action.kind === 'forward' && (
                  <input
                    aria-label="Forward address"
                    value={'value' in action ? action.value : ''}
                    placeholder="assistant@example.com"
                    onChange={(event) =>
                      updateAction(index, { ...action, value: event.target.value })
                    }
                  />
                )}
                {action.kind !== 'label' && action.kind !== 'move' && action.kind !== 'forward' && (
                  <span className="rule-builder-static">{describeAction(action)}</span>
                )}
                <button
                  type="button"
                  className="icon-button"
                  aria-label="Remove action"
                  onClick={() =>
                    patchDraft({
                      actions: draft.actions.filter((_, itemIndex) => itemIndex !== index),
                    })
                  }
                >
                  <Trash2 size={15} />
                </button>
              </div>
            ))}
            <button
              type="button"
              className="text-button"
              onClick={() => {
                if (!draft) return
                patchDraft({ actions: [...draft.actions, buildAction('archive')] })
              }}
            >
              <Plus size={14} />
              Add action
            </button>
          </div>

          {error && <p className="settings-notice">{error}</p>}
          <div className="row-actions">
            <button type="button" className="secondary-button" onClick={closeEditor}>
              Cancel
            </button>
            <button type="button" className="primary-button" onClick={saveRule} disabled={!valid}>
              <Filter size={15} />
              Save rule
            </button>
          </div>
        </div>
      )}

      {rules.map((rule) => (
        <div className="rule-row" key={rule.id}>
          <input
            type="checkbox"
            aria-label={`Enable rule ${rule.name}`}
            checked={rule.enabled}
            onChange={(event) => setRules(rulesApi.toggle(rule.id, event.target.checked))}
          />
          <div className="rule-row__text">
            <strong>{rule.name}</strong>
            <small>
              When {rule.conditions.map(describeCondition).join(' and ')} →{' '}
              {rule.actions.map(describeAction).join(', then ')}
            </small>
          </div>
          <div className="row-actions rule-row__actions">
            <button type="button" className="secondary-button" onClick={() => setDraft(rule)}>
              Edit
            </button>
            <button
              type="button"
              className="icon-button"
              aria-label={`Delete rule ${rule.name}`}
              onClick={() => removeRule(rule.id)}
            >
              <Trash2 size={15} />
            </button>
          </div>
        </div>
      ))}

      {rules.length === 0 && !draft && (
        <p className="settings-hint">
          No rules yet — create one so incoming mail lands where you want it.
        </p>
      )}
      {notice && <p className="settings-notice settings-notice--ok">{notice}</p>}
    </div>
  )
}
