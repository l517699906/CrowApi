import {
    type ButtonHTMLAttributes,
    type ReactNode,
    useEffect,
    useId,
    useRef,
} from "react";
import { Check, X } from "lucide-react";

interface PageTitleProps {
    title: string;
    meta?: string;
    action?: ReactNode;
}

export function PageTitle({ title, meta, action }: PageTitleProps) {
    return (
        <div className="mb-6 flex min-h-10 flex-wrap items-start justify-between gap-4">
            <div>
                <h1 className="font-display text-[24px] font-semibold leading-8 text-ink">{title}</h1>
                {meta ? <p className="mt-1 text-sm text-muted">{meta}</p> : null}
            </div>
            {action ? <div className="page-title-action flex items-center gap-2">{action}</div> : null}
        </div>
    );
}

interface IconButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
    label: string;
    children: ReactNode;
    tone?: "default" | "danger";
}

export function IconButton({ label, children, tone = "default", className = "", ...props }: IconButtonProps) {
    return (
        <button
            type="button"
            aria-label={label}
            title={label}
            className={`icon-button ${tone === "danger" ? "icon-button-danger" : ""} ${className}`}
            {...props}
        >
            {children}
        </button>
    );
}

interface ModalProps {
    title: string;
    description?: string;
    children: ReactNode;
    onClose: () => void;
    footer?: ReactNode;
    size?: "sm" | "md" | "lg";
}

export function Modal({ title, description, children, onClose, footer, size = "md" }: ModalProps) {
    const titleId = useId();
    const descriptionId = useId();
    const panelRef = useRef<HTMLElement>(null);
    const onCloseRef = useRef(onClose);

    useEffect(() => {
        onCloseRef.current = onClose;
    }, [onClose]);

    useEffect(() => {
        const previousOverflow = document.body.style.overflow;
        const previousActiveElement = document.activeElement instanceof HTMLElement ? document.activeElement : null;
        document.body.style.overflow = "hidden";
        const handleKeyDown = (event: KeyboardEvent) => {
            if (event.key === "Escape") {
                onCloseRef.current();
                return;
            }

            if (event.key === "Tab" && panelRef.current) {
                const focusableElements = Array.from(panelRef.current.querySelectorAll<HTMLElement>(
                    "button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), a[href], [tabindex]:not([tabindex='-1'])",
                ));
                const firstElement = focusableElements[0];
                const lastElement = focusableElements[focusableElements.length - 1];

                if (event.shiftKey && document.activeElement === firstElement) {
                    event.preventDefault();
                    lastElement?.focus();
                } else if (!event.shiftKey && document.activeElement === lastElement) {
                    event.preventDefault();
                    firstElement?.focus();
                }
            }
        };
        window.addEventListener("keydown", handleKeyDown);
        window.requestAnimationFrame(() => {
            const autofocusElement = panelRef.current?.querySelector<HTMLElement>("[autofocus]");
            const firstFocusableElement = panelRef.current?.querySelector<HTMLElement>(
                "button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), a[href]",
            );
            (autofocusElement ?? firstFocusableElement ?? panelRef.current)?.focus();
        });

        return () => {
            document.body.style.overflow = previousOverflow;
            window.removeEventListener("keydown", handleKeyDown);
            previousActiveElement?.focus();
        };
    }, []);

    return (
        <div
            className="modal-backdrop"
            role="presentation"
            onMouseDown={(event) => {
                if (event.target === event.currentTarget) {
                    onClose();
                }
            }}
        >
            <section
                ref={panelRef}
                className={`modal-panel modal-${size}`}
                role="dialog"
                aria-modal="true"
                aria-labelledby={titleId}
                aria-describedby={description ? descriptionId : undefined}
                tabIndex={-1}
            >
                <header className="flex items-start justify-between gap-5 border-b border-line px-6 py-5">
                    <div>
                        <h2 id={titleId} className="font-display text-lg font-semibold text-ink">{title}</h2>
                        {description ? <p id={descriptionId} className="mt-1 text-sm text-muted">{description}</p> : null}
                    </div>
                    <IconButton label="关闭" onClick={onClose} className="-mr-1 -mt-1">
                        <X size={18} />
                    </IconButton>
                </header>
                <div className="modal-content px-6 py-5">{children}</div>
                {footer ? <footer className="flex flex-wrap justify-end gap-2 border-t border-line px-6 py-4">{footer}</footer> : null}
            </section>
        </div>
    );
}

interface ToggleProps {
    checked: boolean;
    onChange: (checked: boolean) => void;
    label: string;
    disabled?: boolean;
}

export function Toggle({ checked, onChange, label, disabled = false }: ToggleProps) {
    return (
        <button
            type="button"
            role="switch"
            aria-checked={checked}
            aria-label={label}
            title={label}
            disabled={disabled}
            className={`toggle ${checked ? "toggle-on" : ""}`}
            onClick={() => onChange(!checked)}
        >
            <span className="toggle-thumb" />
        </button>
    );
}

interface StatusBadgeProps {
    status: "success" | "warning" | "danger" | "neutral" | "info";
    children: ReactNode;
    dot?: boolean;
}

export function StatusBadge({ status, children, dot = false }: StatusBadgeProps) {
    return (
        <span className={`status-badge status-${status}`}>
            {dot ? <span className="status-dot" aria-hidden="true" /> : null}
            {children}
        </span>
    );
}

interface ProviderMarkProps {
    type: string;
    size?: "sm" | "md";
}

const providerInitials: Record<string, string> = {
    openai: "OA",
    deepseek: "DS",
    claude: "CL",
    gemini: "GE",
    custom: "API",
};

export function ProviderMark({ type, size = "md" }: ProviderMarkProps) {
    const normalizedType = providerInitials[type.toLowerCase()] ? type.toLowerCase() : "custom";

    return (
        <span className={`provider-mark provider-${normalizedType} provider-${size}`} aria-hidden="true">
            {providerInitials[normalizedType]}
        </span>
    );
}

interface SegmentedControlProps<T extends string> {
    value: T;
    options: ReadonlyArray<{ value: T; label: string }>;
    onChange: (value: T) => void;
    label: string;
}

export function SegmentedControl<T extends string>({ value, options, onChange, label }: SegmentedControlProps<T>) {
    return (
        <div className="segmented-control" role="group" aria-label={label}>
            {options.map((option) => (
                <button
                    key={option.value}
                    type="button"
                    className={value === option.value ? "is-active" : ""}
                    aria-pressed={value === option.value}
                    onClick={() => onChange(option.value)}
                >
                    {option.label}
                </button>
            ))}
        </div>
    );
}

interface ToastProps {
    message: string;
}

export function Toast({ message }: ToastProps) {
    return (
        <div className="toast" role="status">
            <Check size={16} />
            <span>{message}</span>
        </div>
    );
}
