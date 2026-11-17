import { Component, type ErrorInfo, type ReactNode } from "react";

interface ErrorBoundaryProps {
  children: ReactNode;
  context: string;
  /** Rendered in place of the children after a render error. */
  fallback: (error: Error, componentStack: string) => ReactNode;
}

interface ErrorBoundaryState {
  error: Error | null;
  componentStack: string;
}

/** Keeps a render error from unmounting the whole window and leaving it blank. */
export class ErrorBoundary extends Component<
  ErrorBoundaryProps,
  ErrorBoundaryState
> {
  state: ErrorBoundaryState = { error: null, componentStack: "" };

  static getDerivedStateFromError(error: unknown): Partial<ErrorBoundaryState> {
    return {
      error: error instanceof Error ? error : new Error(String(error)),
    };
  }

  componentDidCatch(error: unknown, info: ErrorInfo): void {
    console.error(
      `Error rendering ${this.props.context}:`,
      error,
      info.componentStack,
    );
    this.setState({ componentStack: info.componentStack ?? "" });
  }

  render(): ReactNode {
    const { error, componentStack } = this.state;
    if (error) return this.props.fallback(error, componentStack);
    return this.props.children;
  }
}
