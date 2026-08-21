import { describe, expect, it } from "vitest";
import { buildReleaseConfig } from "./write-tauri-release-config.mjs";

const updaterEnv = {
    TAURI_UPDATER_PUBLIC_KEY: "test-public-key",
};

describe("buildReleaseConfig", () => {
    it("enables hardened runtime and certificate inference on macOS", () => {
        expect(buildReleaseConfig(updaterEnv, "darwin")).toEqual({
            bundle: {
                createUpdaterArtifacts: true,
                macOS: {
                    hardenedRuntime: true,
                    signingIdentity: null,
                },
            },
            plugins: {
                updater: { pubkey: "test-public-key" },
            },
        });
    });

    it("preserves an explicit macOS signing identity", () => {
        const config = buildReleaseConfig({
            ...updaterEnv,
            APPLE_SIGNING_IDENTITY: "Developer ID Application: CrowAPI",
        }, "darwin");

        expect(config.bundle.macOS?.signingIdentity)
            .toBe("Developer ID Application: CrowAPI");
    });

    it("configures Windows Authenticode signing", () => {
        expect(buildReleaseConfig({
            ...updaterEnv,
            WINDOWS_CERTIFICATE_THUMBPRINT: "AA BB CC",
        }, "win32")).toEqual({
            bundle: {
                createUpdaterArtifacts: true,
                windows: {
                    certificateThumbprint: "AA BB CC",
                    digestAlgorithm: "sha256",
                    timestampUrl: "http://timestamp.digicert.com",
                },
            },
            plugins: {
                updater: { pubkey: "test-public-key" },
            },
        });
    });

    it("requires the Windows certificate thumbprint", () => {
        expect(() => buildReleaseConfig(updaterEnv, "win32"))
            .toThrow("缺少 WINDOWS_CERTIFICATE_THUMBPRINT");
    });

    it("keeps Linux release configuration updater-only", () => {
        expect(buildReleaseConfig(updaterEnv, "linux").bundle).toEqual({
            createUpdaterArtifacts: true,
        });
    });
});
