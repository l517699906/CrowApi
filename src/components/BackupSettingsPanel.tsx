import { useState } from "react";
import { relaunch } from "@tauri-apps/plugin-process";
import {
    ArchiveRestore,
    BookOpen,
    Check,
    Database,
    FileArchive,
    Files,
    HardDriveDownload,
    KeyRound,
    ShieldCheck,
    TriangleAlert,
} from "lucide-react";
import { backupApi } from "../lib/api";
import { errorMessage } from "../lib/query";
import type { BackupPreview, BackupSummary, BackupWriteResult } from "../types";
import { Modal, Toggle } from "./ui";

interface BackupSettingsPanelProps {
    onNotice: (message: string) => void;
}

type BackupAction = "create" | "inspect" | "restore" | null;

const numberFormatter = new Intl.NumberFormat("zh-CN");
const dateFormatter = new Intl.DateTimeFormat("zh-CN", {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
});

function formatBytes(bytes: number) {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    if (bytes < 1024 * 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
    return `${(bytes / 1024 / 1024 / 1024).toFixed(2)} GB`;
}

function BackupManifest({ summary }: { summary: BackupSummary }) {
    const entries = [
        { label: "数据库", value: formatBytes(summary.databaseBytes), icon: Database },
        { label: "外部文件", value: numberFormatter.format(summary.fileCount), icon: Files },
        { label: "渠道", value: numberFormatter.format(summary.channelCount), icon: HardDriveDownload },
        { label: "访问密钥", value: numberFormatter.format(summary.apiKeyCount), icon: KeyRound },
        { label: "知识库", value: numberFormatter.format(summary.knowledgeBaseCount), icon: FileArchive },
        { label: "Wiki 项目", value: numberFormatter.format(summary.wikiProjectCount), icon: BookOpen },
    ];

    return (
        <div className="backup-manifest" aria-label="备份内容清单">
            {entries.map((entry) => {
                const Icon = entry.icon;
                return (
                    <div key={entry.label}>
                        <Icon size={14} />
                        <span>{entry.label}</span>
                        <strong>{entry.value}</strong>
                    </div>
                );
            })}
        </div>
    );
}

export function BackupSettingsPanel({ onNotice }: BackupSettingsPanelProps) {
    const [createPassword, setCreatePassword] = useState("");
    const [confirmPassword, setConfirmPassword] = useState("");
    const [includeLogs, setIncludeLogs] = useState(false);
    const [lastBackup, setLastBackup] = useState<BackupWriteResult | null>(null);
    const [restorePassword, setRestorePassword] = useState("");
    const [keepLocalSettings, setKeepLocalSettings] = useState(true);
    const [preview, setPreview] = useState<BackupPreview | null>(null);
    const [confirmRestore, setConfirmRestore] = useState(false);
    const [restoreAcknowledged, setRestoreAcknowledged] = useState(false);
    const [activeAction, setActiveAction] = useState<BackupAction>(null);
    const [actionError, setActionError] = useState("");

    const handleCreate = async () => {
        setActionError("");
        if (createPassword.length < 10) {
            setActionError("备份口令至少需要 10 个字符");
            return;
        }
        if (createPassword !== confirmPassword) {
            setActionError("两次输入的备份口令不一致");
            return;
        }

        setActiveAction("create");
        try {
            const result = await backupApi.create(createPassword, includeLogs);
            if (result) {
                setLastBackup(result);
                setCreatePassword("");
                setConfirmPassword("");
                onNotice("加密备份已保存");
            }
        } catch (failure) {
            setActionError(errorMessage(failure));
        } finally {
            setActiveAction(null);
        }
    };

    const handleInspect = async () => {
        setActionError("");
        setPreview(null);
        if (restorePassword.length < 10) {
            setActionError("请输入创建备份时使用的口令");
            return;
        }

        setActiveAction("inspect");
        try {
            const result = await backupApi.inspect(restorePassword);
            if (result) {
                setPreview(result);
            }
        } catch (failure) {
            setActionError(errorMessage(failure));
        } finally {
            setActiveAction(null);
        }
    };

    const handleRestore = async () => {
        if (!preview || !restoreAcknowledged) return;
        setActionError("");
        setActiveAction("restore");
        try {
            await backupApi.scheduleRestore(
                preview.selectionId,
                restorePassword,
                keepLocalSettings,
            );
            onNotice("恢复已安排，正在重启 CrowAPI");
            await relaunch();
        } catch (failure) {
            setConfirmRestore(false);
            setRestoreAcknowledged(false);
            setPreview(null);
            setActionError(errorMessage(failure));
        } finally {
            setActiveAction(null);
        }
    };

    return (
        <>
            <div className="backup-console">
                <section className="backup-workflow" aria-labelledby="create-backup-title">
                    <div className="backup-workflow-header">
                        <span className="backup-workflow-icon"><HardDriveDownload size={18} /></span>
                        <div className="backup-workflow-heading">
                            <strong id="create-backup-title">创建加密备份</strong>
                            <span>数据库、设置、知识库文件和 Wiki 文件</span>
                        </div>
                        <span className="backup-security-mark"><ShieldCheck size={13} />本地加密</span>
                    </div>

                    <div className="backup-form-grid">
                        <label className="field-label">
                            <span>备份口令</span>
                            <input
                                className="field-input"
                                type="password"
                                autoComplete="new-password"
                                minLength={10}
                                value={createPassword}
                                onChange={(event) => setCreatePassword(event.target.value)}
                                placeholder="至少 10 个字符"
                            />
                        </label>
                        <label className="field-label">
                            <span>确认口令</span>
                            <input
                                className="field-input"
                                type="password"
                                autoComplete="new-password"
                                minLength={10}
                                value={confirmPassword}
                                onChange={(event) => setConfirmPassword(event.target.value)}
                                placeholder="再次输入备份口令"
                            />
                        </label>
                    </div>

                    <div className="backup-option-row">
                        <div>
                            <strong>包含请求日志</strong>
                            <span>日志可能明显增加备份大小，默认不包含</span>
                        </div>
                        <Toggle checked={includeLogs} label="包含请求日志" onChange={setIncludeLogs} />
                    </div>

                    <div className="backup-action-row">
                        <span>口令不会写入备份，丢失后无法恢复。</span>
                        <button
                            type="button"
                            className="button-primary"
                            disabled={activeAction !== null}
                            onClick={() => void handleCreate()}
                        >
                            {activeAction === "create" ? <span className="button-spinner is-inverse" /> : <HardDriveDownload size={16} />}
                            {activeAction === "create" ? "正在生成" : "创建备份"}
                        </button>
                    </div>

                    {lastBackup ? (
                        <div className="backup-result" role="status">
                            <Check size={15} />
                            <div>
                                <strong>最近备份已写入</strong>
                                <code title={lastBackup.path}>{lastBackup.path}</code>
                            </div>
                        </div>
                    ) : null}
                </section>

                <section className="backup-workflow backup-restore-workflow" aria-labelledby="restore-backup-title">
                    <div className="backup-workflow-header">
                        <span className="backup-workflow-icon is-amber"><ArchiveRestore size={18} /></span>
                        <div className="backup-workflow-heading">
                            <strong id="restore-backup-title">从备份恢复</strong>
                            <span>先解密检查清单，确认后在重启时替换本地数据</span>
                        </div>
                        <span className="backup-security-mark is-amber"><ArchiveRestore size={13} />需要重启</span>
                    </div>

                    <div className="backup-form-grid is-restore">
                        <label className="field-label">
                            <span>备份口令</span>
                            <input
                                className="field-input"
                                type="password"
                                autoComplete="current-password"
                                minLength={10}
                                value={restorePassword}
                                onChange={(event) => {
                                    setRestorePassword(event.target.value);
                                    setPreview(null);
                                    setConfirmRestore(false);
                                }}
                                placeholder="输入备份口令"
                            />
                        </label>
                        <button
                            type="button"
                            className="button-secondary"
                            disabled={activeAction !== null}
                            onClick={() => void handleInspect()}
                        >
                            {activeAction === "inspect" ? <span className="button-spinner" /> : <FileArchive size={16} />}
                            {activeAction === "inspect" ? "正在检查" : "选择并检查备份"}
                        </button>
                    </div>

                    <div className="backup-option-row">
                        <div>
                            <strong>保留当前设置</strong>
                            <span>恢复数据与文件，但继续使用本机的界面和服务设置</span>
                        </div>
                        <Toggle checked={keepLocalSettings} label="保留当前设置" onChange={setKeepLocalSettings} />
                    </div>

                    {preview ? (
                        <div className="backup-preview">
                            <div className="backup-preview-meta">
                                <div>
                                    <strong>备份清单</strong>
                                    <span>{dateFormatter.format(new Date(preview.summary.createdAt))} · CrowAPI v{preview.summary.appVersion}</span>
                                </div>
                                <span>{preview.summary.includesLogs ? "包含日志" : "不含日志"}</span>
                            </div>
                            <BackupManifest summary={preview.summary} />
                            <ul className="backup-warning-list">
                                {preview.warnings.map((warning) => (
                                    <li key={warning}><TriangleAlert size={14} />{warning}</li>
                                ))}
                            </ul>
                            <div className="backup-action-row is-restore">
                                <span>恢复前会为当前数据创建回滚副本。</span>
                                <button
                                    type="button"
                                    className="button-danger"
                                    disabled={activeAction !== null}
                                    onClick={() => {
                                        setRestoreAcknowledged(false);
                                        setConfirmRestore(true);
                                    }}
                                >
                                    <ArchiveRestore size={16} />恢复此备份
                                </button>
                            </div>
                        </div>
                    ) : null}
                </section>
            </div>

            {actionError ? <p className="form-error mt-4" role="alert">{actionError}</p> : null}

            {confirmRestore && preview ? (
                <Modal
                    title="确认恢复完整备份"
                    description="应用将在重启时替换本地数据"
                    size="sm"
                    onClose={() => {
                        if (activeAction !== "restore") setConfirmRestore(false);
                    }}
                    footer={(
                        <>
                            <button
                                type="button"
                                className="button-secondary"
                                disabled={activeAction === "restore"}
                                onClick={() => setConfirmRestore(false)}
                            >
                                取消
                            </button>
                            <button
                                type="button"
                                className="button-danger"
                                disabled={!restoreAcknowledged || activeAction === "restore"}
                                onClick={() => void handleRestore()}
                            >
                                {activeAction === "restore" ? <span className="button-spinner is-inverse" /> : <ArchiveRestore size={16} />}
                                {activeAction === "restore" ? "正在安排" : "恢复并重启"}
                            </button>
                        </>
                    )}
                >
                    <div className="backup-confirmation">
                        <span><TriangleAlert size={20} /></span>
                        <p>当前数据库、知识库文件和 Wiki 文件将被备份中的内容替换。恢复失败时会自动回滚。</p>
                    </div>
                    <label className="backup-confirm-check">
                        <input
                            type="checkbox"
                            checked={restoreAcknowledged}
                            onChange={(event) => setRestoreAcknowledged(event.target.checked)}
                        />
                        <span>已确认恢复范围，并已妥善保存当前所需数据</span>
                    </label>
                </Modal>
            ) : null}
        </>
    );
}
