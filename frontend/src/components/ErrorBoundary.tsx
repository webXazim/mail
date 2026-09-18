import { Component, type ErrorInfo, type ReactNode } from 'react'

export class ErrorBoundary extends Component<{ children: ReactNode }, { failed: boolean }> {
  state = { failed: false }
  static getDerivedStateFromError() {
    return { failed: true }
  }
  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error('Harbor Mail render error', error, info)
  }
  render() {
    if (!this.state.failed) return this.props.children
    return (
      <main className="error-page">
        <section className="error-page__card" role="alert">
          <p className="eyebrow">Harbor Mail</p>
          <h1>Something went wrong</h1>
          <p>An unexpected error interrupted this view. Reload to continue, or try again now.</p>
          <div className="error-page__actions">
            <button type="button" className="primary-button" onClick={() => location.reload()}>
              Reload
            </button>
            <button
              type="button"
              className="text-button"
              onClick={() => this.setState({ failed: false })}
            >
              Try again
            </button>
          </div>
        </section>
      </main>
    )
  }
}
