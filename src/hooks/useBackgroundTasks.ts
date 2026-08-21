import { useQuery, useQueryClient } from "@tanstack/react-query";
import { taskApi } from "../lib/api";
import { queryKeys } from "../lib/query";
import type {
    BackgroundTaskEvent,
    BackgroundTaskFilter,
} from "../types";
import { useTauriEvent } from "./useTauriEvent";

const ACTIVE_STATUSES = new Set(["pending", "running"]);

export function useBackgroundTasks(filter: BackgroundTaskFilter, enabled = true) {
    const queryClient = useQueryClient();
    const domain = filter.domain;
    const resourceType = filter.resourceType;
    const resourceId = filter.resourceId;
    const status = filter.status;
    const limit = filter.limit;
    const queryKey = queryKeys.backgroundTasks(
        domain,
        resourceType,
        resourceId,
        status,
        limit ?? "*",
    );
    const query = useQuery({
        queryKey,
        queryFn: () => taskApi.getAll({
            domain,
            resourceType,
            resourceId,
            status,
            limit,
        }),
        enabled,
        refetchInterval: ({ state }) => (
            state.data?.some((task) => ACTIVE_STATUSES.has(task.status)) ? 3_000 : false
        ),
    });

    useTauriEvent<BackgroundTaskEvent>("background-task-progress", (event) => {
        if (domain && event.domain !== domain) return;
        if (resourceType && event.resourceType !== resourceType) return;
        if (resourceId && event.resourceId !== resourceId) return;
        if (status && event.status !== status) return;
        void queryClient.invalidateQueries({ queryKey });
    });

    return query;
}
