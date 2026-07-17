import { createHash } from "node:crypto";
import { readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const directory = process.argv[2] ?? "artifacts";
const packageJson = JSON.parse(readFileSync("package.json", "utf8"));
const files = readdirSync(directory)
  .filter((name) => !["BUILD-INFO.json", "SHA256SUMS.txt"].includes(name))
  .filter((name) => statSync(join(directory, name)).isFile())
  .sort();

if (files.length === 0) {
  throw new Error(`No package artifacts found in ${directory}`);
}

const artifacts = files.map((name) => {
  const contents = readFileSync(join(directory, name));
  return {
    name,
    bytes: contents.length,
    sha256: createHash("sha256").update(contents).digest("hex"),
  };
});
const buildInfo = {
  schemaVersion: 1,
  product: "LumaDrobe",
  version: packageJson.version,
  buildCommit: process.env.LUMADROBE_BUILD_SHA ?? "development",
  platform: process.env.RUNNER_OS ?? process.platform,
  architecture: process.env.RUNNER_ARCH ?? process.arch,
  workflowRun: process.env.GITHUB_RUN_ID ?? null,
  createdAt: new Date().toISOString(),
  signed: false,
  artifacts,
};

writeFileSync(
  join(directory, "BUILD-INFO.json"),
  `${JSON.stringify(buildInfo, null, 2)}\n`,
);
writeFileSync(
  join(directory, "SHA256SUMS.txt"),
  `${artifacts.map(({ name, sha256 }) => `${sha256}  ${name}`).join("\n")}\n`,
);
