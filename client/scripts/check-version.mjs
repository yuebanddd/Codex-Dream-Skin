import { readFileSync } from "node:fs";

const packageJson = JSON.parse(readFileSync("package.json", "utf8"));
const packageLock = JSON.parse(readFileSync("package-lock.json", "utf8"));
const tauriConfig = JSON.parse(
  readFileSync("src-tauri/tauri.conf.json", "utf8"),
);
const cargoToml = readFileSync("src-tauri/Cargo.toml", "utf8");
const cargoVersion = cargoToml.match(/^version\s*=\s*"([^"]+)"$/m)?.[1];

const versions = new Map([
  ["package.json", packageJson.version],
  ["package-lock.json", packageLock.version],
  ["package-lock root", packageLock.packages?.[""]?.version],
  ["tauri.conf.json", tauriConfig.version],
  ["Cargo.toml", cargoVersion],
]);
const expected = packageJson.version;
const mismatches = [...versions].filter(([, version]) => version !== expected);

if (!expected || mismatches.length > 0) {
  for (const [source, version] of versions) {
    process.stderr.write(`${source}: ${version ?? "missing"}\n`);
  }
  process.exitCode = 1;
} else {
  process.stdout.write(`LumaDrobe version ${expected} is consistent.\n`);
}
