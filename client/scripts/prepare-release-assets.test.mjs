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
import assert from "node:assert/strict";
import test from "node:test";

const repositoryRoot = resolve(
  dirname(fileURLToPath(import.meta.url)),
  "../..",
);
const buildCommit = "a".repeat(40);
const version = "0.5.8";
const fixtures = [
  ["LumaDrobe-macOS-arm64", "original-arm64.dmg"],
  ["LumaDrobe-Windows-x64", "original-setup.exe"],
];

function signatureMetadata(signed) {
  return signed
    ? {
        format: "Authenticode",
        provider: "SignPath.io",
        publisher: "SignPath Foundation",
        verified: true,
      }
    : null;
}

function createFixture({ signedWindows = false } = {}) {
  const temporary = mkdtempSync(join(tmpdir(), "lumadrobe-release-assets-"));
  const source = join(temporary, "source");
  const output = join(temporary, "output");

  for (const [artifact, packageName] of fixtures) {
    const directory = join(source, artifact);
    const signed = artifact === "LumaDrobe-Windows-x64" && signedWindows;
    mkdirSync(directory, { recursive: true });
    const contents = Buffer.from(`fixture:${artifact}`);
    const sha256 = createHash("sha256").update(contents).digest("hex");
    writeFileSync(join(directory, packageName), contents);
    writeFileSync(
      join(directory, "BUILD-INFO.json"),
      JSON.stringify({
        schemaVersion: 2,
        product: "LumaDrobe",
        version,
        buildCommit,
        signed,
        signature: signatureMetadata(signed),
        artifacts: [{ name: packageName, bytes: contents.length, sha256 }],
      }),
    );
    writeFileSync(
      join(directory, "SHA256SUMS.txt"),
      `${sha256}  ${packageName}\n`,
    );
  }

  return { temporary, source, output };
}

function prepare(fixture, requireWindowsSignature = false, githubOutput) {
  return spawnSync(
    process.execPath,
    [
      "client/scripts/prepare-release-assets.mjs",
      fixture.source,
      fixture.output,
    ],
    {
      cwd: repositoryRoot,
      encoding: "utf8",
      env: {
        ...process.env,
        GITHUB_RUN_NUMBER: "14",
        LUMADROBE_BUILD_SHA: buildCommit,
        LUMADROBE_REQUIRE_WINDOWS_SIGNATURE: String(requireWindowsSignature),
        ...(githubOutput ? { GITHUB_OUTPUT: githubOutput } : {}),
      },
    },
  );
}

test("release assets allow unsigned Windows packages with explicit metadata", () => {
  const fixture = createFixture();
  try {
    const githubOutput = join(fixture.temporary, "github-output");
    const result = prepare(fixture, false, githubOutput);
    assert.equal(result.status, 0, result.stderr);
    assert.match(readFileSync(githubOutput, "utf8"), /windows_signed=false/);

    const expectedPackages = [
      "LumaDrobe-v0.5.8-Windows-x64-Setup.exe",
      "LumaDrobe-v0.5.8-macOS-arm64.dmg",
    ];
    const files = readdirSync(fixture.output);
    assert.equal(files.length, 6);
    for (const packageName of expectedPackages) {
      assert.ok(
        files.includes(packageName),
        `${packageName} should be published`,
      );
    }
    for (const artifact of fixtures.map(([name]) => name)) {
      const metadata = JSON.parse(
        readFileSync(
          join(fixture.output, `${artifact}-BUILD-INFO.json`),
          "utf8",
        ),
      );
      const checksums = readFileSync(
        join(fixture.output, `${artifact}-SHA256SUMS.txt`),
        "utf8",
      );
      assert.equal(
        metadata.artifacts[0].name,
        checksums.trim().split(/\s+/)[1],
      );
      assert.ok(expectedPackages.includes(metadata.artifacts[0].name));
    }
  } finally {
    rmSync(fixture.temporary, { recursive: true, force: true });
  }
});

test("release assets accept a verified SignPath Windows package", () => {
  const fixture = createFixture({ signedWindows: true });
  try {
    const githubOutput = join(fixture.temporary, "github-output");
    const result = prepare(fixture, true, githubOutput);
    assert.equal(result.status, 0, result.stderr);
    assert.match(readFileSync(githubOutput, "utf8"), /windows_signed=true/);
    const metadata = JSON.parse(
      readFileSync(
        join(fixture.output, "LumaDrobe-Windows-x64-BUILD-INFO.json"),
        "utf8",
      ),
    );
    assert.equal(metadata.signed, true);
    assert.equal(metadata.signature.publisher, "SignPath Foundation");
  } finally {
    rmSync(fixture.temporary, { recursive: true, force: true });
  }
});

test("release gate rejects an unsigned Windows package", () => {
  const fixture = createFixture();
  try {
    const result = prepare(fixture, true);
    assert.notEqual(result.status, 0);
    assert.match(
      result.stderr,
      /LumaDrobe-Windows-x64 BUILD-INFO\.json is inconsistent/,
    );
  } finally {
    rmSync(fixture.temporary, { recursive: true, force: true });
  }
});

test("release gate rejects unapproved Windows signature metadata", () => {
  const fixture = createFixture({ signedWindows: true });
  try {
    const metadataPath = join(
      fixture.source,
      "LumaDrobe-Windows-x64",
      "BUILD-INFO.json",
    );
    const metadata = JSON.parse(readFileSync(metadataPath, "utf8"));
    metadata.signature.publisher = "Unknown Publisher";
    writeFileSync(metadataPath, JSON.stringify(metadata));

    const result = prepare(fixture, true);
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /verified SignPath Authenticode metadata/);
  } finally {
    rmSync(fixture.temporary, { recursive: true, force: true });
  }
});
