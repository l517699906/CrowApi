import type { ApiAccessScope } from "../types";

export const ACCESS_SCOPE_OPTIONS: ReadonlyArray<{
    value: ApiAccessScope;
    label: string;
    description: string;
}> = [
    { value: "gateway", label: "模型网关", description: "调用 /v1 模型兼容接口" },
    { value: "mcp:read", label: "MCP 只读", description: "查询知识库、Wiki 和任务状态" },
    { value: "mcp:write", label: "MCP 读写", description: "创建、导入、修改和删除内容" },
    { value: "admin", label: "管理接口", description: "调用 /api 管理端点" },
];

export const ACCESS_SCOPE_LABELS = Object.fromEntries(
    ACCESS_SCOPE_OPTIONS.map((option) => [option.value, option.label]),
) as Record<ApiAccessScope, string>;

export function updateAccessScopes(
    current: ApiAccessScope[],
    scope: ApiAccessScope,
    checked: boolean,
): ApiAccessScope[] {
    const next = new Set(current);
    if (checked) {
        next.add(scope);
        if (scope === "mcp:write") next.add("mcp:read");
    } else {
        next.delete(scope);
        if (scope === "mcp:read") next.delete("mcp:write");
    }
    return ACCESS_SCOPE_OPTIONS
        .map((option) => option.value)
        .filter((value) => next.has(value));
}
