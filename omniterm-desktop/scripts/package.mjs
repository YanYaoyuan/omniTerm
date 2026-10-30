import { packager } from "@electron/packager";
import { access } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../", import.meta.url));
const repository = path.resolve(root, "..");
const agent = path.join(repository, "dist/omniterm-agent");
await access(path.join(agent, "codex-package.json"));
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
