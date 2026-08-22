import { readFileSync } from "node:fs";

const [manifestPath, releaseTag] = process.argv.slice(2);
if (!manifestPath || !releaseTag) {
    throw new Error("用法：node scripts/check-updater-manifest.mjs <latest.json> <v版本>");
}

const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
const expectedVersion = releaseTag.startsWith("v") ? releaseTag.slice(1) : releaseTag;
if (manifest.version !== expectedVersion) {
    throw new Error(`更新清单版本 ${manifest.version ?? "<缺失>"} 与 ${expectedVersion} 不一致`);
}

const requiredPlatforms = [
    "darwin-aarch64",
    "darwin-x86_64",
    "linux-x86_64",
    "windows-x86_64",
];

for (const platform of requiredPlatforms) {
    const entry = manifest.platforms?.[platform];
    if (!entry || typeof entry.signature !== "string" || !entry.signature.trim()) {
        throw new Error(`更新清单缺少 ${platform} 的签名`);
    }
    if (typeof entry.url !== "string" || !entry.url.startsWith("https://")) {
        throw new Error(`更新清单缺少 ${platform} 的 HTTPS 下载地址`);
    }
}

console.log(`更新清单校验通过：${expectedVersion}，${requiredPlatforms.length} 个平台`);
