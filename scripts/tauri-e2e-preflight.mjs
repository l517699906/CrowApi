import { randomUUID } from "node:crypto";
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export function parsePort(value, label) {
    const port = Number(value);
    if (!Number.isInteger(port) || port < 1024 || port > 65_535) {
        throw new Error(`${label} 必须是 1024 到 65535 之间的整数`);
    }
    return port;
}

export function buildOverlay({ identifier, uiPort }) {
    if (!/^com\.llf\.crowapi\.e2e\.[a-z0-9]+$/.test(identifier)) {
        throw new Error("E2E identifier 格式无效");
    }
    return {
        identifier,
        build: {
            devUrl: `http://127.0.0.1:${parsePort(uiPort, "UI 端口")}`,
        },
    };
}

function commandExists(command) {
    try {
        execFileSync(process.platform === "win32" ? "where" : "which", [command], {
            stdio: "ignore",
        });
        return true;
    } catch {
        return false;
    }
}

export function createPreflightFiles({ outputDir, uiPort, hmrPort, serverPort }) {
    const parsedUiPort = parsePort(uiPort, "UI 端口");
    const parsedHmrPort = parsePort(hmrPort, "HMR 端口");
    const parsedServerPort = parsePort(serverPort, "服务端口");
    if (parsedUiPort === parsedHmrPort || parsedUiPort === parsedServerPort || parsedHmrPort === parsedServerPort) {
        throw new Error("UI、HMR 和服务端口必须互不相同");
    }

    const targetDir = resolve(outputDir);
    mkdirSync(targetDir, { recursive: true, mode: 0o700 });
    const identifier = `com.llf.crowapi.e2e.${randomUUID().replaceAll("-", "").slice(0, 12)}`;
    const configPath = join(targetDir, "tauri.e2e.conf.json");
    const envPath = join(targetDir, "tauri.e2e.env");
    writeFileSync(configPath, `${JSON.stringify(buildOverlay({ identifier, uiPort: parsedUiPort }), null, 2)}\n`, {
        encoding: "utf8",
        mode: 0o600,
    });
    writeFileSync(envPath, [
        `TAURI_E2E_PORT=${parsedUiPort}`,
        `TAURI_E2E_HMR_PORT=${parsedHmrPort}`,
        `CROWAPI_E2E_SERVER_PORT=${parsedServerPort}`,
        `CROWAPI_E2E_IDENTIFIER=${identifier}`,
        `CROWAPI_E2E_TAURI_CONFIG=${configPath}`,
        "",
    ].join("\n"), { encoding: "utf8", mode: 0o600 });
    return { targetDir, configPath, envPath, identifier, uiPort: parsedUiPort, hmrPort: parsedHmrPort, serverPort: parsedServerPort };
}

function randomPort(excluded = new Set(), start = 20_000, end = 59_000) {
    let port;
    do {
        port = start + Math.floor(Math.random() * (end - start));
    } while (excluded.has(port));
    return port;
}

function main() {
    const requestedDir = process.argv[2];
    const outputDir = requestedDir
        ? resolve(requestedDir)
        : mkdtempSync(join(tmpdir(), "crowapi-e2e-"));
    const uiPort = parsePort(process.env.TAURI_E2E_PORT ?? randomPort(), "UI 端口");
    const hmrPort = parsePort(process.env.TAURI_E2E_HMR_PORT ?? uiPort + 1, "HMR 端口");
    const serverPort = parsePort(
        process.env.CROWAPI_E2E_SERVER_PORT ?? randomPort(new Set([uiPort, hmrPort])),
        "服务端口",
    );
    const result = createPreflightFiles({ outputDir, uiPort, hmrPort, serverPort });
    const missing = ["node", "cargo"].filter((command) => !commandExists(command));
    console.log(JSON.stringify({
        ...result,
        missingCommands: missing,
        launch: `TAURI_E2E_PORT=${result.uiPort} TAURI_E2E_HMR_PORT=${result.hmrPort} CROWAPI_E2E_SERVER_PORT=${result.serverPort} npm run tauri -- dev --config "${result.configPath}"`,
        note: "这是隔离启动预检，不会启动桌面应用，也不等同于 Playwright/tauri-driver E2E。",
    }, null, 2));
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
    main();
}
