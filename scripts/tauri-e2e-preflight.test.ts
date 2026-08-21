import { describe, expect, it } from "vitest";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { buildOverlay, createPreflightFiles, parsePort } from "./tauri-e2e-preflight.mjs";

describe("tauri E2E preflight", () => {
    it("validates isolated port boundaries", () => {
        expect(parsePort("1422", "UI 端口")).toBe(1422);
        expect(() => parsePort("80", "UI 端口")).toThrow("1024");
        expect(() => parsePort("not-a-port", "UI 端口")).toThrow("整数");
    });

    it("writes a per-run overlay and environment contract", () => {
        const target = mkdtempSync(join(tmpdir(), "crowapi-e2e-test-"));
        try {
            const result = createPreflightFiles({
                outputDir: target,
                uiPort: 14522,
                hmrPort: 14523,
                serverPort: 18777,
            });
            expect(result.identifier).toMatch(/^com\.llf\.crowapi\.e2e\.[a-z0-9]+$/);
            expect(JSON.parse(readFileSync(result.configPath, "utf8"))).toEqual({
                identifier: result.identifier,
                build: { devUrl: "http://127.0.0.1:14522" },
            });
            expect(readFileSync(result.envPath, "utf8")).toContain("CROWAPI_E2E_SERVER_PORT=18777");
        } finally {
            rmSync(target, { recursive: true, force: true });
        }
    });

    it("rejects an overlay that could collide with its own ports", () => {
        expect(() => buildOverlay({ identifier: "com.llf.crowapi", uiPort: 1422 }))
            .toThrow("identifier");
    });
});
