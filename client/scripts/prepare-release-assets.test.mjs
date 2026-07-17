import { createHash } from "node:crypto";
import {
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import test from "node:test";
import assert from "node:assert/strict";

const repositoryRoot = resolve(
  dirname(fileURLToPath(import.meta.url)),
  "../..",
);
const buildCommit = "a".repeat(40);
const version = "0.5.3";
const fixtures = [
  ["LumaDrobe-macOS-arm64", "original-arm64.dmg"],
  ["LumaDrobe-Windows-x64", "original-setup.exe"],
];

test("release assets use platform-explicit names and matching metadata", () => {
  const temporary = mkdtempSync(join(tmpdir(), "lumadrobe-release-assets-"));
  const source = join(temporary, "source");
  const output = join(temporary, "output");
  try {
    for (const [artifact, packageName] of fixtures) {
      const directory = join(source, artifact);
      mkdirSync(directory, { recursive: true });
      const contents = Buffer.from(`fixture:${artifact}`);
      const sha256 = createHash("sha256").update(contents).digest("hex");
      writeFileSync(join(directory, packageName), contents);
      writeFileSync(
        join(directory, "BUILD-INFO.json"),
        JSON.stringify({
          product: "LumaDrobe",
          version,
          buildCommit,
          signed: false,
          artifacts: [{ name: packageName, bytes: contents.length, sha256 }],
        }),
      );
      writeFileSync(
        join(directory, "SHA256SUMS.txt"),
        `${sha256}  ${packageName}\n`,
      );
    }

    const result = spawnSync(
      process.execPath,
      ["client/scripts/prepare-release-assets.mjs", source, output],
      {
        cwd: repositoryRoot,
        encoding: "utf8",
        env: {
          ...process.env,
          GITHUB_RUN_NUMBER: "14",
          LUMADROBE_BUILD_SHA: buildCommit,
        },
      },
    );
    assert.equal(result.status, 0, result.stderr);

    const expectedPackages = [
      "LumaDrobe-v0.5.3-Windows-x64-Setup.exe",
      "LumaDrobe-v0.5.3-macOS-arm64.dmg",
    ];
    const files = readdirSync(output);
    assert.equal(files.length, 6);
    for (const packageName of expectedPackages) {
      assert.ok(
        files.includes(packageName),
        `${packageName} should be published`,
      );
    }
    for (const artifact of fixtures.map(([name]) => name)) {
      const metadata = JSON.parse(
        readFileSync(join(output, `${artifact}-BUILD-INFO.json`), "utf8"),
      );
      const checksums = readFileSync(
        join(output, `${artifact}-SHA256SUMS.txt`),
        "utf8",
      );
      assert.equal(
        metadata.artifacts[0].name,
        checksums.trim().split(/\s+/)[1],
      );
      assert.ok(expectedPackages.includes(metadata.artifacts[0].name));
    }
  } finally {
    rmSync(temporary, { recursive: true, force: true });
  }
});
