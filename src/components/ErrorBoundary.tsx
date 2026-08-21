import {
    Component,
    type ErrorInfo,
    type ReactNode,
} from "react";
import { QueryErrorResetBoundary } from "@tanstack/react-query";
import {
    LayoutDashboard,
    RefreshCw,
    RotateCcw,
    TriangleAlert,
} from "lucide-react";
import { useLocation, useNavigate } from "react-router-dom";
import { normalizeAppError } from "../lib/query";

interface BoundaryFallbackProps {
    error: unknown;
    resetErrorBoundary: () => void;
}

interface RenderErrorBoundaryProps {
    children: ReactNode;
    fallback: (props: BoundaryFallbackProps) => ReactNode;
    onReset?: () => void;
}

interface RenderErrorBoundaryState {
    error: unknown;
    hasError: boolean;
}

class RenderErrorBoundary extends Component<RenderErrorBoundaryProps, RenderErrorBoundaryState> {
    state: RenderErrorBoundaryState = {
        error: null,
        hasError: false,
    };

    static getDerivedStateFromError(error: unknown): RenderErrorBoundaryState {
        return { error, hasError: true };
    }

    componentDidCatch(error: unknown, info: ErrorInfo) {
        console.error("CrowAPI render error", error, info.componentStack);
    }

    private resetErrorBoundary = () => {
        this.props.onReset?.();
        this.setState({ error: null, hasError: false });
    };

    render() {
        if (this.state.hasError) {
            return this.props.fallback({
                error: this.state.error,
                resetErrorBoundary: this.resetErrorBoundary,
            });
        }
        return this.props.children;
    }
}

function GuardedQueryErrorBoundary({
    children,
    fallback,
}: {
    children: ReactNode;
    fallback: (props: BoundaryFallbackProps) => ReactNode;
}) {
    return (
        <RenderErrorBoundary fallback={fallback}>
            <QueryErrorResetBoundary>
                {({ reset }) => (
                    <RenderErrorBoundary onReset={reset} fallback={fallback}>
                        {children}
                    </RenderErrorBoundary>
                )}
            </QueryErrorResetBoundary>
        </RenderErrorBoundary>
    );
}

interface ErrorStateProps {
    error: unknown;
    scope: "root" | "route";
    onRetry: () => void;
    onSecondary: () => void;
}

function ErrorState({ error, scope, onRetry, onSecondary }: ErrorStateProps) {
    const normalized = normalizeAppError(error);
    const isRoot = scope === "root";

    return (
        <div className={`error-boundary-stage ${isRoot ? "is-root" : ""}`}>
            <section className="error-boundary-panel" role="alert" aria-live="assertive">
                <div className="error-boundary-signal" aria-hidden="true" />
                <div className="error-boundary-body">
                    <span className="error-boundary-icon" aria-hidden="true">
                        <TriangleAlert size={22} />
                    </span>
                    <div className="error-boundary-copy">
                        <p className="error-boundary-kicker">运行中断</p>
                        <h1>{isRoot ? "CrowAPI 界面启动失败" : "当前页面加载失败"}</h1>
                        <p className="error-boundary-message">
                            {normalized.message}
                        </p>
                        {(normalized.code !== "UNKNOWN" || normalized.trace_id) ? (
                            <dl className="error-boundary-diagnostics">
                                {normalized.code !== "UNKNOWN" ? (
                                    <div>
                                        <dt>错误码</dt>
                                        <dd>{normalized.code}</dd>
                                    </div>
                                ) : null}
                                {normalized.trace_id ? (
                                    <div>
                                        <dt>Trace ID</dt>
                                        <dd>{normalized.trace_id}</dd>
                                    </div>
                                ) : null}
                            </dl>
                        ) : null}
                        <div className="error-boundary-actions">
                            <button type="button" className="button-primary" onClick={onRetry} autoFocus>
                                <RefreshCw size={15} />
                                重试
                            </button>
                            <button type="button" className="button-secondary" onClick={onSecondary}>
                                {isRoot ? <RotateCcw size={15} /> : <LayoutDashboard size={15} />}
                                {isRoot ? "重新加载应用" : "返回仪表盘"}
                            </button>
                        </div>
                    </div>
                </div>
            </section>
        </div>
    );
}

export function RootErrorBoundary({ children }: { children: ReactNode }) {
    const fallback = ({ error, resetErrorBoundary }: BoundaryFallbackProps) => (
        <ErrorState
            error={error}
            scope="root"
            onRetry={resetErrorBoundary}
            onSecondary={() => window.location.reload()}
        />
    );

    return (
        <GuardedQueryErrorBoundary fallback={fallback}>
            {children}
        </GuardedQueryErrorBoundary>
    );
}

interface RouteErrorBoundaryProps {
    children: ReactNode;
}

export function RouteErrorBoundary({ children }: RouteErrorBoundaryProps) {
    const location = useLocation();
    const navigate = useNavigate();
    const fallback = ({ error, resetErrorBoundary }: BoundaryFallbackProps) => (
        <ErrorState
            error={error}
            scope="route"
            onRetry={resetErrorBoundary}
            onSecondary={() => {
                if (location.pathname === "/dashboard") {
                    window.location.reload();
                    return;
                }
                resetErrorBoundary();
                navigate("/dashboard", { replace: true });
            }}
        />
    );

    return (
        <GuardedQueryErrorBoundary key={location.pathname} fallback={fallback}>
            {children}
        </GuardedQueryErrorBoundary>
    );
}
