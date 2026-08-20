import { useEffect, useRef } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export function useTauriEvent<T>(eventName: string, handler: (payload: T) => void) {
    const handlerRef = useRef(handler);

    useEffect(() => {
        handlerRef.current = handler;
    }, [handler]);

    useEffect(() => {
        let disposed = false;
        let unlisten: UnlistenFn | undefined;

        void listen<T>(eventName, ({ payload }) => handlerRef.current(payload))
            .then((cleanup) => {
                if (disposed) {
                    cleanup();
                } else {
                    unlisten = cleanup;
                }
            })
            .catch(() => {
                // Browser preview has no Tauri event bridge; query polling remains available.
            });

        return () => {
            disposed = true;
            unlisten?.();
        };
    }, [eventName]);
}
