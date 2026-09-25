import { describe, expect, it, vi } from 'vitest'
import { render, screen } from '@testing-library/react'
import { Reader } from './Reader'
import type { Mail, ReaderThreadItem } from '../types'

vi.mock('../services/remote-mail', () => ({ isRemoteMail: () => true }))

const mail: Mail = {
  id: 'message-1',
  threadId: 'thread-1',
  initials: 'WA',
  sender: 'Webx Azim',
  email: 'webxazim@gmail.com',
  subject: 'Test',
  preview: 'Hello world',
  time: 'Just now',
  label: 'Mail',
  color: 'teal',
  unread: false,
  to: ['hello@webxazim.com'],
}

const actions = {
  onReply: vi.fn(), onReplyAll: vi.fn(), onForward: vi.fn(),
  onToggleStar: vi.fn(), onToggleRead: vi.fn(),
}

describe('live mailbox reader', () => {
  it('does not fabricate a conversation while the provider request is pending or failed', () => {
    const view = render(<Reader mail={mail} threadStatus="loading" {...actions} />)
    expect(screen.getByText(/Loading the message from your mailbox/)).toBeTruthy()
    expect(screen.queryByText('Alex Morgan')).toBeNull()
    expect(screen.queryByText(/Hi Alex/)).toBeNull()

    view.rerender(<Reader mail={mail} threadStatus="error" onRetryThread={vi.fn()} {...actions} />)
    expect(screen.getByText('Could not load this conversation')).toBeTruthy()
    expect(screen.getByRole('button', { name: 'Retry' })).toBeTruthy()
    expect(screen.queryByText('Alex Morgan')).toBeNull()
  })

  it('renders only the messages returned by the provider', () => {
    const thread: ReaderThreadItem[] = [{
      id: 'message-1', threadId: 'thread-1', sender: 'Webx Azim',
      email: 'webxazim@gmail.com', initials: 'WA', color: 'teal',
      copy: 'Hello world', clearBody: 'Hello world', bodyHtml: '',
      subject: 'Test', time: 'Just now', to: ['hello@webxazim.com'],
    }]
    render(<Reader mail={mail} thread={thread} threadStatus="ready" {...actions} />)
    expect(screen.getByText('Hello world')).toBeTruthy()
    expect(screen.queryByText('Alex Morgan')).toBeNull()
    expect(screen.queryByText(/Hi Alex/)).toBeNull()
  })
})
