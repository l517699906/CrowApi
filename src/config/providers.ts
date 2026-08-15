import type { ChannelType } from "../types";

interface ProviderDefinition {
    value: ChannelType;
    label: string;
    baseUrl: string;
    models: string;
}

export const PROVIDERS: readonly ProviderDefinition[] = [
    { value: "openai", label: "OpenAI", baseUrl: "https://api.openai.com/v1", models: "gpt-5.4, gpt-5-mini" },
    { value: "deepseek", label: "DeepSeek", baseUrl: "https://api.deepseek.com/v1", models: "deepseek-chat, deepseek-reasoner" },
    { value: "claude", label: "Claude", baseUrl: "https://api.anthropic.com", models: "claude-sonnet-4-20250514, claude-3-5-haiku-20241022" },
    { value: "gemini", label: "Gemini", baseUrl: "https://generativelanguage.googleapis.com", models: "gemini-2.5-pro, gemini-2.5-flash" },
    { value: "custom", label: "Custom", baseUrl: "http://127.0.0.1:11434/v1", models: "" },
] as const;

export const PROVIDER_DEFAULTS = Object.fromEntries(
    PROVIDERS.map((provider) => [provider.value, provider]),
) as Record<ChannelType, ProviderDefinition>;

export function providerLabel(type: string): string {
    return PROVIDERS.find((provider) => provider.value === type.toLowerCase())?.label ?? "Custom";
}
