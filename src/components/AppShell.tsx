import { useEffect, useMemo, useState } from "react";
import {
    BarChart3,
    Bell,
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
import { useGatewayStore } from "../store/gatewayStore";
import { ApiKeysPage } from "../pages/ApiKeysPage";
import { ChannelsPage } from "../pages/ChannelsPage";
import { DashboardPage } from "../pages/DashboardPage";
import { LogsPage } from "../pages/LogsPage";
import { SettingsPage } from "../pages/SettingsPage";
import { UsagePage } from "../pages/UsagePage";
import { IconButton } from "./ui";

const navItems = [
    { to: "/dashboard", label: "仪表盘", icon: LayoutDashboard },
    { to: "/usage", label: "用量", icon: BarChart3 },
    { to: "/channels", label: "渠道", icon: Radio },
    { to: "/keys", label: "密钥", icon: KeyRound },
    { to: "/logs", label: "日志", icon: ScrollText },
    { to: "/settings", label: "设置", icon: Settings2 },
] as const;

const pageNames = new Map<string, string>(navItems.map((item) => [item.to, item.label]));

export function AppShell() {
    const location = useLocation();
    const settings = useGatewayStore((state) => state.settings);
    const channels = useGatewayStore((state) => state.channels);
    const [sidebarOpen, setSidebarOpen] = useState(false);
    const [endpointCopied, setEndpointCopied] = useState(false);
    const pageName = pageNames.get(location.pathname) ?? "仪表盘";
    const endpoint = `http://${settings.server_host}:${settings.server_port}/v1`;
    const activeChannels = useMemo(
        () => channels.filter((channel) => channel.status === 1).length,
        [channels],
    );

    useEffect(() => {
        document.documentElement.dataset.theme = settings.ui_theme;
    }, [settings.ui_theme]);

    useEffect(() => {
        setSidebarOpen(false);
    }, [location.pathname]);

    const copyEndpoint = async () => {
        try {
            await navigator.clipboard.writeText(endpoint);
            setEndpointCopied(true);
            window.setTimeout(() => setEndpointCopied(false), 1600);
        } catch {
            setEndpointCopied(false);
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
            <aside className={`sidebar ${sidebarOpen ? "is-open" : ""}`}>
                <div className="sidebar-brand">
                    <div className="brand-mark" aria-hidden="true">
                        <span />
                        <span />
                        <span />
                    </div>
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
                            <span className="live-dot" />
                            网关运行中
                        </span>
                        <span className="font-mono text-[10px] text-sidebar-muted">v0.1.0</span>
                    </div>
                    <div className="mt-3 flex items-end justify-between">
                        <div>
                            <div className="font-mono text-xl font-semibold text-sidebar-ink">{activeChannels}/{channels.length}</div>
                            <div className="text-[11px] text-sidebar-muted">渠道在线</div>
                        </div>
                        <div className="signal-bars" aria-label="网关信号正常">
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
                        <IconButton label="打开导航" className="mobile-menu-button" onClick={() => setSidebarOpen(true)}>
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
                            <span className="live-dot" />
                            <span className="endpoint-text">{endpoint}</span>
                            {endpointCopied ? <Check size={14} /> : <Copy size={14} />}
                        </button>
                        <IconButton label="通知">
                            <Bell size={18} />
                            <span className="notification-dot" />
                        </IconButton>
                        <div className="user-avatar" aria-label="本地管理员">WA</div>
                    </div>
                </header>

                <main className="app-content">
                    <Routes>
                        <Route path="/dashboard" element={<DashboardPage />} />
                        <Route path="/usage" element={<UsagePage />} />
                        <Route path="/channels" element={<ChannelsPage />} />
                        <Route path="/keys" element={<ApiKeysPage />} />
                        <Route path="/logs" element={<LogsPage />} />
                        <Route path="/settings" element={<SettingsPage />} />
                        <Route path="*" element={<Navigate to="/dashboard" replace />} />
                    </Routes>
                </main>
            </div>
        </div>
    );
}
