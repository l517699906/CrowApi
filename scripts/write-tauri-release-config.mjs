import { writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const defaultOutputPath = resolve(root, "src-tauri/tauri.release.conf.json");
const WINDOWS_TIMESTAMP_URL = "http://timestamp.digicert.com";

function requiredUpdaterPublicKey(value) {
    const publicKey = value?.trim();
    if (!publicKey || publicKey === "REPLACE_WITH_TAURI_UPDATER_PUBLIC_KEY") {
        throw new Error("缺少有效的 TAURI_UPDATER_PUBLIC_KEY");
    }
    return publicKey;
}

export function buildReleaseConfig(env = process.env, platform = process.platform) {
    const bundle = {
        createUpdaterArtifacts: true,
    };

    if (platform === "darwin") {
        bundle.macOS = {
            hardenedRuntime: true,
            // null overrides the local ad-hoc identity and lets Tauri infer the
            // imported Developer ID identity from APPLE_CERTIFICATE.
            signingIdentity: env.APPLE_SIGNING_IDENTITY?.trim() || null,
        };
    } else if (platform === "win32") {
        const certificateThumbprint = env.WINDOWS_CERTIFICATE_THUMBPRINT?.trim();
        if (!certificateThumbprint) {
            throw new Error("缺少 WINDOWS_CERTIFICATE_THUMBPRINT");
        }
        bundle.windows = {
            certificateThumbprint,
            digestAlgorithm: "sha256",
            timestampUrl: WINDOWS_TIMESTAMP_URL,
        };
    }

    return {
        bundle,
        plugins: {
            updater: {
                pubkey: requiredUpdaterPublicKey(env.TAURI_UPDATER_PUBLIC_KEY),
            },
        },
    };
}

export function writeReleaseConfig(env = process.env, platform = process.platform) {
    const configuredPath = env.TAURI_RELEASE_CONFIG_PATH?.trim();
    const outputPath = configuredPath
        ? resolve(configuredPath)
        : defaultOutputPath;
    const config = buildReleaseConfig(env, platform);
    writeFileSync(outputPath, `${JSON.stringify(config, null, 2)}\n`, {
        encoding: "utf8",
        mode: 0o600,
    });
    return outputPath;
}

const invokedPath = process.argv[1] ? resolve(process.argv[1]) : "";
if (invokedPath === fileURLToPath(import.meta.url)) {
    const outputPath = writeReleaseConfig();
    console.log(`Tauri 发布配置已生成: ${outputPath}`);
}
