import { create } from "zustand";
import { persist } from "zustand/middleware";
import { initialApiKeys, initialChannels, initialLogs, initialSettings } from "../data/mockData";
import type {
    ApiKey,
    Channel,
    CreateApiKeyInput,
    CreateChannelInput,
    Settings,
} from "../types";

interface GatewayState {
    channels: Channel[];
    apiKeys: ApiKey[];
    settings: Settings;
    logs: typeof initialLogs;
    addChannel: (input: CreateChannelInput) => void;
    updateChannel: (id: string, input: CreateChannelInput) => void;
    deleteChannel: (id: string) => void;
    toggleChannel: (id: string) => void;
    recordChannelTest: (id: string, ok: boolean) => void;
    createApiKey: (input: CreateApiKeyInput) => ApiKey;
    toggleApiKey: (id: string) => void;
    deleteApiKey: (id: string) => void;
    saveSettings: (settings: Settings) => void;
}

function createId(prefix: string): string {
    const randomPart = crypto.randomUUID?.() ?? Math.random().toString(36).slice(2);
    return `${prefix}-${randomPart}`;
}

function createCrowApiKey(): string {
    const bytes = crypto.getRandomValues(new Uint8Array(18));
    const suffix = Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
    return `sk-crowapi-${suffix}`;
}

export const useGatewayStore = create<GatewayState>()(
    persist(
        (set) => ({
            channels: initialChannels,
            apiKeys: initialApiKeys,
            logs: initialLogs,
            settings: initialSettings,
            addChannel: (input) => set((state) => {
                const now = new Date().toISOString();
                const channel: Channel = {
                    ...input,
                    id: createId("channel"),
                    status: 1,
                    config: {},
                    model_mapping: {},
                    created_at: now,
                    updated_at: now,
                    last_test_at: null,
                    last_test_ok: null,
                };

                return { channels: [...state.channels, channel] };
            }),
            updateChannel: (id, input) => set((state) => ({
                channels: state.channels.map((channel) => channel.id === id
                    ? { ...channel, ...input, updated_at: new Date().toISOString() }
                    : channel),
            })),
            deleteChannel: (id) => set((state) => ({
                channels: state.channels.filter((channel) => channel.id !== id),
            })),
            toggleChannel: (id) => set((state) => ({
                channels: state.channels.map((channel) => channel.id === id
                    ? { ...channel, status: channel.status === 1 ? 0 : 1, updated_at: new Date().toISOString() }
                    : channel),
            })),
            recordChannelTest: (id, ok) => set((state) => ({
                channels: state.channels.map((channel) => channel.id === id
                    ? { ...channel, last_test_at: new Date().toISOString(), last_test_ok: ok ? 1 : 0 }
                    : channel),
            })),
            createApiKey: (input) => {
                const now = new Date().toISOString();
                const apiKey: ApiKey = {
                    ...input,
                    id: createId("key"),
                    key: createCrowApiKey(),
                    status: 1,
                    quota_used: 0,
                    created_at: now,
                    updated_at: now,
                };

                set((state) => ({ apiKeys: [apiKey, ...state.apiKeys] }));
                return apiKey;
            },
            toggleApiKey: (id) => set((state) => ({
                apiKeys: state.apiKeys.map((apiKey) => apiKey.id === id
                    ? { ...apiKey, status: apiKey.status === 1 ? 0 : 1, updated_at: new Date().toISOString() }
                    : apiKey),
            })),
            deleteApiKey: (id) => set((state) => ({
                apiKeys: state.apiKeys.filter((apiKey) => apiKey.id !== id),
            })),
            saveSettings: (settings) => set({ settings }),
        }),
        {
            name: "crowapi.console.v1",
            version: 1,
            partialize: ({ channels, apiKeys, logs, settings }) => ({
                channels,
                apiKeys,
                logs,
                settings,
            }),
        },
    ),
);
