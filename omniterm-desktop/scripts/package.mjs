import { packager } from "@electron/packager";
import { readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../", import.meta.url));
const repository = path.resolve(root, "..");
const agent = path.join(repository, "dist/omniterm-agent");
const agentManifest = JSON.parse(await readFile(path.join(agent, "codex-package.json"), "utf8"));
const agentTargets = {
  "darwin-arm64": ["aarch64-apple-darwin"],
  "darwin-x64": ["x86_64-apple-darwin"],
  "linux-arm64": ["aarch64-unknown-linux-gnu", "aarch64-unknown-linux-musl"],
  "linux-x64": ["x86_64-unknown-linux-gnu", "x86_64-unknown-linux-musl"],
  "win32-arm64": ["aarch64-pc-windows-msvc"],
  "win32-x64": ["x86_64-pc-windows-msvc"],
};
const nativePlatform = `${process.platform}-${process.arch}`;
if (!agentTargets[nativePlatform]?.includes(agentManifest.target)) {
  throw new Error(`Agent target ${JSON.stringify(agentManifest.target)} does not match ${nativePlatform}. Rebuild dist/omniterm-agent for this platform before packaging.`);
}
const appPaths = await packager({
  dir: root,
  name: "omniTerm",
  appBundleId: "dev.omniterm.desktop",
  icon: path.join(root, "assets", process.platform === "darwin" ? "icon.icns" : process.platform === "win32" ? "icon.ico" : "icon.png"),
  appCopyright: "Copyright 2026 omniTerm contributors",
  out: path.join(repository, "dist/desktop"),
  overwrite: true,
  asar: false,
  prune: true,
  ...(process.env.OMNITERM_ELECTRON_ZIP_DIR ? { electronZipDir: process.env.OMNITERM_ELECTRON_ZIP_DIR } : {}),
  extraResource: [agent],
  ignore: [/^\/src($|\/)/, /^\/electron($|\/)/, /^\/scripts($|\/)/, /^\/test($|\/)/, /^\/dist($|\/)/],
  darwinDarkModeSupport: true,
});
console.log(appPaths.join("\n"));
