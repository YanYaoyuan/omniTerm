import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import electron from "electron";

const root = fileURLToPath(new URL("../", import.meta.url));
const env = { ...process.env };
// Editor hosts may export this for their own Node helpers. A desktop launch
// must run Electron as an application, including when started from such a host.
delete env.ELECTRON_RUN_AS_NODE;
const child = spawn(electron, [root, ...process.argv.slice(2)], {
  env,
  stdio: "inherit",
});
child.on("error", (error) => { console.error(error.message); process.exitCode = 1; });
child.on("exit", (code) => { process.exitCode = code ?? 1; });
