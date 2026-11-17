import { spawnSync } from "node:child_process";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const projectRoot = fileURLToPath(new URL("../", import.meta.url));
const args = process.argv.slice(2);
const command = args.find((arg) => !arg.startsWith("-"));

// OpenSpec 2.1's update command also refreshes machine-wide skills, even when
// this project uses project-scoped skills. Reinitialization refreshes only the
// explicitly selected project integrations instead.
if (command === "update" && !args.includes("--help") && !args.includes("-h")) {
  console.error(
    "Use npm run openspec:init to refresh this project's skills. " +
      "OpenSpec 2.1's update command can also modify machine-wide skills.",
  );
  process.exit(1);
}

if (
  command === "init" &&
  args.some(
    (arg, index) =>
      arg === "--scope=global" ||
      (arg === "--scope" && args[index + 1] === "global"),
  )
) {
  console.error("This project uses project-scoped OpenSpec skills.");
  process.exit(1);
}

const localState = join(projectRoot, ".openspec-local");
const result = spawnSync(
  process.execPath,
  [join(projectRoot, "node_modules/@fission-ai/openspec/bin/openspec.js"), ...args],
  {
    cwd: projectRoot,
    stdio: "inherit",
    env: {
      ...process.env,
      XDG_CONFIG_HOME: join(localState, "config"),
      XDG_DATA_HOME: join(localState, "data"),
      OPENSPEC_SKILLS_SCOPE: "project",
      OPENSPEC_TELEMETRY: "0",
    },
  },
);

if (result.error) {
  console.error(`Unable to start the project-local OpenSpec CLI: ${result.error.message}`);
}
process.exit(result.status ?? 1);
