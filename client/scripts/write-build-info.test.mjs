import {
  mkdtempSync,
  mkdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import assert from "node:assert/strict";
import test from "node:test";

const clientRoot = resolve(import.meta.dirname, "..");

function runWriter(environment = {}) {
  const temporary = mkdtempSync(join(tmpdir(), "lumadrobe-build-info-"));
  const artifactDirectory = join(temporary, "artifacts");
  mkdirSync(artifactDirectory);
  writeFileSync(join(artifactDirectory, "LumaDrobe-Setup.exe"), "fixture");
  const result = spawnSync(
    process.execPath,
    ["scripts/write-build-info.mjs", artifactDirectory],
    {
      cwd: clientRoot,
      encoding: "utf8",
      env: { ...process.env, ...environment },
    },
  );
  return { artifactDirectory, result, temporary };
}

test("records verified SignPath Authenticode metadata", () => {
  const fixture = runWriter({
    LUMADROBE_SIGNED: "true",
    LUMADROBE_SIGNATURE_PROVIDER: "SignPath.io",
    LUMADROBE_SIGNATURE_PUBLISHER: "SignPath Foundation",
  });
  try {
    assert.equal(fixture.result.status, 0, fixture.result.stderr);
    const metadata = JSON.parse(
      readFileSync(join(fixture.artifactDirectory, "BUILD-INFO.json"), "utf8"),
    );
    assert.equal(metadata.schemaVersion, 2);
    assert.equal(metadata.signed, true);
    assert.deepEqual(metadata.signature, {
      format: "Authenticode",
      provider: "SignPath.io",
      publisher: "SignPath Foundation",
      verified: true,
    });
  } finally {
    rmSync(fixture.temporary, { recursive: true, force: true });
  }
});

test("rejects a signed claim without the approved publisher", () => {
  const fixture = runWriter({
    LUMADROBE_SIGNED: "true",
    LUMADROBE_SIGNATURE_PROVIDER: "SignPath.io",
    LUMADROBE_SIGNATURE_PUBLISHER: "Unknown Publisher",
  });
  try {
    assert.notEqual(fixture.result.status, 0);
    assert.match(fixture.result.stderr, /SignPath\.io and SignPath Foundation/);
  } finally {
    rmSync(fixture.temporary, { recursive: true, force: true });
  }
});

test("keeps unsigned metadata explicit", () => {
  const fixture = runWriter({ LUMADROBE_SIGNED: "false" });
  try {
    assert.equal(fixture.result.status, 0, fixture.result.stderr);
    const metadata = JSON.parse(
      readFileSync(join(fixture.artifactDirectory, "BUILD-INFO.json"), "utf8"),
    );
    assert.equal(metadata.signed, false);
    assert.equal(metadata.signature, null);
  } finally {
    rmSync(fixture.temporary, { recursive: true, force: true });
  }
});
