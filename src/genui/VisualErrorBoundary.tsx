import { Component, type ReactNode } from 'react';

export class VisualErrorBoundary extends Component<
  { children: ReactNode; resetKey: string; fallback?: ReactNode; onError?: (error: Error) => void }, { failed: boolean }
> {
  state = { failed: false };
  componentDidCatch(error: Error) { this.props.onError?.(error); }
  static getDerivedStateFromError() { return { failed: true }; }
  componentDidUpdate(previous: Readonly<{ children: ReactNode; resetKey: string; fallback?: ReactNode; onError?: (error: Error) => void }>) {
    if (this.state.failed && previous.resetKey !== this.props.resetKey) this.setState({ failed: false });
  }
  render() {
    return this.state.failed
      ? this.props.fallback ?? <p className="genui-block-note" role="status">Couldn’t display this UI. Its source is still available.</p>
      : this.props.children;
  }
}
