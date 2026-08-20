export const SERVICE_VIEWS = [
    { id: "knowledge", path: "/services", label: "知识库", pageName: "知识库" },
    { id: "wiki", path: "/services/wiki", label: "Wiki", pageName: "Wiki" },
    { id: "mcp", path: "/services/mcp", label: "MCP", pageName: "MCP 服务" },
] as const;

export type ServiceView = (typeof SERVICE_VIEWS)[number];
export type ServiceViewId = ServiceView["id"];

function matchesPath(pathname: string, path: string) {
    return pathname === path || pathname.startsWith(`${path}/`);
}

export function serviceViewForPath(pathname: string): ServiceView | undefined {
    const nestedView = SERVICE_VIEWS.slice(1).find((view) => matchesPath(pathname, view.path));
    if (nestedView) return nestedView;

    const rootView = SERVICE_VIEWS[0];
    return matchesPath(pathname, rootView.path) ? rootView : undefined;
}
