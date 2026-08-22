import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const packageJsonPath = resolve(root, "package.json");
const tauriConfigPath = resolve(root, "src-tauri/tauri.conf.json");
const cargoManifestPath = resolve(root, "src-tauri/Cargo.toml");

const packageVersion = JSON.parse(readFileSync(packageJsonPath, "utf8")).version;
const tauriVersion = JSON.parse(readFileSync(tauriConfigPath, "utf8")).version;
const cargoMetadata = JSON.parse(execFileSync(
    "cargo",
    ["metadata", "--no-deps", "--format-version", "1", "--manifest-path", cargoManifestPath],
    { cwd: root, encoding: "utf8" },
));
const cargoPackage = cargoMetadata.packages.find(
    (item) => resolve(item.manifest_path) === cargoManifestPath,
);

if (!cargoPackage) {
    throw new Error(`未找到 ${cargoManifestPath} 对应的 Cargo 包`);
}

const versions = {
    "package.json": packageVersion,
    "src-tauri/tauri.conf.json": tauriVersion,
    "src-tauri/Cargo.toml": cargoPackage.version,
};
const uniqueVersions = new Set(Object.values(versions));

if (uniqueVersions.size !== 1) {
    const details = Object.entries(versions)
        .map(([file, version]) => `${file}=${version}`)
        .join(", ");
    throw new Error(`发布版本不一致：${details}`);
}

const version = packageVersion;
const releaseTag = process.argv[2];
if (releaseTag && releaseTag !== `v${version}`) {
    throw new Error(`发布标签 ${releaseTag} 与应用版本 v${version} 不一致`);
}

console.log(`发布版本校验通过：v${version}`);
