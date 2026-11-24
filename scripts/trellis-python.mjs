#!/usr/bin/env node
// Runs a Trellis Python hook with whichever Python 3 this machine has.
//
// Trellis writes a fixed interpreter name into the hook commands it generates:
// `python` when `trellis init` runs on Windows, `python3` elsewhere. Neither works
// everywhere: macOS usually has only `python3`, and on Windows `python3` is often the
// Microsoft Store stub, which exits non-zero instead of running anything. The
// committed hook configs (.claude/settings.json, .codex/hooks.json) call this
// launcher through Node, which every checkout of this repo already needs.
//
// Usage: node scripts/trellis-python.mjs [python args...] <script.py> [script args...]
// Set TRELLIS_PYTHON to force an interpreter (a command name or a full path).

import { spawnSync } from "node:child_process";

const candidates = process.env.TRELLIS_PYTHON
  ? [[process.env.TRELLIS_PYTHON]]
  : process.platform === "win32"
    ? [["python"], ["py", "-3"], ["python3"]]
    : [["python3"], ["python"]];

function works([command, ...prefix]) {
  const probe = spawnSync(command, [...prefix, "-c", "import sys; sys.exit(0 if sys.version_info[0] == 3 else 1)"], {
    stdio: "ignore",
    windowsHide: true,
  });
  return !probe.error && probe.status === 0;
}

const python = candidates.find(works);
if (!python) {
  process.stderr.write(
    `trellis-python: no working Python 3 found (tried ${candidates.map((c) => c.join(" ")).join(", ")}). ` +
      "Install Python 3 or set TRELLIS_PYTHON.\n",
  );
  process.exit(1);
}

const [command, ...prefix] = python;
const result = spawnSync(command, [...prefix, ...process.argv.slice(2)], {
  stdio: "inherit",
  windowsHide: true,
});
if (result.error) {
  process.stderr.write(`trellis-python: ${result.error.message}\n`);
  process.exit(1);
}
process.exit(result.status ?? 1);
