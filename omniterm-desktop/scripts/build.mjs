import { build as bundle } from "esbuild";
import { build as buildRenderer } from "vite";
import { mkdir, copyFile, chmod } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../", import.meta.url));
if (process.platform !== "win32") {
  // Some node-pty packages lose this executable bit in their prebuild archive.
  for (const directory of ["build/Release", "build/Debug", `prebuilds/${process.platform}-${process.arch}`]) {
    await chmod(path.join(root, "node_modules/node-pty", directory, "spawn-helper"), 0o755)
      .catch((error) => { if (error.code !== "ENOENT") throw error; });
  }
}
await mkdir(path.join(root, "build"), { recursive: true });
await bundle({
  entryPoints: [path.join(root, "electron/main.mjs")],
  outfile: path.join(root, "build/main.mjs"),
  bundle: true,
  platform: "node",
  format: "esm",
  target: "node24",
  external: ["electron", "node-pty"],
  sourcemap: true,
});
await copyFile(path.join(root, "electron/preload.cjs"), path.join(root, "build/preload.cjs"));
await buildRenderer({
  root,
  base: "./",
  configFile: false,
  build: { outDir: "build-renderer", emptyOutDir: true, sourcemap: true },
});
