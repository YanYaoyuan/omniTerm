// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
// Package an already built native Ubuntu 22.04 amd64 application; never download at install time.
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { chmod, cp, lstat, mkdir, mkdtemp, open, readFile, readdir, readlink, realpath, rename, rm, stat, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const desktop = fileURLToPath(new URL("../", import.meta.url));
const repository = path.resolve(desktop, "..");
const defaults = [
  "libc6 (>= 2.35)", "libstdc++6", "libgcc-s1", "libasound2", "libatk-bridge2.0-0",
  "libatk1.0-0", "libatspi2.0-0", "libcairo2", "libcap2", "libcups2", "libdbus-1-3",
  "libdrm2", "libexpat1", "libgbm1", "libglib2.0-0", "libgtk-3-0", "libnspr4",
  "libnss3", "libpango-1.0-0", "libsecret-1-0", "libssl3", "libudev1", "libx11-6", "libxcb1",
  "libxcb-dri3-0", "libxcomposite1", "libxdamage1", "libxext6", "libxfixes3",
  "libxkbcommon0", "libxrandr2", "libxss1", "libxtst6", "zlib1g", "ca-certificates",
  "xdg-utils", "openssh-client", "python3", "python3-serial",
];
const help = `Create an Ubuntu 22.04 amd64 .deb from the existing Packager output.

Usage: node scripts/package-deb.mjs [options]
  --app-dir PATH       Default: ../dist/desktop/omniTerm-linux-x64 (relative to this project)
  --output-dir PATH    Default: ../dist/deb
  --version VERSION    Default: desktop package version plus -1
  --depends-file PATH  Extra Debian dependencies, or dpkg-shlibdeps -O output; merged with defaults
  --maintainer TEXT    Override the public package maintainer identity
  --allow-dirty       Explicitly allow uncommitted source for a preview package
  --help              Show help without requiring Linux

Run on native Ubuntu 22.04 x64 after npm ci, npm run rebuild, and npm run package.
The Agent must be the complete x86_64-unknown-linux-gnu package.
Set SOURCE_DATE_EPOCH when the build directory does not contain Git metadata.
No Node.js, Rust, npm downloads, sandbox-disabling flags, or user-data deletion at install time.
`;

function command(executable, args, options = {}) {
  const output = execFileSync(executable, args, { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"], maxBuffer: 32 * 1024 * 1024, ...options });
  return typeof output === "string" ? output.trim() : "";
}
function singleLine(value, label) {
  if (!value || /[\r\n\0]/.test(value)) throw new Error(`${label} must be a non-empty single line.`);
  return value;
}
function parseArgs(argv) {
  const allowed = new Set(["app-dir", "output-dir", "version", "depends-file", "maintainer"]);
  const result = {};
  for (let index = 0; index < argv.length; index++) {
    const name = argv[index].replace(/^--/, "");
    if (argv[index] === "--allow-dirty") {
      if (result[name] !== undefined) throw new Error(`Duplicate option: --${name}`);
      result[name] = true; continue;
    }
    if (!argv[index].startsWith("--") || !allowed.has(name) || !argv[index + 1] || argv[index + 1].startsWith("--")) throw new Error(`Unknown or incomplete option: ${argv[index]}`);
    if (result[name] !== undefined) throw new Error(`Duplicate option: --${name}`);
    result[name] = argv[++index];
  }
  return result;
}
function firstAllowedCpus(list, limit = 4) {
  const cpus = [];
  for (const range of list.split(",")) {
    if (!/^\d+(?:-\d+)?$/.test(range)) throw new Error("Cannot parse the builder's allowed CPU list.");
    const [first, last = first] = range.split("-").map(Number);
    if (last < first) throw new Error("Invalid allowed CPU range.");
    for (let cpu = first; cpu <= last && cpus.length < limit; cpu++) cpus.push(cpu);
    if (cpus.length === limit) break;
  }
  return cpus.join(",");
}
function parseDependencies(value) {
  const text = value.trim().replace(/^shlibs:Depends=/, "").replace(/\s+/g, " ");
  const atom = "[a-z0-9][a-z0-9+.-]*(?::(?:any|native|amd64))?(?: *\\((?:<<|<=|=|>=|>>) *[0-9A-Za-z.+:~\\-]+\\))?";
  const valid = new RegExp(`^${atom}(?: *\\| *${atom})*$`);
  const values = text.split(",").map((entry) => entry.trim()).filter(Boolean);
  if (!values.length || values.some((entry) => !valid.test(entry))) throw new Error("The dependency file must contain Debian package relationships or one shlibs:Depends= line.");
  return values;
}
async function walk(directory) {
  const files = [];
  for (const entry of (await readdir(directory, { withFileTypes: true })).sort((a, b) => a.name.localeCompare(b.name, "en"))) {
    const filename = path.join(directory, entry.name);
    if (entry.isDirectory()) files.push(...await walk(filename));
    else files.push(filename);
  }
  return files;
}
async function elfHeader(filename, required = true) {
  const handle = await open(filename, "r");
  const bytes = Buffer.alloc(20);
  try { await handle.read(bytes, 0, bytes.length, 0); } finally { await handle.close(); }
  if (bytes.readUInt32BE(0) !== 0x7f454c46) {
    if (required) throw new Error(`Expected an ELF binary: ${filename}`);
    return false;
  }
  if (bytes[4] !== 2 || bytes[5] !== 1 || bytes.readUInt16LE(18) !== 62) throw new Error(`Expected an ELF64 little-endian x86-64 binary: ${filename}`);
  if (required && ![2, 3].includes(bytes.readUInt16LE(16))) throw new Error(`Expected an executable or shared-library ELF, not a relocatable object: ${filename}`);
  return true;
}
async function executable(filename) {
  const info = await stat(filename);
  if (!info.isFile() || !(info.mode & 0o111)) throw new Error(`Required executable is missing or not executable: ${filename}`);
  await elfHeader(filename);
}
async function digest(filename, algorithm = "sha256") {
  const hash = createHash(algorithm);
  const handle = await open(filename, "r");
  try { for await (const chunk of handle.createReadStream()) hash.update(chunk); } finally { await handle.close(); }
  return hash.digest("hex");
}
function verifyDependencies(filename) {
  let output;
  try { output = command("ldd", [filename]); }
  catch (error) {
    output = `${error.stdout || ""}\n${error.stderr || ""}`;
    if (!/not a dynamic executable|statically linked/.test(output)) throw new Error(`ldd failed for ${filename}: ${output.trim()}`);
  }
  if (/\bnot found\b/.test(output)) throw new Error(`Missing runtime libraries for ${filename}:\n${output}`);
  return output;
}
async function normalizeTree(directory) {
  for (const filename of await walk(directory)) {
    const info = await lstat(filename);
    if (info.isSymbolicLink()) {
      const target = await realpath(filename);
      if (!target.startsWith(`${directory}${path.sep}`)) throw new Error(`Bundle symlink escapes its installation directory: ${filename} -> ${await readlink(filename)}`);
    } else if (info.isFile()) await chmod(filename, info.mode & 0o111 ? 0o755 : 0o644);
    else throw new Error(`Unsupported bundle entry: ${filename}`);
  }
  async function directories(current) {
    await chmod(current, 0o755);
    for (const item of await readdir(current, { withFileTypes: true })) if (item.isDirectory()) await directories(path.join(current, item.name));
  }
  await directories(directory);
}

async function main() {
  if (process.argv.slice(2).includes("--help")) { console.log(help); return; }
  const options = parseArgs(process.argv.slice(2));
  if (process.platform !== "linux" || process.arch !== "x64" || command("dpkg", ["--print-architecture"]) !== "amd64") throw new Error("Build this package natively on Ubuntu 22.04 amd64; cross-packaging is not supported.");
  const release = await readFile("/etc/os-release", "utf8");
  if (!/^ID=ubuntu$/m.test(release) || !/^VERSION_ID="?22\.04"?$/m.test(release)) throw new Error("Use Ubuntu 22.04 for the package's glibc 2.35 compatibility baseline.");
  command("dpkg-deb", ["--version"]);
  const supportsThreadLimit = command("dpkg-deb", ["--help"]).includes("--threads-max");
  // Stock Jammy dpkg predates --threads-max. Affinity still caps CPU use at four.
  const compressionCpus = supportsThreadLimit ? null : firstAllowedCpus((await readFile("/proc/self/status", "utf8")).match(/^Cpus_allowed_list:\s*(.+)$/m)?.[1] || "");
  const metadata = JSON.parse(await readFile(path.join(desktop, "package.json"), "utf8"));
  const version = singleLine(options.version || `${metadata.version}-1`, "Version");
  if (!/^[0-9][0-9A-Za-z.+:~\-]*$/.test(version)) throw new Error("Invalid Debian package version.");
  command("dpkg", ["--validate-version", version]);
  const maintainer = singleLine(options.maintainer || "omniTerm contributors <YanYaoyuan@users.noreply.github.com>", "Maintainer");
  const appDirectory = await realpath(path.resolve(options["app-dir"] || path.join(repository, "dist/desktop/omniTerm-linux-x64")));
  let outputDirectory = path.resolve(options["output-dir"] || path.join(repository, "dist/deb"));
  if (outputDirectory === appDirectory || outputDirectory.startsWith(`${appDirectory}${path.sep}`)) throw new Error("Output directory cannot be inside the input application bundle.");
  const agentRoot = path.join(appDirectory, "resources/omniterm-agent");
  const manifest = JSON.parse(await readFile(path.join(agentRoot, "codex-package.json"), "utf8"));
  if (manifest.target !== "x86_64-unknown-linux-gnu" || manifest.variant !== "codex" || manifest.entrypoint !== "bin/codex" || manifest.layoutVersion !== 1) throw new Error("The bundle must include the complete GNU x86-64 Codex package (layout version 1).");
  const binaries = ["omniTerm", "chrome-sandbox", "resources/omniterm-agent/bin/codex", "resources/omniterm-agent/bin/codex-code-mode-host", "resources/omniterm-agent/codex-path/rg", "resources/omniterm-agent/codex-resources/bwrap"];
  for (const relative of binaries) await executable(path.join(appDirectory, relative));
  for (const relative of ["resources/app/package.json", "resources/app/build/main.mjs", "resources/app/build/preload.cjs", "resources/app/build-renderer/index.html", "LICENSE", "LICENSES.chromium.html"]) await stat(path.join(appDirectory, relative));
  const packagedMetadata = JSON.parse(await readFile(path.join(appDirectory, "resources/app/package.json"), "utf8"));
  if (packagedMetadata.name !== metadata.name || packagedMetadata.version !== metadata.version) throw new Error("The application bundle metadata does not match this desktop source; rebuild before packaging.");
  const ptyRoot = "resources/app/node_modules/node-pty";
  let nativePty;
  for (const candidate of ["build/Release/pty.node", "build/Debug/pty.node", "prebuilds/linux-x64/pty.node"]) {
    const relative = `${ptyRoot}/${candidate}`;
    try { await stat(path.join(appDirectory, relative)); } catch (error) { if (error.code === "ENOENT") continue; throw error; }
    await elfHeader(path.join(appDirectory, relative)); nativePty = relative; break;
  }
  if (!nativePty) throw new Error("No Linux x64 node-pty native module was found; run npm run rebuild on the target platform.");
  const dependencies = [...new Set([...defaults, ...(options["depends-file"] ? parseDependencies(await readFile(options["depends-file"], "utf8")) : [])])];
  let sourceCommit = null;
  try { sourceCommit = command("git", ["rev-parse", "HEAD"], { cwd: repository }); } catch { /* Source archives may not include .git. */ }
  const sourceDirty = sourceCommit ? command("git", ["status", "--porcelain", "--untracked-files=normal"], { cwd: repository }) !== "" : null;
  if (sourceDirty && !options["allow-dirty"]) throw new Error("The source checkout has uncommitted changes. Commit them first, or explicitly pass --allow-dirty for a preview package.");
  let epoch = process.env.SOURCE_DATE_EPOCH;
  if (!epoch) { try { epoch = command("git", ["show", "-s", "--format=%ct", "HEAD"], { cwd: repository }); } catch { throw new Error("Set SOURCE_DATE_EPOCH to the source commit timestamp when building without Git metadata."); } }
  if (!/^\d+$/.test(epoch) || !Number.isSafeInteger(Number(epoch))) throw new Error("SOURCE_DATE_EPOCH must be a non-negative integer.");
  await mkdir(outputDirectory, { recursive: true });
  outputDirectory = await realpath(outputDirectory);
  if (outputDirectory === appDirectory || outputDirectory.startsWith(`${appDirectory}${path.sep}`)) throw new Error("Output directory resolves inside the input application bundle.");
  const temporary = await mkdtemp(path.join(outputDirectory, ".omniterm-deb-"));
  const stage = path.join(temporary, "root");
  const installRoot = path.join(stage, "opt/omniterm");
  const packageName = `omniterm_${version}_amd64`;
  try {
    await mkdir(path.dirname(installRoot), { recursive: true });
    await cp(appDirectory, installRoot, { recursive: true, dereference: false, preserveTimestamps: true, verbatimSymlinks: true });
    const prebuilds = path.join(installRoot, ptyRoot, "prebuilds");
    try { for (const name of await readdir(prebuilds)) if (name !== "linux-x64") await rm(path.join(prebuilds, name), { recursive: true, force: true }); } catch (error) { if (error.code !== "ENOENT") throw error; }
    await normalizeTree(installRoot);
    const elfFiles = [];
    for (const filename of await walk(installRoot)) {
      const info = await lstat(filename);
      if (!info.isFile()) continue;
      const isElf = await elfHeader(filename, filename.endsWith(".node"));
      if (isElf) { verifyDependencies(filename); elfFiles.push(path.relative(installRoot, filename)); }
    }
    for (const directory of ["DEBIAN", "usr/bin", "usr/share/applications", "usr/share/icons/hicolor/1024x1024/apps", "usr/share/doc/omniterm"]) await mkdir(path.join(stage, directory), { recursive: true, mode: 0o755 });
    await writeFile(path.join(stage, "usr/bin/omniterm"), '#!/bin/sh\nunset ELECTRON_RUN_AS_NODE\nexec /opt/omniterm/omniTerm "$@"\n', { mode: 0o755 });
    await writeFile(path.join(stage, "usr/share/applications/omniterm.desktop"), `[Desktop Entry]\nType=Application\nName=omniTerm\nComment=Terminal workspace with a local Codex Agent\nExec=/usr/bin/omniterm\nTryExec=/usr/bin/omniterm\nIcon=omniterm\nTerminal=false\nCategories=System;TerminalEmulator;\nKeywords=terminal;ssh;sftp;serial;agent;codex;\nStartupNotify=true\n`, { mode: 0o644 });
    await cp(path.join(desktop, "assets/icon.png"), path.join(stage, "usr/share/icons/hicolor/1024x1024/apps/omniterm.png"));
    for (const name of ["LICENSE", "NOTICE", "THIRD_PARTY_NOTICES.txt"]) await cp(path.join(desktop, name), path.join(stage, "usr/share/doc/omniterm", name));
    const cacheScript = '#!/bin/sh\nset -e\nif command -v update-desktop-database >/dev/null 2>&1; then update-desktop-database -q /usr/share/applications || true; fi\nif command -v gtk-update-icon-cache >/dev/null 2>&1; then gtk-update-icon-cache -q -t /usr/share/icons/hicolor || true; fi\nexit 0\n';
    for (const name of ["postinst", "postrm"]) await writeFile(path.join(stage, "DEBIAN", name), cacheScript, { mode: 0o755 });
    const installedSize = Number(command("du", ["-sk", stage]).split(/\s+/)[0]);
    const control = `Package: omniterm\nVersion: ${version}\nSection: utils\nPriority: optional\nArchitecture: amd64\nMaintainer: ${maintainer}\nInstalled-Size: ${installedSize}\nDepends: ${dependencies.join(", ")}\nRecommends: fonts-noto-cjk\nHomepage: https://github.com/YanYaoyuan/omniTerm\nDescription: Desktop terminal workspace with a local Codex Agent\n Includes Electron, a native PTY module, and the source-built GNU x86-64 Agent.\n SSH, SFTP, port forwarding, and serial clients are supplied through dependencies.\n`;
    await writeFile(path.join(stage, "DEBIAN/control"), control, { mode: 0o644 });
    const sums = [];
    for (const filename of await walk(stage)) {
      const relative = path.relative(stage, filename);
      if (relative.startsWith("DEBIAN/") || !(await lstat(filename)).isFile()) continue;
      sums.push(`${await digest(filename, "md5")}  ${relative}`);
    }
    await writeFile(path.join(stage, "DEBIAN/md5sums"), `${sums.join("\n")}\n`, { mode: 0o644 });
    // Normalize the entire staging tree, including /opt and /usr parents, even
    // when the builder has a restrictive umask. Apply the one setuid bit last.
    await normalizeTree(stage);
    await chmod(path.join(installRoot, "chrome-sandbox"), 0o4755);
    const temporaryDeb = path.join(temporary, `${packageName}.deb`);
    const debArguments = ["--root-owner-group", "-Zxz", "-z6", "--build", stage, temporaryDeb];
    if (supportsThreadLimit) debArguments.unshift("--threads-max=4");
    command(supportsThreadLimit ? "dpkg-deb" : "taskset", supportsThreadLimit ? debArguments : ["--cpu-list", compressionCpus, "dpkg-deb", ...debArguments], { env: { ...process.env, SOURCE_DATE_EPOCH: epoch }, stdio: "inherit" });
    const contents = command("dpkg-deb", ["--contents", temporaryDeb]);
    if (!contents.split("\n").some((line) => /^-rwsr-xr-x\s+root\/root\s/.test(line) && line.endsWith("./opt/omniterm/chrome-sandbox"))) throw new Error("The generated archive does not encode chrome-sandbox as root:root mode 4755.");
    const unexpectedSetId = contents.split("\n").filter((line) => /[sS]/.test(line.slice(0, 10)) && !line.endsWith("./opt/omniterm/chrome-sandbox"));
    if (unexpectedSetId.length) throw new Error("Unexpected set-id files in package contents.");
    const result = path.join(outputDirectory, `${packageName}.deb`);
    const hashes = {};
    for (const relative of [...binaries, nativePty]) hashes[relative] = await digest(path.join(installRoot, relative));
    const report = { package: "omniterm", version, architecture: "amd64", target: "Ubuntu 22.04", sourceCommit, sourceDirty, allowDirty: Boolean(options["allow-dirty"]), sourceDateEpoch: Number(epoch), compression: supportsThreadLimit ? { threadsMax: 4 } : { cpuAffinity: compressionCpus }, electronVersion: metadata.devDependencies.electron, agentTarget: manifest.target, dependencies, nativePty, elfFiles, sha256: hashes, debSha256: await digest(temporaryDeb) };
    await rename(temporaryDeb, result);
    await writeFile(path.join(outputDirectory, `${packageName}.sha256`), `${report.debSha256}  ${path.basename(result)}\n`);
    await writeFile(path.join(outputDirectory, `${packageName}.build.json`), `${JSON.stringify(report, null, 2)}\n`);
    console.log(`Created ${result}\nSHA-256: ${report.debSha256}\nValidated ${elfFiles.length} x86-64 ELF files. Install and test as a normal user before distributing.`);
  } finally { await rm(temporary, { recursive: true, force: true }); }
}

export { command, parseArgs, firstAllowedCpus, parseDependencies, elfHeader, digest };

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch((error) => { console.error(`package-deb: ${error.message}`); process.exitCode = 1; });
}
