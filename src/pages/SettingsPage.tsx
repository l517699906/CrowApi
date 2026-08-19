import { useEffect, useRef, useState } from "react";
import {
    Check,
    CircleHelp,
    DownloadCloud,
    Gauge,
    Globe2,
    Monitor,
    Palette,
    RefreshCcw,
    Save,
    Server,
    Settings2,
    ShieldCheck,
} from "lucide-react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { getVersion } from "@tauri-apps/api/app";
import { relaunch } from "@tauri-apps/plugin-process";
import { check as checkForAppUpdate, type DownloadEvent, type Update } from "@tauri-apps/plugin-updater";
import { DEFAULT_SETTINGS } from "../config/defaults";
import { settingsApi } from "../lib/api";
import { MAX_QUOTA, normalizeQuota } from "../lib/quota";
import { errorMessage, queryKeys } from "../lib/query";
import { applyUiTheme } from "../lib/theme";
import type { Settings, UiTheme } from "../types";
import { PageTitle, Toast, Toggle } from "../components/ui";

type SettingsTab = "service" | "quota" | "general" | "interface" | "retry" | "update";

const tabs: Array<{ id: SettingsTab; label: string; icon: typeof Server }> = [
    { id: "service", label: "服务配置", icon: Server },
    { id: "quota", label: "配额管理", icon: Gauge },
    { id: "general", label: "通用设置", icon: Settings2 },
    { id: "interface", label: "界面设置", icon: Palette },
    { id: "retry", label: "重试策略", icon: RefreshCcw },
    { id: "update", label: "软件更新", icon: DownloadCloud },
];

const themeOptions: ReadonlyArray<{ value: UiTheme; label: string; colors: readonly [string, string, string] }> = [
    { value: "light", label: "浅色", colors: ["#f4f6f5", "#11815d", "#3d73c3"] },
    { value: "system", label: "跟随系统", colors: ["#f4f6f5", "#121714", "#3db88a"] },
    { value: "dark", label: "深色", colors: ["#121714", "#3db88a", "#76a4e5"] },
    { value: "mist", label: "雾青", colors: ["#eef3f5", "#087f72", "#3a6fc8"] },
    { value: "ember", label: "余烬", colors: ["#181616", "#df6258", "#68a4d7"] },
    { value: "graphite", label: "石墨", colors: ["#f1f2f3", "#c33b32", "#326ca8"] },
    { value: "frost", label: "冰川", colors: ["#edf4f7", "#167e94", "#4b63c7"] },
    { value: "sakura", label: "樱庭", colors: ["#f8f1f5", "#ad3f70", "#2f7f86"] },
    { value: "mono", label: "黑白", colors: ["#f5f5f3", "#1e5fd8", "#d14d61"] },
    { value: "ocean", label: "深海", colors: ["#0f1820", "#36c2b4", "#69a7e8"] },
    { value: "neon", label: "霓虹", colors: ["#101114", "#22c8bd", "#d66ac7"] },
];

type UpdatePhase = "idle" | "checking" | "downloading" | "installing" | "current" | "error";

export function SettingsPage() {
    const queryClient = useQueryClient();
    const { data: settings, isPending, error } = useQuery({
        queryKey: queryKeys.settings,
        queryFn: settingsApi.get,
    });
    const [activeTab, setActiveTab] = useState<SettingsTab>("service");
    const [draft, setDraft] = useState<Settings>(() => ({ ...DEFAULT_SETTINGS }));
    const [toast, setToast] = useState("");
    const [currentVersion, setCurrentVersion] = useState("--");
    const [latestVersion, setLatestVersion] = useState<string | null>(null);
    const [updatePhase, setUpdatePhase] = useState<UpdatePhase>("idle");
    const [updateProgress, setUpdateProgress] = useState(0);
    const [updateError, setUpdateError] = useState("");
    const persistedThemeRef = useRef<UiTheme>(DEFAULT_SETTINGS.ui_theme);
    const previewingThemeRef = useRef(false);
    const saveMutation = useMutation({
        mutationFn: settingsApi.save,
        onSuccess: (_, savedSettings) => {
            persistedThemeRef.current = savedSettings.ui_theme;
            previewingThemeRef.current = false;
            queryClient.setQueryData(queryKeys.settings, savedSettings);
            setToast("设置已保存");
            window.setTimeout(() => setToast(""), 1800);
        },
    });

    useEffect(() => {
        if (settings) {
            persistedThemeRef.current = settings.ui_theme;
            setDraft({ ...settings });
        }
    }, [settings]);

    useEffect(() => () => {
        if (previewingThemeRef.current) {
            applyUiTheme(persistedThemeRef.current);
        }
    }, []);

    useEffect(() => {
        void getVersion().then(setCurrentVersion).catch(() => setCurrentVersion("0.1.0"));
    }, []);

    function updateSetting<K extends keyof Settings>(key: K, value: Settings[K]) {
        setDraft((current) => ({ ...current, [key]: value }));
    }

    function previewTheme(theme: UiTheme) {
        previewingThemeRef.current = true;
        updateSetting("ui_theme", theme);
        applyUiTheme(theme);
    }

    const save = async () => {
        try {
            await saveMutation.mutateAsync(draft);
        } catch {
            // Mutation error is rendered with the settings form.
        }
    };

    const checkAndInstallUpdate = async () => {
        let update: Update | null = null;
        setUpdatePhase("checking");
        setUpdateProgress(0);
        setUpdateError("");
        setLatestVersion(null);

        try {
            update = await checkForAppUpdate({ timeout: 10_000 });
            if (!update) {
                setUpdatePhase("current");
                return;
            }

            setLatestVersion(update.version);
            setUpdatePhase("downloading");
            let downloadedBytes = 0;
            let contentLength = 0;
            const handleDownloadEvent = (event: DownloadEvent) => {
                if (event.event === "Started") {
                    contentLength = event.data.contentLength ?? 0;
                    downloadedBytes = 0;
                } else if (event.event === "Progress") {
                    downloadedBytes += event.data.chunkLength;
                    if (contentLength > 0) {
                        setUpdateProgress(Math.min(99, Math.round((downloadedBytes / contentLength) * 100)));
                    }
                } else {
                    setUpdateProgress(100);
                    setUpdatePhase("installing");
                }
            };

            await update.downloadAndInstall(handleDownloadEvent, { timeout: 120_000 });
            await update.close();
            update = null;
            await relaunch();
        } catch (updateFailure) {
            setUpdatePhase("error");
            setUpdateError(errorMessage(updateFailure));
        } finally {
            if (update) {
                await update.close().catch(() => undefined);
            }
        }
    };

    return (
        <div className="page-enter">
            <PageTitle
                title="设置"
                meta="本地网关配置"
                action={(
                    <button type="button" className="button-primary" onClick={save} disabled={saveMutation.isPending || isPending}>
                        <Save size={16} />{saveMutation.isPending ? "保存中..." : "保存更改"}
                    </button>
                )}
            />

            <div className="settings-layout">
                <nav className="settings-tabs" aria-label="设置分类">
                    {tabs.map((tab) => {
                        const Icon = tab.icon;
                        return (
                            <button
                                key={tab.id}
                                type="button"
                                className={activeTab === tab.id ? "is-active" : ""}
                                aria-current={activeTab === tab.id ? "page" : undefined}
                                onClick={() => setActiveTab(tab.id)}
                            >
                                <Icon size={17} />
                                <span>{tab.label}</span>
                            </button>
                        );
                    })}
                </nav>

                <section className="settings-panel">
                    {activeTab === "service" ? (
                        <div className="settings-section page-enter" key="service">
                            <div className="settings-heading">
                                <span className="settings-heading-icon"><Server size={19} /></span>
                                <div><h2>服务配置</h2><p>本地监听地址与端口</p></div>
                            </div>
                            <div className="settings-form-block">
                                <div className="form-grid">
                                    <label className="field-label">
                                        <span>监听地址</span>
                                        <input className="field-input font-mono" value={draft.server_host} onChange={(event) => updateSetting("server_host", event.target.value)} />
                                    </label>
                                    <label className="field-label">
                                        <span>服务端口</span>
                                        <input className="field-input font-mono" type="number" min="1024" max="65535" value={draft.server_port} onChange={(event) => updateSetting("server_port", Number(event.target.value))} />
                                    </label>
                                </div>
                                <label className="field-label mt-5">
                                    <span>OpenAI 兼容地址</span>
                                    <div className="readonly-field">
                                        <code>{`http://${draft.server_host}:${draft.server_port}/v1`}</code>
                                        <Check size={15} className="text-accent" />
                                    </div>
                                </label>
                            </div>
                            <div className="settings-note"><CircleHelp size={17} /><span>服务配置将在网关下次启动时生效。</span></div>
                        </div>
                    ) : null}

                    {activeTab === "quota" ? (
                        <div className="settings-section page-enter" key="quota">
                            <div className="settings-heading">
                                <span className="settings-heading-icon settings-heading-blue"><Gauge size={19} /></span>
                                <div><h2>配额管理</h2><p>密钥默认额度与网关总额度</p></div>
                            </div>
                            <div className="settings-form-block">
                                <div className="form-grid">
                                    <label className="field-label">
                                        <span>默认密钥配额</span>
                                        <input
                                            className="field-input font-mono"
                                            type="number"
                                            min="0"
                                            max={MAX_QUOTA}
                                            step="1"
                                            value={draft.default_key_quota}
                                            onChange={(event) => updateSetting("default_key_quota", normalizeQuota(event.target.value))}
                                        />
                                        <small>新建密钥的默认 Token 上限，0 表示不限制</small>
                                    </label>
                                    <label className="field-label">
                                        <span>总配额</span>
                                        <input
                                            className="field-input font-mono"
                                            type="number"
                                            min="0"
                                            max={MAX_QUOTA}
                                            step="1"
                                            value={draft.total_quota}
                                            onChange={(event) => updateSetting("total_quota", normalizeQuota(event.target.value))}
                                        />
                                        <small>所有密钥累计 Token 上限，0 表示不限制</small>
                                    </label>
                                    <label className="field-label">
                                        <span>配额告警阈值</span>
                                        <div className="quota-threshold-field">
                                            <input
                                                className="field-input font-mono"
                                                type="number"
                                                min="1"
                                                max="100"
                                                step="1"
                                                value={draft.quota_warning_threshold}
                                                onChange={(event) => updateSetting(
                                                    "quota_warning_threshold",
                                                    Math.min(100, Math.max(1, Math.trunc(Number(event.target.value) || 1))),
                                                )}
                                            />
                                            <span>%</span>
                                        </div>
                                        <small>单个密钥或总配额达到该比例时显示告警</small>
                                    </label>
                                </div>
                            </div>
                            <div className="settings-note"><CircleHelp size={17} /><span>总配额和告警阈值保存后立即应用，默认密钥配额用于之后创建的密钥。</span></div>
                        </div>
                    ) : null}

                    {activeTab === "general" ? (
                        <div className="settings-section page-enter" key="general">
                            <div className="settings-heading">
                                <span className="settings-heading-icon settings-heading-blue"><Monitor size={19} /></span>
                                <div><h2>通用设置</h2><p>启动与后台行为</p></div>
                            </div>
                            <div className="setting-rows">
                                <div className="setting-row">
                                    <div><strong>开机自动启动</strong><span>登录系统后启动 CrowAPI</span></div>
                                    <Toggle checked={draft.auto_start} label="开机自动启动" onChange={(value) => updateSetting("auto_start", value)} />
                                </div>
                                <div className="setting-row">
                                    <div><strong>最小化到托盘</strong><span>最小化窗口时保留网关进程</span></div>
                                    <Toggle checked={draft.minimize_to_tray} label="最小化到托盘" onChange={(value) => updateSetting("minimize_to_tray", value)} />
                                </div>
                                <div className="setting-row">
                                    <div><strong>关闭到托盘</strong><span>关闭窗口时不停止服务</span></div>
                                    <Toggle checked={draft.close_to_tray} label="关闭到托盘" onChange={(value) => updateSetting("close_to_tray", value)} />
                                </div>
                            </div>
                            <div className="settings-heading mt-8">
                                <span className="settings-heading-icon settings-heading-amber"><ShieldCheck size={19} /></span>
                                <div><h2>请求安全</h2><p>敏感内容扫描与脱敏</p></div>
                            </div>
                            <div className="setting-rows">
                                <div className="setting-row">
                                    <div><strong>启用安全扫描</strong><span>检查请求中的工具、网络和 Unicode 风险</span></div>
                                    <Toggle checked={draft.security_enabled} label="启用安全扫描" onChange={(value) => updateSetting("security_enabled", value)} />
                                </div>
                                <div className="setting-row">
                                    <div><strong>自动脱敏密钥</strong><span>日志中隐藏检测到的凭据</span></div>
                                    <Toggle checked={draft.security_redact_secrets} label="自动脱敏密钥" onChange={(value) => updateSetting("security_redact_secrets", value)} />
                                </div>
                                <div className="setting-row">
                                    <div><strong>拦截严重风险</strong><span>阻止风险等级为 critical 的请求</span></div>
                                    <Toggle checked={draft.security_block_on_critical} label="拦截严重风险" onChange={(value) => updateSetting("security_block_on_critical", value)} />
                                </div>
                            </div>
                        </div>
                    ) : null}

                    {activeTab === "interface" ? (
                        <div className="settings-section page-enter" key="interface">
                            <div className="settings-heading">
                                <span className="settings-heading-icon settings-heading-coral"><Palette size={19} /></span>
                                <div><h2>界面设置</h2><p>外观与显示语言</p></div>
                            </div>
                            <div className="settings-form-block">
                                <div className="field-label">
                                    <span>主题</span>
                                </div>
                                <div className="theme-choice-grid" role="radiogroup" aria-label="界面主题">
                                    {themeOptions.map((option) => (
                                        <button
                                            key={option.value}
                                            type="button"
                                            role="radio"
                                            aria-checked={draft.ui_theme === option.value}
                                            className={`theme-choice theme-choice-${option.value} ${draft.ui_theme === option.value ? "is-selected" : ""}`}
                                            onClick={() => previewTheme(option.value)}
                                        >
                                            <span className="theme-choice-preview" aria-hidden="true">
                                                <i /><i /><i />
                                            </span>
                                            <span className="theme-choice-footer">
                                                <span className="theme-swatches" aria-hidden="true">
                                                    {option.colors.map((color) => <i key={color} style={{ background: color }} />)}
                                                </span>
                                                <strong>{option.label}</strong>
                                                {draft.ui_theme === option.value ? <Check size={14} /> : null}
                                            </span>
                                        </button>
                                    ))}
                                </div>
                                <label className="field-label mt-6 max-w-sm">
                                    <span><Globe2 size={14} />显示语言</span>
                                    <select className="field-input" value={draft.ui_language} onChange={(event) => updateSetting("ui_language", event.target.value)}>
                                        <option value="zh-CN">简体中文</option>
                                        <option value="en-US">English</option>
                                    </select>
                                </label>
                            </div>
                        </div>
                    ) : null}

                    {activeTab === "retry" ? (
                        <div className="settings-section page-enter" key="retry">
                            <div className="settings-heading">
                                <span className="settings-heading-icon settings-heading-blue"><RefreshCcw size={19} /></span>
                                <div><h2>重试策略</h2><p>上游失败后的请求恢复</p></div>
                            </div>
                            <div className="setting-rows">
                                <div className="setting-row">
                                    <div><strong>启用自动重试</strong><span>上游超时或返回可重试状态时切换线路</span></div>
                                    <Toggle checked={draft.retry_enabled} label="启用自动重试" onChange={(value) => updateSetting("retry_enabled", value)} />
                                </div>
                            </div>
                            <div className={`settings-form-block mt-6 ${draft.retry_enabled ? "" : "is-disabled"}`}>
                                <label className="field-label max-w-sm">
                                    <span>最大重试次数</span>
                                    <div className="stepper-field">
                                        <button type="button" aria-label="减少重试次数" disabled={!draft.retry_enabled || draft.retry_times <= 0} onClick={() => updateSetting("retry_times", Math.max(0, draft.retry_times - 1))}>−</button>
                                        <output>{draft.retry_times}</output>
                                        <button type="button" aria-label="增加重试次数" disabled={!draft.retry_enabled || draft.retry_times >= 5} onClick={() => updateSetting("retry_times", Math.min(5, draft.retry_times + 1))}>+</button>
                                    </div>
                                </label>
                                <div className="retry-sequence" aria-label={`最多重试 ${draft.retry_times} 次`}>
                                    <span className="is-primary">首次请求</span>
                                    {Array.from({ length: draft.retry_times }, (_, index) => (
                                        <span key={index}>重试 {index + 1}<small>{1.2 * (index + 1)}s</small></span>
                                    ))}
                                </div>
                                <div className="mt-6 grid gap-3 sm:grid-cols-2">
                                    <div className="retry-condition"><Check size={15} />HTTP 429 / 5xx</div>
                                    <div className="retry-condition"><Check size={15} />连接超时或中断</div>
                                </div>
                            </div>
                        </div>
                    ) : null}

                    {activeTab === "update" ? (
                        <div className="settings-section page-enter" key="update">
                            <div className="settings-heading">
                                <span className="settings-heading-icon settings-heading-blue"><DownloadCloud size={19} /></span>
                                <div><h2>软件更新</h2><p>检查并安装 CrowAPI 新版本</p></div>
                            </div>
                            <div className="update-status-panel">
                                <div className="update-version-row">
                                    <div><span>当前版本</span><strong>v{currentVersion}</strong></div>
                                    <div><span>最新版本</span><strong>{latestVersion ? `v${latestVersion}` : "--"}</strong></div>
                                    <button
                                        type="button"
                                        className="button-primary"
                                        disabled={["checking", "downloading", "installing"].includes(updatePhase)}
                                        onClick={() => void checkAndInstallUpdate()}
                                    >
                                        {updatePhase === "checking" ? <span className="button-spinner is-inverse" /> : <RefreshCcw size={16} />}
                                        {updatePhase === "checking" ? "正在检查" : "检查更新"}
                                    </button>
                                </div>
                                {updatePhase === "downloading" || updatePhase === "installing" ? (
                                    <div className="update-progress" role="status" aria-live="polite">
                                        <div><span>{updatePhase === "downloading" ? "正在下载更新" : "正在安装更新"}</span><strong>{updateProgress}%</strong></div>
                                        <div><span style={{ width: `${updateProgress}%` }} /></div>
                                    </div>
                                ) : null}
                                {updatePhase === "current" ? <p className="update-message is-success"><Check size={15} />当前已是最新版本</p> : null}
                                {updatePhase === "error" ? <p className="update-message is-error"><CircleHelp size={15} />{updateError}</p> : null}
                            </div>
                            <div className="settings-note"><CircleHelp size={17} /><span>发现新版本后会直接下载并安装，完成后自动重启应用。</span></div>
                        </div>
                    ) : null}
                </section>
            </div>
            {error || saveMutation.error ? (
                <p className="form-error mt-4" role="alert">{errorMessage(error ?? saveMutation.error)}</p>
            ) : null}
            {toast ? <Toast message={toast} /> : null}
        </div>
    );
}
