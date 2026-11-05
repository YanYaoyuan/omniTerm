// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
// Assembles the native app into an unsigned, downloadable artifact beside the desktop ones:
// a ZIP with omniTerm.app on macOS, a portable ZIP on Windows, a .deb and a tarball on Linux.
// The Agent engine the release built (dist/omniterm-agent) ships next to the executable, where
// the app looks for it first; `--without-agent` packages the app alone, for CI checks.
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { chmod, cp, mkdir, readFile, rm, stat, writeFile } from "node:fs/promises";
import { existsSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const native = fileURLToPath(new URL("../", import.meta.url));
const repository = path.resolve(native, "..");
const desktop = path.join(repository, "omniterm-desktop");
// Kept apart from dist/release, whose validator admits only the desktop assets.
const output = path.join(repository, "dist/release-native");
const withoutAgent = process.argv.includes("--without-agent");
const run = (executable, args, options = {}) => execFileSync(executable, args, { cwd: repository, stdio: "inherit", ...options });

const manifest = await readFile(path.join(native, "Cargo.toml"), "utf8");
const version = /^version\s*=\s*"([^"]+)"/m.exec(manifest)?.[1];
const desktopVersion = JSON.parse(await readFile(path.join(desktop, "package.json"), "utf8")).version;
const releaseVersion = desktopVersion ?? version;
if (!/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(releaseVersion ?? "")) throw new Error("Invalid release version");

const target = process.env.CARGO_TARGET_DIR ? path.resolve(process.env.CARGO_TARGET_DIR) : path.join(native, "target");
const windows = process.platform === "win32";
const binary = path.join(target, "release", windows ? "omniterm-app.exe" : "omniterm-app");
if (!existsSync(binary)) throw new Error(`Build the native app first: ${binary} is missing (cargo build --release --manifest-path omniterm-native/Cargo.toml).`);

const agent = path.join(repository, "dist/omniterm-agent");
if (!withoutAgent && !existsSync(path.join(agent, "omniterm-source.json"))) throw new Error("The release Agent is missing from dist/omniterm-agent; pass --without-agent for an app-only package.");

async function sha256(filename) {
  const hash = createHash("sha256");
  hash.update(await readFile(filename));
  return hash.digest("hex");
}

// Every bundled Agent binary must match the hashes its build recorded.
async function verifyAgent(root) {
  const provenance = JSON.parse(await readFile(path.join(root, "omniterm-source.json"), "utf8"));
  const binaries = Object.entries(provenance.binarySha256 ?? {});
  if (!binaries.length) throw new Error("Agent provenance contains no binaries");
  for (const [relative, expected] of binaries) {
    if (path.isAbsolute(relative) || relative.split(/[\\/]/).includes("..")) throw new Error("Invalid Agent provenance path");
    if ((await sha256(path.join(root, relative))) !== expected) throw new Error(`Bundled Agent hash mismatch: ${relative}`);
  }
  return provenance;
}

const notices = async (directory) => {
  await mkdir(directory, { recursive: true });
  for (const name of ["LICENSE", "NOTICE"]) await cp(path.join(repository, name), path.join(directory, name));
  await cp(path.join(native, "assets/Phosphor-LICENSE-MIT.txt"), path.join(directory, "Phosphor-LICENSE-MIT.txt"));
};

const stage = path.join(os.tmpdir(), `omniterm-native-${process.pid}`);
await rm(stage, { recursive: true, force: true });
await mkdir(output, { recursive: true });
const provenance = withoutAgent ? null : await verifyAgent(agent);
const suffix = withoutAgent ? "-app-only" : "";
const artifacts = [];

if (process.platform === "darwin") {
  const app = path.join(stage, "omniTerm.app");
  const contents = path.join(app, "Contents");
  await mkdir(path.join(contents, "MacOS"), { recursive: true });
  await mkdir(path.join(contents, "Resources"), { recursive: true });
  await cp(binary, path.join(contents, "MacOS/omniterm"));
  await cp(path.join(desktop, "assets/icon.icns"), path.join(contents, "Resources/omniTerm.icns"));
  if (!withoutAgent) await cp(agent, path.join(contents, "Resources/omniterm-agent"), { recursive: true });
  await notices(path.join(contents, "Resources/licenses"));
  await writeFile(path.join(contents, "Info.plist"), `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleName</key><string>omniTerm</string>
<key>CFBundleDisplayName</key><string>omniTerm</string>
<key>CFBundleIdentifier</key><string>dev.omniterm.native</string>
<key>CFBundleVersion</key><string>${releaseVersion}</string>
<key>CFBundleShortVersionString</key><string>${releaseVersion}</string>
<key>CFBundleExecutable</key><string>omniterm</string>
<key>CFBundleIconFile</key><string>omniTerm.icns</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>LSMinimumSystemVersion</key><string>11.0</string>
<key>NSHighResolutionCapable</key><true/>
</dict></plist>
`);
  // Ad hoc only: no Developer ID or notarization credentials are used.
  run("/usr/bin/codesign", ["--force", "--deep", "--sign", "-", app]);
  const artifact = path.join(output, `omniTerm-native-${releaseVersion}-macos-${process.arch}${suffix}.zip`);
  run("/usr/bin/ditto", ["-c", "-k", "--sequesterRsrc", "--keepParent", app, artifact]);
  artifacts.push(artifact);
} else if (windows) {
  const root = path.join(stage, "omniTerm");
  await mkdir(root, { recursive: true });
  await cp(binary, path.join(root, "omniTerm.exe"));
  if (!withoutAgent) await cp(agent, path.join(root, "omniterm-agent"), { recursive: true });
  await notices(path.join(root, "licenses"));
  const artifact = path.join(output, `omniTerm-native-${releaseVersion}-windows-x64${suffix}.zip`);
  run("python", ["-c", "import pathlib,sys,zipfile; root=pathlib.Path(sys.argv[1]); out=pathlib.Path(sys.argv[2]); z=zipfile.ZipFile(out,'w',zipfile.ZIP_DEFLATED,compresslevel=6); [z.write(p,p.relative_to(root.parent)) for p in sorted(root.rglob('*')) if p.is_file()]; z.close()", root, artifact]);
  artifacts.push(artifact);
} else {
  // A relocatable tarball, and a .deb that installs under /usr/lib with a launcher on PATH.
  const root = path.join(stage, `omniterm-native-${releaseVersion}`);
  await mkdir(root, { recursive: true });
  await cp(binary, path.join(root, "omniterm"));
  if (!withoutAgent) await cp(agent, path.join(root, "omniterm-agent"), { recursive: true });
  await notices(path.join(root, "licenses"));
  const tarball = path.join(output, `omniTerm-native-${releaseVersion}-linux-x64${suffix}.tar.gz`);
  run("tar", ["-czf", tarball, "-C", stage, path.basename(root)]);
  artifacts.push(tarball);
  const deb = path.join(stage, "deb");
  for (const directory of ["DEBIAN", "usr/bin", "usr/lib/omniterm-native", "usr/share/applications", "usr/share/icons/hicolor/1024x1024/apps", "usr/share/doc/omniterm-native"]) await mkdir(path.join(deb, directory), { recursive: true });
  await cp(root, path.join(deb, "usr/lib/omniterm-native"), { recursive: true });
  await writeFile(path.join(deb, "usr/bin/omniterm-native"), "#!/bin/sh\nexec /usr/lib/omniterm-native/omniterm \"$@\"\n");
  await chmod(path.join(deb, "usr/bin/omniterm-native"), 0o755);
  await writeFile(path.join(deb, "usr/share/applications/omniterm-native.desktop"), "[Desktop Entry]\nType=Application\nName=omniTerm (native)\nComment=Terminal workspace with a local Codex Agent\nExec=/usr/bin/omniterm-native\nIcon=omniterm-native\nTerminal=false\nCategories=System;TerminalEmulator;Development;\nStartupWMClass=omniterm\n");
  await cp(path.join(desktop, "assets/icon.png"), path.join(deb, "usr/share/icons/hicolor/1024x1024/apps/omniterm-native.png"));
  await notices(path.join(deb, "usr/share/doc/omniterm-native"));
  const size = Math.ceil(((await stat(binary)).size + (withoutAgent ? 0 : 200 * 1024 * 1024)) / 1024);
  await writeFile(path.join(deb, "DEBIAN/control"), `Package: omniterm-native\nVersion: ${releaseVersion}-1\nSection: utils\nPriority: optional\nArchitecture: amd64\nMaintainer: omniTerm contributors\nInstalled-Size: ${size}\nDepends: libc6, libxkbcommon0, libvulkan1 | libgl1\nRecommends: openssh-client\nDescription: omniTerm as a native application\n GPU-rendered terminals, SSH connections and files, omniTerm Agent and monitoring.\n`);
  const package_ = path.join(output, `omniterm-native_${releaseVersion}-1_amd64${suffix}.deb`);
  run("dpkg-deb", ["--root-owner-group", "--build", deb, package_]);
  artifacts.push(package_);
}

for (const artifact of artifacts) {
  const filename = path.basename(artifact);
  const checksum = await sha256(artifact);
  await writeFile(`${artifact}.sha256`, `${checksum}  ${filename}\n`);
  await writeFile(`${artifact}.manifest.json`, `${JSON.stringify({ schemaVersion: 1, app: "omniterm-native", version: releaseVersion, platform: process.platform, arch: process.arch, agent: provenance, signed: false }, null, 2)}\n`);
  console.log(`Native artifact: ${artifact}\nSHA-256: ${checksum}`);
}
await rm(stage, { recursive: true, force: true });
console.log("Unsigned preview; no signing or notarization credentials were used.");
