import React from "react";
import { AlertTriangle, RotateCcw } from "lucide-react";

interface ErrorBoundaryProps {
  children: React.ReactNode;
  /** Shown above the fallback; helps identify which view crashed. */
  label?: string;
}

interface ErrorBoundaryState {
  error: Error | null;
}

/**
 * Isolates render crashes to a single view: without this, one throwing
 * component unmounts the entire app into a blank screen.
 */
export class ErrorBoundary extends React.Component<ErrorBoundaryProps, ErrorBoundaryState> {
  state: ErrorBoundaryState = { error: null };

  static getDerivedStateFromError(error: Error): ErrorBoundaryState {
    return { error };
  }

  componentDidCatch(error: Error, info: React.ErrorInfo) {
    console.error(`[ErrorBoundary${this.props.label ? `:${this.props.label}` : ""}]`, error, info.componentStack);
  }

  render() {
    if (this.state.error) {
      return (
        <div className="rounded-panel border border-rose-500/30 bg-rose-500/[0.07] p-8 text-center">
          <AlertTriangle className="h-8 w-8 text-rose-400 mx-auto" />
          <h3 className="mt-3 text-sm font-semibold text-rose-200">
            {this.props.label ? `Ошибка в разделе «${this.props.label}»` : "Что-то пошло не так"}
          </h3>
          <p className="mt-1.5 text-xs text-ink3 max-w-md mx-auto leading-relaxed">
            {this.state.error.message}
          </p>
          <button
            onClick={() => this.setState({ error: null })}
            className="t-btn t-btn-ghost mt-4 inline-flex items-center gap-2"
          >
            <RotateCcw className="h-3.5 w-3.5" />
            Попробовать снова
          </button>
        </div>
      );
    }
    return this.props.children;
  }
}
