import { createHash } from "node:crypto";
import {
  appendFileSync,
  copyFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { join } from "node:path";

const sourceDirectory = process.argv[2] ?? "downloaded-artifacts";
const outputDirectory = process.argv[3] ?? "release-assets";
const packageJson = JSON.parse(readFileSync("client/package.json", "utf8"));
const version = packageJson.version;
const runNumber = process.env.GITHUB_RUN_NUMBER;
const buildCommit = process.env.LUMADROBE_BUILD_SHA ?? process.env.GITHUB_SHA;

if (!/^\d+\.\d+\.\d+$/.test(version)) {
  throw new Error(`Release version must be x.y.z, received ${version}`);
}
if (!/^\d+$/.test(runNumber ?? "")) {
  throw new Error("GITHUB_RUN_NUMBER must be numeric");
}
if (!/^[0-9a-f]{40}$/.test(buildCommit ?? "")) {
  throw new Error(
    "LUMADROBE_BUILD_SHA or GITHUB_SHA must be a full commit SHA",
  );
}
if (existsSync(outputDirectory)) {
  throw new Error(`${outputDirectory} already exists`);
}

const platforms = [
  {
    artifact: "LumaDrobe-macOS-arm64",
    extension: ".dmg",
    releaseName: `LumaDrobe-v${version}-macOS-arm64.dmg`,
  },
  {
    artifact: "LumaDrobe-macOS-x64",
    extension: ".dmg",
    releaseName: `LumaDrobe-v${version}-macOS-x64.dmg`,
  },
  {
    artifact: "LumaDrobe-Windows-x64",
    extension: ".exe",
    releaseName: `LumaDrobe-v${version}-Windows-x64-Setup.exe`,
  },
];
mkdirSync(outputDirectory, { recursive: true });

for (const platform of platforms) {
  const directory = join(sourceDirectory, platform.artifact);
  if (!existsSync(directory) || !statSync(directory).isDirectory()) {
    throw new Error(`Missing artifact directory ${directory}`);
  }

  const packages = readdirSync(directory).filter((name) =>
    name.toLowerCase().endsWith(platform.extension),
  );
  if (packages.length !== 1) {
    throw new Error(
      `${platform.artifact} must contain exactly one ${platform.extension} package`,
    );
  }

  const packageName = packages[0];
  const packagePath = join(directory, packageName);
  const contents = readFileSync(packagePath);
  const sha256 = createHash("sha256").update(contents).digest("hex");
  const buildInfoPath = join(directory, "BUILD-INFO.json");
  const checksumsPath = join(directory, "SHA256SUMS.txt");
  const buildInfo = JSON.parse(readFileSync(buildInfoPath, "utf8"));
  const checksums = readFileSync(checksumsPath, "utf8").trim();
  const recorded = buildInfo.artifacts?.[0];

  if (
    buildInfo.product !== "LumaDrobe" ||
    buildInfo.version !== version ||
    buildInfo.buildCommit !== buildCommit ||
    buildInfo.signed !== false ||
    buildInfo.artifacts?.length !== 1 ||
    recorded?.name !== packageName ||
    recorded?.bytes !== contents.length ||
    recorded?.sha256 !== sha256
  ) {
    throw new Error(`${platform.artifact} BUILD-INFO.json is inconsistent`);
  }
  if (checksums !== `${sha256}  ${packageName}`) {
    throw new Error(`${platform.artifact} SHA256SUMS.txt is inconsistent`);
  }

  copyFileSync(packagePath, join(outputDirectory, platform.releaseName));
  writeFileSync(
    join(outputDirectory, `${platform.artifact}-BUILD-INFO.json`),
    `${JSON.stringify(
      {
        ...buildInfo,
        artifacts: [{ ...recorded, name: platform.releaseName }],
      },
      null,
      2,
    )}\n`,
  );
  writeFileSync(
    join(outputDirectory, `${platform.artifact}-SHA256SUMS.txt`),
    `${sha256}  ${platform.releaseName}\n`,
  );
}

const tag = `v${version}-preview.${runNumber}`;
if (process.env.GITHUB_OUTPUT) {
  appendFileSync(process.env.GITHUB_OUTPUT, `tag=${tag}\nversion=${version}\n`);
}
process.stdout.write(
  `Prepared ${readdirSync(outputDirectory).length} assets for ${tag}.\n`,
);
