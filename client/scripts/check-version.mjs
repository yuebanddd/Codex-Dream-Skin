import { readFileSync } from "node:fs";

const packageJson = JSON.parse(readFileSync("package.json", "utf8"));
const packageLock = JSON.parse(readFileSync("package-lock.json", "utf8"));
const tauriConfig = JSON.parse(
  readFileSync("src-tauri/tauri.conf.json", "utf8"),
);
const cargoToml = readFileSync("src-tauri/Cargo.toml", "utf8");
const cargoVersion = cargoToml.match(/^version\s*=\s*"([^"]+)"$/m)?.[1];
const cargoTauriVersion = cargoToml.match(
  /^tauri\s*=\s*\{\s*version\s*=\s*"([^"]+)"/m,
)?.[1];

const versions = new Map([
  ["package.json", packageJson.version],
  ["package-lock.json", packageLock.version],
  ["package-lock root", packageLock.packages?.[""]?.version],
  ["tauri.conf.json", tauriConfig.version],
  ["Cargo.toml", cargoVersion],
]);
const expected = packageJson.version;
const mismatches = [...versions].filter(([, version]) => version !== expected);
const npmTauriVersion = packageJson.dependencies?.["@tauri-apps/api"];
const lockedNpmTauriVersion =
  packageLock.packages?.["node_modules/@tauri-apps/api"]?.version;
const tauriSeries = (version) => version?.match(/^(\d+\.\d+)\./)?.[1];
const tauriMismatch =
  !npmTauriVersion ||
  npmTauriVersion !== lockedNpmTauriVersion ||
  tauriSeries(npmTauriVersion) !== tauriSeries(cargoTauriVersion);

if (!expected || mismatches.length > 0 || tauriMismatch) {
  for (const [source, version] of versions) {
    process.stderr.write(`${source}: ${version ?? "missing"}\n`);
  }
  process.stderr.write(
    `Tauri API: package=${npmTauriVersion ?? "missing"}, lock=${lockedNpmTauriVersion ?? "missing"}, Rust=${cargoTauriVersion ?? "missing"}\n`,
  );
  process.exitCode = 1;
} else {
  process.stdout.write(
    `LumaDrobe ${expected} and Tauri ${tauriSeries(npmTauriVersion)} versions are consistent.\n`,
  );
}
