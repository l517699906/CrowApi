import { useMemo, useState } from "react";
import { CircleStop, ListTodo, RefreshCcw, RotateCcw, TriangleAlert } from "lucide-react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { PageTitle, StatusBadge, Toast } from "../components/ui";
import { useBackgroundTasks } from "../hooks/useBackgroundTasks";
import { taskApi } from "../lib/api";
import { errorMessage, queryKeys } from "../lib/query";
import type { BackgroundTask, BackgroundTaskStatus } from "../types";

type TaskStatusFilter = "all" | BackgroundTaskStatus;

const statusOptions: ReadonlyArray<{ value: TaskStatusFilter; label: string }> = [
    { value: "all", label: "全部" },
    { value: "pending", label: "排队中" },
    { value: "running", label: "运行中" },
    { value: "succeeded", label: "已完成" },
    { value: "failed", label: "失败" },
    { value: "cancelled", label: "已取消" },
    { value: "interrupted", label: "已中断" },
];

function statusTone(status: BackgroundTaskStatus): "success" | "warning" | "danger" | "neutral" | "info" {
    if (status === "succeeded") return "success";
    if (status === "failed") return "danger";
    if (status === "running") return "info";
    if (status === "pending" || status === "interrupted") return "warning";
    return "neutral";
}

function statusLabel(status: BackgroundTaskStatus) {
    return statusOptions.find((option) => option.value === status)?.label ?? status;
}

function taskLabel(task: BackgroundTask) {
    const labels: Record<string, string> = {
        knowledge: "知识库",
        wiki: "Wiki",
        maintenance: "维护",
    };
    return labels[task.domain] ?? task.domain;
}

function formatTime(value: string | null) {
    if (!value) return "-";
    const date = new Date(value);
    return Number.isNaN(date.getTime()) ? value : date.toLocaleString("zh-CN", { hour12: false });
}

function isActive(status: BackgroundTaskStatus) {
    return status === "pending" || status === "running";
}

export function TasksPage() {
    const queryClient = useQueryClient();
    const [status, setStatus] = useState<TaskStatusFilter>("all");
    const [selectedId, setSelectedId] = useState<string | null>(null);
    const [toast, setToast] = useState("");
    const filter = useMemo(
        () => ({ limit: 200, ...(status === "all" ? {} : { status }) }),
        [status],
    );
    const tasksQuery = useBackgroundTasks(filter);
    const cancelMutation = useMutation({
        mutationFn: taskApi.cancel,
        onSuccess: () => {
            void queryClient.invalidateQueries({ queryKey: queryKeys.backgroundTasksRoot });
            setToast("已请求取消任务");
        },
    });
    const retryMutation = useMutation({
        mutationFn: taskApi.retry,
        onSuccess: () => {
            void queryClient.invalidateQueries({ queryKey: queryKeys.backgroundTasksRoot });
            setToast("任务已重新启动");
        },
    });
    const tasks = tasksQuery.data ?? [];
    const selected = tasks.find((task) => task.id === selectedId) ?? null;
    const activeCount = tasks.filter((task) => isActive(task.status)).length;

    function runAction(action: () => Promise<unknown>, successMessage: string) {
        void action()
            .then(() => setToast(successMessage))
            .catch((error) => setToast(errorMessage(error)));
    }

    return (
        <div className="page-enter">
            <PageTitle
                title="任务中心"
                meta={`${tasks.length} 条任务记录${activeCount ? ` · ${activeCount} 条进行中` : ""}`}
                action={(
                    <button
                        type="button"
                        className="button-secondary"
                        onClick={() => void tasksQuery.refetch()}
                        disabled={tasksQuery.isFetching}
                    >
                        <RefreshCcw size={15} className={tasksQuery.isFetching ? "animate-spin" : ""} />
                        刷新
                    </button>
                )}
            />

            <div className="mb-5 flex flex-wrap items-center gap-2" role="tablist" aria-label="任务状态筛选">
                {statusOptions.map((option) => (
                    <button
                        key={option.value}
                        type="button"
                        role="tab"
                        aria-selected={status === option.value}
                        className={`filter-chip ${status === option.value ? "is-active" : ""}`}
                        onClick={() => setStatus(option.value)}
                    >
                        {option.label}
                    </button>
                ))}
            </div>

            {tasksQuery.isPending ? (
                <div className="surface empty-state px-6 py-16"><span className="button-spinner" /><strong>正在读取后台任务</strong></div>
            ) : tasksQuery.error ? (
                <div className="surface empty-state px-6 py-16"><TriangleAlert size={22} /><strong>任务读取失败</strong><span>{errorMessage(tasksQuery.error)}</span></div>
            ) : tasks.length === 0 ? (
                <div className="surface empty-state px-6 py-16"><ListTodo size={24} /><strong>暂无任务记录</strong><span>知识库和 Wiki 的导入、索引任务会显示在这里。</span></div>
            ) : (
                <div className="surface table-scroll">
                    <table className="data-table min-w-[980px]">
                        <thead>
                            <tr>
                                <th>任务</th>
                                <th>资源</th>
                                <th>状态</th>
                                <th>进度</th>
                                <th>尝试</th>
                                <th>更新时间</th>
                                <th aria-label="操作" />
                            </tr>
                        </thead>
                        <tbody>
                            {tasks.map((task) => (
                                <tr key={task.id} className={selectedId === task.id ? "is-selected" : ""}>
                                    <td>
                                        <button type="button" className="text-left" onClick={() => setSelectedId((id) => id === task.id ? null : task.id)}>
                                            <strong className="block text-ink">{taskLabel(task)} · {task.taskType}</strong>
                                            <code className="mt-1 block text-xs text-muted">{task.id.slice(0, 12)}</code>
                                        </button>
                                    </td>
                                    <td>
                                        <span className="block text-ink">{task.resourceType}</span>
                                        <code className="text-xs text-muted">{task.resourceId.slice(0, 18)}</code>
                                    </td>
                                    <td><StatusBadge status={statusTone(task.status)} dot>{statusLabel(task.status)}</StatusBadge></td>
                                    <td>
                                        <div className="min-w-32">
                                            <div className="mb-1 flex justify-between text-xs text-muted"><span>{task.stage}</span><span>{Math.round(task.progress)}%</span></div>
                                            <div className="h-1.5 overflow-hidden rounded-full bg-soft"><div className="h-full rounded-full bg-accent transition-all" style={{ width: `${Math.max(0, Math.min(100, task.progress))}%` }} /></div>
                                        </div>
                                    </td>
                                    <td className="font-mono text-xs">{task.attempt}/{task.maxAttempts}</td>
                                    <td className="whitespace-nowrap text-xs text-muted">{formatTime(task.updatedAt)}</td>
                                    <td>
                                        <div className="flex justify-end gap-1">
                                            {isActive(task.status) ? (
                                                <button type="button" className="button-secondary button-compact" onClick={() => runAction(() => cancelMutation.mutateAsync(task.id), "已请求取消任务")} disabled={cancelMutation.isPending}>
                                                    <CircleStop size={14} />取消
                                                </button>
                                            ) : null}
                                            {task.retryable === 1 && ["failed", "cancelled", "interrupted"].includes(task.status) ? (
                                                <button type="button" className="button-secondary button-compact" onClick={() => runAction(() => retryMutation.mutateAsync(task.id), "任务已重新启动")} disabled={retryMutation.isPending}>
                                                    <RotateCcw size={14} />重试
                                                </button>
                                            ) : null}
                                        </div>
                                    </td>
                                </tr>
                            ))}
                        </tbody>
                    </table>
                </div>
            )}

            {selected ? (
                <section className="surface mt-4 p-5" aria-label="任务详情">
                    <div className="mb-3 flex flex-wrap items-start justify-between gap-3">
                        <div><h2 className="text-base font-semibold text-ink">任务详情</h2><p className="mt-1 text-xs text-muted">{selected.id}</p></div>
                        <StatusBadge status={statusTone(selected.status)}>{statusLabel(selected.status)}</StatusBadge>
                    </div>
                    <div className="grid gap-3 text-sm sm:grid-cols-2 lg:grid-cols-4">
                        <div><span className="block text-xs text-muted">创建时间</span><span>{formatTime(selected.createdAt)}</span></div>
                        <div><span className="block text-xs text-muted">开始时间</span><span>{formatTime(selected.startedAt)}</span></div>
                        <div><span className="block text-xs text-muted">完成时间</span><span>{formatTime(selected.completedAt)}</span></div>
                        <div><span className="block text-xs text-muted">父任务</span><code>{selected.parentTaskId ?? "-"}</code></div>
                    </div>
                    {selected.errorMessage ? <p className="mt-4 rounded-md bg-danger-soft px-3 py-2 text-sm text-danger">{selected.errorMessage}</p> : null}
                </section>
            ) : null}

            {toast ? <Toast message={toast} /> : null}
        </div>
    );
}
