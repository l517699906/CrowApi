import type { RiskLevel } from "../types";

interface RiskPresentation {
    label: string;
    level: RiskLevel;
}

const RISK_PRESENTATIONS: Record<RiskLevel, RiskPresentation> = {
    clean: { label: "无风险", level: "clean" },
    info: { label: "风险提示", level: "info" },
    low: { label: "低风险", level: "low" },
    medium: { label: "中风险", level: "medium" },
    high: { label: "高风险", level: "high" },
    critical: { label: "严重风险", level: "critical" },
};

const UNKNOWN_RISK_PRESENTATION: RiskPresentation = {
    label: "未知风险",
    level: "info",
};

interface LogRiskBadgeProps {
    level: RiskLevel | string;
    score?: number;
}

export function LogRiskBadge({ level, score }: LogRiskBadgeProps) {
    const presentation = RISK_PRESENTATIONS[level as RiskLevel] ?? UNKNOWN_RISK_PRESENTATION;

    return (
        <span
            className={`risk-badge risk-${presentation.level}`}
            title={`安全等级：${level}`}
        >
            <span className="risk-dot" aria-hidden="true" />
            {presentation.label}
            {typeof score === "number" ? <span className="risk-score">{score}</span> : null}
        </span>
    );
}
