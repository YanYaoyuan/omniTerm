import { execFileSync } from "node:child_process";
import { copyFile, mkdir } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

// The result lands where packaging collects it and where development runs look for it.
const root = fileURLToPath(new URL("../", import.meta.url));
const manifest = path.join(root, "native/terminal-host/Cargo.toml");
const name = process.platform === "win32" ? "omniterm-terminal-host.exe" : "omniterm-terminal-host";
execFileSync("cargo", ["build", "--release", "--locked", "--manifest-path", manifest], { stdio: "inherit" });
const targetDirectory = process.env.CARGO_TARGET_DIR ? path.resolve(process.env.CARGO_TARGET_DIR) : path.join(root, "native/terminal-host/target");
const output = path.join(root, "../dist/terminal-host");
await mkdir(output, { recursive: true });
await copyFile(path.join(targetDirectory, "release", name), path.join(output, name));
console.log(path.join(output, name));
