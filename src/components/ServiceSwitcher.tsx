import { BookOpen, Network, Terminal } from "lucide-react";
import { useLocation, useNavigate } from "react-router-dom";

const serviceViews = [
    { path: "/services", label: "知识库", icon: BookOpen },
    { path: "/services/wiki", label: "Wiki", icon: Network },
    { path: "/services/mcp", label: "MCP", icon: Terminal },
] as const;

function activeServicePath(pathname: string) {
    if (pathname.startsWith("/services/wiki")) return "/services/wiki";
    if (pathname.startsWith("/services/mcp")) return "/services/mcp";
    return "/services";
}

export function ServiceSwitcher() {
    const location = useLocation();
    const navigate = useNavigate();
    const activePath = activeServicePath(location.pathname);

    return (
        <div className="kb-service-switcher" role="tablist" aria-label="知识服务视图">
            {serviceViews.map(({ path, label, icon: Icon }) => (
                <button
                    key={path}
                    type="button"
                    role="tab"
                    aria-selected={activePath === path}
                    className={activePath === path ? "is-active" : ""}
                    onClick={() => navigate(path)}
                >
                    <Icon size={16} />
                    {label}
                </button>
            ))}
        </div>
    );
}
