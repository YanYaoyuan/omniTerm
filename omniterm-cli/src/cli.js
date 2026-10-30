// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
import { spawn, spawnSync } from "node:child_process";
import { accessSync, constants, readFileSync, statSync } from "node:fs";
import { constants as osConstants } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const packageRoot = fileURLToPath(new URL("../", import.meta.url));
const version = JSON.parse(
  readFileSync(path.join(packageRoot, "package.json"), "utf8"),
).version;

export const help = `omniTerm ${version} — terminal tools with a Codex Agent

Usage: omniterm <command> [arguments...]

Commands:
  agent [args...]     Run Codex (all arguments are passed through)
  shell [args...]     Open your local shell, or pass arguments to it
  ssh [args...]       Run the system OpenSSH client
  sftp [args...]      Run the system OpenSSH SFTP client
  tunnel [args...]    Run ssh -N with -L, -R, or -D forwarding
  serial [args...]    Run Python pyserial's miniterm
  doctor             Check local command dependencies

Options:
  --help, -h         Show this help
  --version, -v      Show the omniTerm launcher version

Examples:
  omniterm agent "explain this repository"
  omniterm ssh user@host
  omniterm tunnel -L 8080:localhost:80 user@host
  omniterm sftp user@host
  omniterm serial /dev/ttyUSB0 115200

Set OMNITERM_CODEX_BIN to a Codex executable or its .js launcher.
This initial CLI uses your existing terminal; a desktop UI is planned.
`;

function environmentValue(env, key, platform) {
  if (platform !== "win32") return env[key];
  const actualKey = Object.keys(env).find(
    (candidate) => candidate.toLowerCase() === key.toLowerCase(),
  );
  return actualKey === undefined ? undefined : env[actualKey];
}

function usableFile(filename, platform) {
  try {
    if (!statSync(filename).isFile()) return false;
    accessSync(
      filename,
      platform === "win32" ? constants.F_OK : constants.X_OK,
    );
    return true;
  } catch {
    return false;
  }
}

// Deliberately exclude .cmd/.bat: invoking a command shell would change argument
// semantics and allow shell metacharacters in remote commands to be evaluated.
export function findExecutable(
  name,
  {
    env = process.env,
    platform = process.platform,
    isExecutable = usableFile,
  } = {},
) {
  const paths = pathFor(platform);
  const extensions =
    platform === "win32" && !paths.extname(name) ? [".exe", ".com"] : [""];
  const directories = /[/\\]/.test(name)
    ? [""]
    : (environmentValue(env, "PATH", platform) ?? "")
        .split(platform === "win32" ? ";" : ":")
        .filter(Boolean);
  for (const directory of directories) {
    for (const extension of extensions) {
      const filename = directory
        ? paths.join(directory, name + extension)
        : name + extension;
      if (isExecutable(filename, platform)) return filename;
    }
  }
  return undefined;
}

function pathFor(platform) {
  return platform === "win32" ? path.win32 : path.posix;
}

function isFile(filename) {
  try {
    return statSync(filename).isFile();
  } catch {
    return false;
  }
}

function optionsWithDefaults(options) {
  return {
    env: process.env,
    platform: process.platform,
    repositoryRoot: path.resolve(packageRoot, ".."),
    nodeExecutable: process.execPath,
    isFile,
    ...options,
  };
}

function executableCommand(filename, options) {
  if (/\.[cm]?js$/i.test(filename) && options.isFile(filename)) {
    return { executable: options.nodeExecutable, args: [filename] };
  }
  if (/\.(cmd|bat)$/i.test(filename)) {
    throw new Error(
      "Use a native Codex executable or its bin/codex.js launcher, not a .cmd/.bat file.",
    );
  }
  const executable = findExecutable(filename, options);
  return executable ? { executable, args: [] } : undefined;
}

export function resolveCodex(inputOptions = {}) {
  const options = optionsWithDefaults(inputOptions);
  const override = options.env.OMNITERM_CODEX_BIN;
  if (override) {
    const command = executableCommand(override, options);
    if (!command)
      throw new Error(`OMNITERM_CODEX_BIN is not executable: ${override}`);
    return command;
  }
  const paths = pathFor(options.platform);
  const name = options.platform === "win32" ? "codex.exe" : "codex";
  for (const profile of ["debug", "release"]) {
    const command = executableCommand(
      paths.join(options.repositoryRoot, "codex-rs", "target", profile, name),
      options,
    );
    if (command) return command;
  }
  const command = executableCommand("codex", options);
  if (command) return command;

  // npm installs a .cmd shim on Windows. Run its standard JS entrypoint with
  // Node directly instead of routing user arguments through cmd.exe.
  if (options.platform === "win32") {
    for (const directory of (
      environmentValue(options.env, "PATH", "win32") ?? ""
    )
      .split(";")
      .filter(Boolean)) {
      const launcher = paths.join(
        directory,
        "node_modules",
        "@openai",
        "codex",
        "bin",
        "codex.js",
      );
      if (options.isFile(launcher))
        return { executable: options.nodeExecutable, args: [launcher] };
    }
  }
  throw new Error(
    "Codex was not found. Build codex-rs, install Codex, or set OMNITERM_CODEX_BIN to its executable (or bin/codex.js).",
  );
}

export function resolveShell(inputOptions = {}) {
  const options = optionsWithDefaults(inputOptions);
  if (options.platform === "win32") {
    for (const name of [
      "pwsh",
      "powershell",
      environmentValue(options.env, "COMSPEC", "win32"),
      "cmd",
    ]) {
      if (!name) continue;
      const executable = findExecutable(name, options);
      if (executable)
        return {
          executable,
          args: /(?:^|[\\/])(?:pwsh|powershell)(?:\.exe)?$/i.test(executable)
            ? ["-NoLogo"]
            : [],
        };
    }
  } else {
    for (const name of [options.env.SHELL, "/bin/sh"]) {
      if (!name) continue;
      const executable = findExecutable(name, options);
      if (executable) return { executable, args: ["-i"] };
    }
  }
  throw new Error(
    "No local shell was found. Configure SHELL (Unix) or install PowerShell / configure COMSPEC (Windows).",
  );
}

export function resolvePython(inputOptions = {}) {
  const options = optionsWithDefaults(inputOptions);
  if (options.platform === "win32") {
    const virtualEnvironment = environmentValue(
      options.env,
      "VIRTUAL_ENV",
      "win32",
    );
    if (virtualEnvironment) {
      const executable = findExecutable(
        path.win32.join(virtualEnvironment, "Scripts", "python.exe"),
        options,
      );
      if (executable) return { executable, args: [] };
    }
  }
  // The Windows launcher with an explicit -3 selects a global interpreter;
  // prefer the activated environment / PATH interpreter before falling back.
  const names =
    options.platform === "win32"
      ? ["python", "python3", "py"]
      : ["python3", "python"];
  for (const name of names) {
    const executable = findExecutable(name, options);
    if (executable) return { executable, args: name === "py" ? ["-3"] : [] };
  }
  throw new Error(
    "Python 3 was not found. Install Python 3 and pyserial for serial connections.",
  );
}

function requireExecutable(name, options) {
  const executable = findExecutable(name, options);
  if (!executable)
    throw new Error(
      `${name} was not found. Install the OpenSSH client and make it available on PATH.`,
    );
  return executable;
}

export function planCommand(command, args, inputOptions = {}) {
  const options = optionsWithDefaults(inputOptions);
  if (command === "agent") {
    const codex = resolveCodex(options);
    return { executable: codex.executable, args: [...codex.args, ...args] };
  }
  if (command === "shell") {
    const shell = resolveShell(options);
    return {
      executable: shell.executable,
      args: args.length ? args : shell.args,
    };
  }
  if (command === "ssh" || command === "sftp") {
    return { executable: requireExecutable(command, options), args };
  }
  if (command === "tunnel") {
    if (!args.some((arg) => /^-[LRD]/.test(arg))) {
      throw new Error(
        "A tunnel requires -L, -R, or -D. Example: omniterm tunnel -L 8080:localhost:80 user@host",
      );
    }
    return {
      executable: requireExecutable("ssh", options),
      args: ["-N", "-o", "ExitOnForwardFailure=yes", ...args],
    };
  }
  if (command === "serial") {
    const python = resolvePython(options);
    return {
      executable: python.executable,
      args: [...python.args, "-m", "serial.tools.miniterm", ...args],
      python,
    };
  }
  throw new Error(`Unknown command: ${command}. Run omniterm --help.`);
}

function checkPyserial(python, env, probe = spawnSync) {
  const result = probe(
    python.executable,
    [
      ...python.args,
      "-c",
      "import sys; assert sys.version_info.major >= 3; import serial.tools.miniterm",
    ],
    {
      env,
      stdio: "ignore",
      shell: false,
      timeout: 10000,
    },
  );
  return result.status === 0 && !result.error;
}

function pyserialError(python) {
  const interpreter = [python.executable, ...python.args].join(" ");
  return `Python 3 with pyserial is unavailable for ${interpreter}. Install pyserial in that interpreter's environment using -m pip install pyserial, then retry.`;
}

export function runChild(
  plan,
  {
    env = process.env,
    platform = process.platform,
    spawnChild = spawn,
    signals = process,
  } = {},
) {
  return new Promise((resolve, reject) => {
    const child = spawnChild(plan.executable, plan.args, {
      stdio: "inherit",
      shell: false,
      env,
    });
    let exited = false;
    const handlers = new Map();
    for (const signal of ["SIGINT", "SIGTERM", "SIGHUP"]) {
      const handler = () => {
        // Windows delivers Ctrl+C to processes sharing the inherited console.
        // child.kill("SIGINT") there force-terminates the child instead of
        // preserving the shell/Agent's own interrupt handler.
        if (platform === "win32" && signal === "SIGINT") return;
        // child.killed only records whether a signal was sent successfully.
        // An interactive child may handle SIGINT and remain alive.
        if (!exited) child.kill(signal);
      };
      handlers.set(signal, handler);
      signals.on(signal, handler);
    }
    const cleanup = () => {
      exited = true;
      for (const [signal, handler] of handlers)
        signals.removeListener(signal, handler);
    };
    child.once("error", (error) => {
      cleanup();
      reject(error);
    });
    child.once("exit", (code, signal) => {
      cleanup();
      resolve(code ?? 128 + (osConstants.signals[signal] ?? 1));
    });
  });
}

export async function main(argv, inputOptions = {}) {
  const options = optionsWithDefaults(inputOptions);
  const output =
    inputOptions.output ?? ((message) => process.stdout.write(message));
  const errorOutput =
    inputOptions.errorOutput ?? ((message) => process.stderr.write(message));
  const [command, ...rawArgs] = argv;
  if (
    !command ||
    command === "--help" ||
    command === "-h" ||
    command === "help"
  ) {
    output(help);
    return 0;
  }
  if (command === "--version" || command === "-v") {
    output(`omniTerm ${version}\n`);
    return 0;
  }
  try {
    if (command === "doctor") {
      if (rawArgs.length) throw new Error("Usage: omniterm doctor");
      let missing = false;
      output(`omniTerm ${version} dependency check (${options.platform})\n`);
      for (const [label, resolve] of [
        ["Agent", () => resolveCodex(options)],
        ["Shell", () => resolveShell(options)],
        [
          "SSH",
          () => ({ executable: requireExecutable("ssh", options), args: [] }),
        ],
        [
          "SFTP",
          () => ({ executable: requireExecutable("sftp", options), args: [] }),
        ],
        [
          "Serial",
          () => {
            const python = resolvePython(options);
            if (!checkPyserial(python, options.env, options.probe))
              throw new Error(pyserialError(python));
            return python;
          },
        ],
      ]) {
        try {
          const resolved = resolve();
          output(
            `OK      ${label}: ${[resolved.executable, ...resolved.args].join(" ")}\n`,
          );
        } catch (error) {
          missing = true;
          output(`MISSING ${label}: ${error.message}\n`);
        }
      }
      output(
        "Dependencies are checked locally; no remote connections were made.\n",
      );
      return missing ? 1 : 0;
    }
    const args = rawArgs[0] === "--" ? rawArgs.slice(1) : rawArgs;
    const plan = planCommand(command, args, options);
    if (
      plan.python &&
      !checkPyserial(plan.python, options.env, options.probe)
    ) {
      throw new Error(pyserialError(plan.python));
    }
    return await runChild(plan, options);
  } catch (error) {
    errorOutput(`omniTerm: ${error.message}\n`);
    return error.code === "ENOENT" ? 127 : 1;
  }
}
