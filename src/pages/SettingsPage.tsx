import { useState } from "react";
import {
    Check,
    CircleHelp,
    Globe2,
    Monitor,
    Palette,
    RefreshCcw,
    Save,
    Server,
    Settings2,
    ShieldCheck,
} from "lucide-react";
import { useGatewayStore } from "../store/gatewayStore";
import type { Settings } from "../types";
import { PageTitle, SegmentedControl, Toast, Toggle } from "../components/ui";

type SettingsTab = "service" | "general" | "interface" | "retry";

const tabs: Array<{ id: SettingsTab; label: string; icon: typeof Server }> = [
    { id: "service", label: "服务配置", icon: Server },
    { id: "general", label: "通用设置", icon: Settings2 },
    { id: "interface", label: "界面设置", icon: Palette },
    { id: "retry", label: "重试策略", icon: RefreshCcw },
];

const themeOptions = [
    { value: "light", label: "浅色" },
    { value: "system", label: "跟随系统" },
    { value: "dark", label: "深色" },
] as const;

export function SettingsPage() {
    const settings = useGatewayStore((state) => state.settings);
    const saveSettings = useGatewayStore((state) => state.saveSettings);
    const [activeTab, setActiveTab] = useState<SettingsTab>("service");
    const [draft, setDraft] = useState<Settings>(() => ({ ...settings }));
    const [toast, setToast] = useState("");

    function updateSetting<K extends keyof Settings>(key: K, value: Settings[K]) {
        setDraft((current) => ({ ...current, [key]: value }));
    }

    const save = () => {
        saveSettings(draft);
        setToast("设置已保存");
        window.setTimeout(() => setToast(""), 1800);
    };

    return (
        <div className="page-enter">
            <PageTitle
                title="设置"
                meta="本地网关配置"
                action={(
                    <button type="button" className="button-primary" onClick={save}>
                        <Save size={16} />保存更改
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
                                    <SegmentedControl
                                        value={draft.ui_theme as "light" | "system" | "dark"}
                                        options={themeOptions}
                                        onChange={(value) => updateSetting("ui_theme", value)}
                                        label="界面主题"
                                    />
                                </div>
                                <div className="theme-previews" aria-hidden="true">
                                    {themeOptions.map((option) => (
                                        <button
                                            key={option.value}
                                            type="button"
                                            tabIndex={-1}
                                            className={`theme-preview theme-preview-${option.value} ${draft.ui_theme === option.value ? "is-selected" : ""}`}
                                            onClick={() => updateSetting("ui_theme", option.value)}
                                        >
                                            <span className="theme-preview-sidebar" />
                                            <span className="theme-preview-content"><i /><i /><i /></span>
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
                </section>
            </div>
            {toast ? <Toast message={toast} /> : null}
        </div>
    );
}
