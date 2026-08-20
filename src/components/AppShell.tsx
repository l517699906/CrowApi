import { useEffect, useMemo, useState } from "react";
import {
    BarChart3,
    Bell,
    Database,
    Check,
    ChevronRight,
    Command,
    Copy,
    KeyRound,
    LayoutDashboard,
    Menu,
    Radio,
    ScrollText,
    Settings2,
    X,
} from "lucide-react";
import { NavLink, Navigate, Route, Routes, useLocation } from "react-router-dom";
import { useQuery } from "@tanstack/react-query";
import { DEFAULT_SETTINGS } from "../config/defaults";
import { channelApi, serverApi, settingsApi } from "../lib/api";
import { queryKeys } from "../lib/query";
import { applyUiTheme, DARK_THEME_MEDIA_QUERY } from "../lib/theme";
import { ApiKeysPage } from "../pages/ApiKeysPage";
import { ChannelsPage } from "../pages/ChannelsPage";
import { DashboardPage } from "../pages/DashboardPage";
import { LogsPage } from "../pages/LogsPage";
import { SettingsPage } from "../pages/SettingsPage";
import { UsagePage } from "../pages/UsagePage";
import { KnowledgeBasePage } from "../pages/KnowledgeBasePage";
import { WikiPage } from "../pages/WikiPage";
import { IconButton, Toast } from "./ui";

const navItems = [
    { to: "/dashboard", label: "仪表盘", icon: LayoutDashboard },
    { to: "/usage", label: "用量", icon: BarChart3 },
    { to: "/channels", label: "渠道", icon: Radio },
    { to: "/services", label: "知识库", icon: Database },
    { to: "/keys", label: "密钥", icon: KeyRound },
    { to: "/logs", label: "日志", icon: ScrollText },
    { to: "/settings", label: "设置", icon: Settings2 },
] as const;

const pageNames = new Map<string, string>(navItems.map((item) => [item.to, item.label]));

function resolvePageName(pathname: string) {
    if (pathname.startsWith("/services/wiki")) return "Wiki";
    if (pathname.startsWith("/services/mcp")) return "MCP 服务";
    if (pathname.startsWith("/services")) return "知识库";
    return pageNames.get(pathname) ?? "仪表盘";
}

export function AppShell() {
    const location = useLocation();
    const { data: settings = DEFAULT_SETTINGS } = useQuery({
        queryKey: queryKeys.settings,
        queryFn: settingsApi.get,
    });
    const { data: channels = [] } = useQuery({
        queryKey: queryKeys.channels,
        queryFn: channelApi.getAll,
    });
    const { data: serverStatus } = useQuery({
        queryKey: queryKeys.serverStatus,
        queryFn: serverApi.getStatus,
        refetchInterval: 2_000,
    });
    const [sidebarOpen, setSidebarOpen] = useState(false);
    const [endpointCopied, setEndpointCopied] = useState(false);
    const [notificationOpen, setNotificationOpen] = useState(false);
    const [toastMessage, setToastMessage] = useState<string | null>(null);
    const pageName = resolvePageName(location.pathname);
    const isRunning = serverStatus?.running ?? false;
    const endpointBase = serverStatus?.port ? serverStatus.url : `http://${settings.server_host}:${settings.server_port}`;
    const endpoint = `${endpointBase}/v1`;
    const activeChannels = useMemo(
        () => channels.filter((channel) => channel.status === 1).length,
        [channels],
    );

    useEffect(() => {
        const media = window.matchMedia(DARK_THEME_MEDIA_QUERY);
        const applyTheme = () => applyUiTheme(settings.ui_theme, media.matches);
        applyTheme();
        media.addEventListener("change", applyTheme);
        return () => media.removeEventListener("change", applyTheme);
    }, [settings.ui_theme]);

    useEffect(() => {
        try {
            window.localStorage.removeItem("crowapi.console.v1");
        } catch {
            // WebView storage may be unavailable; no runtime data depends on it anymore.
        }
    }, []);

    useEffect(() => {
        setSidebarOpen(false);
        setNotificationOpen(false);
    }, [location.pathname]);

    useEffect(() => {
        if (!toastMessage) return;
        const timer = window.setTimeout(() => setToastMessage(null), 2200);
        return () => window.clearTimeout(timer);
    }, [toastMessage]);

    const copyEndpoint = async () => {
        try {
            await navigator.clipboard.writeText(endpoint);
            setEndpointCopied(true);
            setToastMessage("API 地址已复制");
            window.setTimeout(() => setEndpointCopied(false), 1600);
        } catch {
            setEndpointCopied(false);
            setToastMessage("当前环境无法访问剪贴板");
        }
    };

    return (
        <div className="app-shell">
            <button
                type="button"
                className={`sidebar-scrim ${sidebarOpen ? "is-visible" : ""}`}
                aria-label="关闭导航"
                onClick={() => setSidebarOpen(false)}
            />
            <aside id="app-sidebar" className={`sidebar ${sidebarOpen ? "is-open" : ""}`}>
                <div className="sidebar-brand">
                    <img
                        className="brand-mark"
                        src="/crowapi-logo.png"
                        alt=""
                        aria-hidden="true"
                        draggable={false}
                    />
                    <div>
                        <div className="font-display text-[17px] font-bold leading-5 text-sidebar-ink">CrowAPI</div>
                        <div className="mt-0.5 font-mono text-[10px] uppercase text-sidebar-muted">Local gateway</div>
                    </div>
                    <IconButton label="关闭导航" className="sidebar-close ml-auto" onClick={() => setSidebarOpen(false)}>
                        <X size={18} />
                    </IconButton>
                </div>

                <nav className="sidebar-nav" aria-label="主导航">
                    <p className="sidebar-section-label">工作台</p>
                    {navItems.map((item) => {
                        const Icon = item.icon;
                        return (
                            <NavLink
                                key={item.to}
                                to={item.to}
                                className={({ isActive }) => `nav-item ${isActive ? "is-active" : ""}`}
                            >
                                <Icon size={18} strokeWidth={1.8} />
                                <span>{item.label}</span>
                                <ChevronRight className="nav-chevron" size={15} />
                            </NavLink>
                        );
                    })}
                </nav>

                <div className="sidebar-runtime">
                    <div className="flex items-center justify-between gap-3">
                        <span className="flex items-center gap-2 text-xs font-medium text-sidebar-ink">
                            <span className={`live-dot ${isRunning ? "" : "is-offline"}`} />
                            {isRunning ? "网关运行中" : "网关未连接"}
                        </span>
                        <span className="font-mono text-[10px] text-sidebar-muted">v0.1.5</span>
                    </div>
                    <div className="mt-3 flex items-end justify-between">
                        <div>
                            <div className="font-mono text-xl font-semibold text-sidebar-ink">{activeChannels}/{channels.length}</div>
                            <div className="text-[11px] text-sidebar-muted">渠道在线</div>
                        </div>
                        <div className="signal-bars" aria-label={isRunning ? "网关运行正常" : "网关未连接"}>
                            {[42, 63, 52, 78, 66, 91, 72, 82].map((height, index) => (
                                <span key={`${height}-${index}`} style={{ height: `${height}%` }} />
                            ))}
                        </div>
                    </div>
                </div>
            </aside>

            <div className="app-workspace">
                <header className="topbar">
                    <div className="flex min-w-0 items-center gap-3">
                        <IconButton label="打开导航" aria-expanded={sidebarOpen} aria-controls="app-sidebar" className="mobile-menu-button" onClick={() => setSidebarOpen(true)}>
                            <Menu size={19} />
                        </IconButton>
                        <div className="hidden items-center gap-2 text-sm text-muted sm:flex">
                            <Command size={15} />
                            <span>CrowAPI</span>
                            <ChevronRight size={14} />
                        </div>
                        <strong className="truncate text-sm font-semibold text-ink">{pageName}</strong>
                    </div>

                    <div className="flex min-w-0 items-center gap-2">
                        <button
                            type="button"
                            className="endpoint-chip"
                            title="复制 API 地址"
                            onClick={copyEndpoint}
                        >
                            <span className={`live-dot ${isRunning ? "" : "is-offline"}`} />
                            <span className="endpoint-text">{endpoint}</span>
                            {endpointCopied ? <Check size={14} /> : <Copy size={14} />}
                        </button>
                        <div className="relative">
                            <IconButton label="通知" aria-expanded={notificationOpen} onClick={() => setNotificationOpen((open) => !open)}>
                                <Bell size={18} />
                                {!isRunning ? <span className="notification-dot" /> : null}
                            </IconButton>
                            {notificationOpen ? (
                                <div className="notification-popover" role="dialog" aria-label="运行状态通知">
                                    <strong>{isRunning ? "网关运行正常" : "网关未连接"}</strong>
                                    <p>{isRunning ? `${activeChannels} 个渠道处于启用状态。` : "启动服务后才能接收本地 API 请求。"}</p>
                                    <NavLink to={isRunning ? "/logs" : "/settings"} onClick={() => setNotificationOpen(false)}>
                                        {isRunning ? "查看请求日志" : "打开服务设置"}
                                    </NavLink>
                                </div>
                            ) : null}
                        </div>
                        <div className="user-avatar" aria-label="本地管理员">Crow</div>
                    </div>
                </header>

                <main className="app-content">
                    <Routes>
                        <Route path="/dashboard" element={<DashboardPage />} />
                        <Route path="/usage" element={<UsagePage />} />
                        <Route path="/channels" element={<ChannelsPage />} />
                        <Route path="/services/wiki" element={<WikiPage />} />
                        <Route path="/services/*" element={<KnowledgeBasePage />} />
                        <Route path="/keys" element={<ApiKeysPage />} />
                        <Route path="/logs" element={<LogsPage />} />
                        <Route path="/settings" element={<SettingsPage />} />
                        <Route path="*" element={<Navigate to="/dashboard" replace />} />
                    </Routes>
                </main>
            </div>
            {toastMessage ? <Toast message={toastMessage} /> : null}
        </div>
    );
}
