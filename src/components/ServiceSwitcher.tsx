import { BookOpen, Network, Terminal, type LucideIcon } from "lucide-react";
import { useLocation, useNavigate } from "react-router-dom";
import { SERVICE_VIEWS, serviceViewForPath, type ServiceViewId } from "../config/serviceViews";

const serviceIcons: Record<ServiceViewId, LucideIcon> = {
    knowledge: BookOpen,
    wiki: Network,
    mcp: Terminal,
};

export function ServiceSwitcher() {
    const location = useLocation();
    const navigate = useNavigate();
    const activePath = serviceViewForPath(location.pathname)?.path ?? SERVICE_VIEWS[0].path;

    return (
        <div className="kb-service-switcher" role="tablist" aria-label="知识服务视图">
            {SERVICE_VIEWS.map(({ id, path, label }) => {
                const Icon = serviceIcons[id];
                return (
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
                );
            })}
        </div>
    );
}
