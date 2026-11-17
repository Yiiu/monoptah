import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const projectRoot = fileURLToPath(new URL("../", import.meta.url));

function fixture(t) {
  const root = mkdtempSync(join(tmpdir(), "monoptah-openspec-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  for (const dir of ["scripts", "openspec/specs/development-workflow", "machine-config/openspec"]) {
    mkdirSync(join(root, dir), { recursive: true });
  }
  for (const path of [
    "scripts/openspec.mjs",
    "openspec/config.yaml",
    "openspec/specs/development-workflow/spec.md",
  ]) {
    copyFileSync(join(projectRoot, path), join(root, path));
  }
  symlinkSync(
    join(projectRoot, "node_modules"),
    join(root, "node_modules"),
    process.platform === "win32" ? "junction" : "dir",
  );
  const machineConfig = JSON.stringify({ profile: "custom", workflows: ["explore"] });
  const machineConfigPath = join(root, "machine-config/openspec/config.json");
  writeFileSync(machineConfigPath, machineConfig);
  return {
    root,
    machineConfig,
    machineConfigPath,
    run(args) {
      return spawnSync(process.execPath, [join(root, "scripts/openspec.mjs"), ...args], {
        encoding: "utf8",
        env: {
          ...process.env,
          XDG_CONFIG_HOME: join(root, "machine-config"),
          XDG_DATA_HOME: join(root, "machine-data"),
          OPENSPEC_SKILLS_SCOPE: "global",
        },
      });
    },
  };
}

function succeeds(result) {
  assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`);
}

test("project initialization preserves authored files and isolates machine preferences", (t) => {
  const f = fixture(t);
  const config = readFileSync(join(f.root, "openspec/config.yaml"), "utf8");
  const specPath = join(f.root, "openspec/specs/development-workflow/spec.md");
  const spec = readFileSync(specPath, "utf8");
  const args = ["init", "--tools", "claude,codex", "--profile", "core", "--scope", "project", "--no-animation"];
  succeeds(f.run(args));
  succeeds(f.run(args));
  assert.equal(readFileSync(f.machineConfigPath, "utf8"), f.machineConfig);
  assert.equal(readFileSync(join(f.root, "openspec/config.yaml"), "utf8"), config);
  assert.equal(readFileSync(specPath, "utf8"), spec);
  assert.equal(existsSync(join(f.root, "machine-data")), false);
  assert.equal(existsSync(join(f.root, ".openspec-local/config/openspec/config.json")), true);
  assert.deepEqual(
    readdirSync(join(f.root, ".agents/skills")).filter((name) => name.startsWith("openspec-")).sort(),
    [
      "openspec-apply-change",
      "openspec-archive-change",
      "openspec-explore",
      "openspec-propose",
      "openspec-sync-specs",
      "openspec-update-change",
    ],
  );
});

test("the project launcher blocks machine-wide skill refresh and initialization", (t) => {
  const f = fixture(t);
  for (const args of [
    ["update", "--force"],
    ["--no-color", "update", "--force"],
    ["init", "--scope", "global"],
    ["init", "--scope=global"],
    ["--no-color", "init", "--scope", "global"],
  ]) {
    const result = f.run(args);
    assert.equal(result.status, 1);
    assert.match(result.stderr, /project/);
  }
  assert.equal(existsSync(join(f.root, ".openspec-local")), false);
  assert.equal(existsSync(join(f.root, ".agents")), false);
  assert.equal(readFileSync(f.machineConfigPath, "utf8"), f.machineConfig);
});

test("the project launcher forwards successful and failed validation results", (t) => {
  const f = fixture(t);
  const args = ["validate", "--all", "--strict", "--no-interactive"];
  succeeds(f.run(args));
  writeFileSync(join(f.root, "openspec/specs/development-workflow/spec.md"), "# Invalid specification\n");
  const failed = f.run(args);
  assert.equal(failed.status, 1);
  assert.match(failed.stdout + failed.stderr, /fail|invalid|issue/i);
});
