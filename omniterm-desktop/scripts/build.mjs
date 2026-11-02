import { build as bundle } from "esbuild";
import { build as buildRenderer } from "vite";
import { mkdir, copyFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../", import.meta.url));
await mkdir(path.join(root, "build"), { recursive: true });
await bundle({
  entryPoints: [path.join(root, "electron/main.mjs")],
  outfile: path.join(root, "build/main.mjs"),
  bundle: true,
  platform: "node",
  format: "esm",
  target: "node24",
  external: ["electron"],
  sourcemap: true,
});
await copyFile(path.join(root, "electron/preload.cjs"), path.join(root, "build/preload.cjs"));
await buildRenderer({
  root,
  base: "./",
  configFile: false,
  build: { outDir: "build-renderer", emptyOutDir: true, sourcemap: true },
});
