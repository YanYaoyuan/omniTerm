import { packager } from "@electron/packager";
import { open, readFile, writeFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

async function sha256(filename) {
  const hash = createHash("sha256");
  const file = await open(filename, "r");
  try { for await (const chunk of file.createReadStream()) hash.update(chunk); } finally { await file.close(); }
  return hash.digest("hex");
}
const root = fileURLToPath(new URL("../", import.meta.url));
const repository = path.resolve(root, "..");
const agent = path.join(repository, "dist/omniterm-agent");
const agentManifest = JSON.parse(await readFile(path.join(agent, "codex-package.json"), "utf8"));
const agentTargets = {
  "darwin-arm64": ["aarch64-apple-darwin"], "darwin-x64": ["x86_64-apple-darwin"],
  "linux-arm64": ["aarch64-unknown-linux-gnu", "aarch64-unknown-linux-musl"],
  "linux-x64": ["x86_64-unknown-linux-gnu", "x86_64-unknown-linux-musl"],
  "win32-arm64": ["aarch64-pc-windows-msvc"], "win32-x64": ["x86_64-pc-windows-msvc"],
};
if (!agentTargets[`${process.platform}-${process.arch}`]?.includes(agentManifest.target)) {
  throw new Error(`Agent target ${JSON.stringify(agentManifest.target)} does not match this native desktop platform. Rebuild dist/omniterm-agent.`);
}
let agentSource = null;
try { agentSource = JSON.parse(await readFile(path.join(agent, "omniterm-source.json"), "utf8")); }
catch (error) { if (error.code !== "ENOENT") throw error; }
if (process.env.OMNITERM_REQUIRE_AGENT_PROVENANCE === "1" && !agentSource) throw new Error("Release packaging requires scripts/release/build-agent.py provenance.");
let sourceCommit = null;
try { sourceCommit = execFileSync("git", ["rev-parse", "HEAD"], { cwd: repository, encoding: "utf8" }).trim(); } catch {}
const metadata = JSON.parse(await readFile(path.join(root, "package.json"), "utf8"));
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
for (const directory of appPaths) {
  const appRoot = process.platform === "darwin" ? path.join(directory, "omniTerm.app/Contents/Resources/app") : path.join(directory, "resources/app");
  await writeFile(path.join(appRoot, "omniterm-build.json"), `${JSON.stringify({ version: metadata.version, desktopSourceCommit: sourceCommit, platform: process.platform, arch: process.arch, agent: agentSource, signed: false, developerIDSigned: false, adHocSigned: process.platform === "darwin", notarized: false, linuxLauncherUtf8Fallback: process.platform === "linux" }, null, 2)}\n`);
  if (process.platform === "darwin") {
    const app = path.join(directory, "omniTerm.app");
    const packagedAgent = path.join(app, "Contents/Resources/omniterm-agent");
    if (agentSource) {
      for (const [relative, expected] of Object.entries(agentSource.binarySha256)) {
        if (path.isAbsolute(relative) || relative.split(/[\\/]/).includes("..")) throw new Error("Invalid Agent provenance path");
        const binary = path.join(packagedAgent, relative);
        if (await sha256(binary) !== expected) throw new Error(`Agent changed before ad-hoc signing: ${relative}`);
        execFileSync("/usr/bin/codesign", ["--force", "--sign", "-", binary], { stdio: "inherit" });
      }
    }
    // No developer certificate/key: ad-hoc signing seals the final bundle after
    // metadata is written. Preserve Electron's existing helper entitlements.
    execFileSync("/usr/bin/codesign", ["--force", "--deep", "--sign", "-", "--preserve-metadata=entitlements", app], { stdio: "inherit" });
    execFileSync("/usr/bin/codesign", ["--verify", "--deep", "--strict", app], { stdio: "inherit" });
    const packagedSignedBinarySha256 = {};
    for (const relative of Object.keys(agentSource?.binarySha256 || {})) packagedSignedBinarySha256[relative] = await sha256(path.join(packagedAgent, relative));
    // Writing this record inside the app after signing would invalidate its seal.
    await writeFile(path.join(directory, "omniterm-signature.json"), `${JSON.stringify({ adHoc: true, developerIDSigned: false, notarized: false, originalAgentBinarySha256: agentSource?.binarySha256 || {}, packagedSignedBinarySha256 }, null, 2)}\n`);
  }
}
console.log(appPaths.join("\n"));
