// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
import assert from "node:assert/strict";
import { EventEmitter, once } from "node:events";
import { spawn, spawnSync } from "node:child_process";
import test from "node:test";
import { fileURLToPath } from "node:url";
import {
  findExecutable,
  main,
  planCommand,
  resolveCodex,
  resolvePython,
  resolveShell,
  runChild,
} from "../src/cli.js";

const cliPath = fileURLToPath(new URL("../bin/omniterm.js", import.meta.url));

function environment(files, overrides = {}) {
  return {
    platform: "linux",
    env: { PATH: "/tools" },
    repositoryRoot: "/repo",
    nodeExecutable: "/tools/node",
    isExecutable: (filename) => files.includes(filename),
    isFile: (filename) => files.includes(filename),
    ...overrides,
  };
}

test("agent uses explicit executable and preserves all argument boundaries", () => {
  const args = [
    "exec",
    "hello world",
    "$(echo unsafe)",
    "; rm -rf something",
    "--config",
    'model="test"',
  ];
  const result = planCommand(
    "agent",
    args,
    environment(
      [
        "/custom/codex",
        "/repo/dist/omniterm-agent/bin/codex",
        "/repo/codex-rs/target/debug/codex",
        "/tools/codex",
      ],
      { env: { OMNITERM_CODEX_BIN: "/custom/codex", PATH: "/tools" } },
    ),
  );
  assert.deepEqual(result, { executable: "/custom/codex", args });
});

test("explicit missing Agent override fails instead of silently using another binary", () => {
  assert.throws(
    () =>
      resolveCodex(
        environment(["/tools/codex"], {
          env: { OMNITERM_CODEX_BIN: "/missing/codex", PATH: "/tools" },
        }),
      ),
    /OMNITERM_CODEX_BIN is not executable/,
  );
});

test("Agent resolution prefers the packaged build, raw debug/release builds, then PATH", () => {
  const files = [
    "/repo/dist/omniterm-agent/bin/codex",
    "/repo/codex-rs/target/debug/codex",
    "/repo/codex-rs/target/release/codex",
    "/tools/codex",
  ];
  for (let index = 0; index < files.length; index++) {
    assert.equal(
      resolveCodex(environment(files.slice(index))).executable,
      files[index],
    );
  }
});

test("the packaged Agent is available without any Codex on PATH", () => {
  const executable = "/repo/dist/omniterm-agent/bin/codex";
  const options = environment([executable], { env: { PATH: "" } });
  assert.deepEqual(planCommand("agent", ["--version"], options), {
    executable,
    args: ["--version"],
  });
});

test("Windows packaged Agent paths with spaces remain a single executable path", () => {
  const executable =
    "C:\\my projects\\omniTerm\\dist\\omniterm-agent\\bin\\codex.exe";
  const options = environment([executable], {
    platform: "win32",
    env: { Path: "" },
    repositoryRoot: "C:\\my projects\\omniTerm",
  });
  assert.deepEqual(
    planCommand("agent", ["exec", "explain this project"], options),
    {
      executable,
      args: ["exec", "explain this project"],
    },
  );
});

test("Windows npm Codex entrypoint runs via Node without using a .cmd shell", () => {
  const launcher = "C:\\tools\\node_modules\\@openai\\codex\\bin\\codex.js";
  const result = resolveCodex(
    environment([launcher, "C:\\tools\\codex.cmd"], {
      platform: "win32",
      env: { Path: "C:\\tools" },
      repositoryRoot: "C:\\repo",
      nodeExecutable: "C:\\tools\\node.exe",
    }),
  );
  assert.deepEqual(result, {
    executable: "C:\\tools\\node.exe",
    args: [launcher],
  });
});

test("Agent .js override is supported and shell script overrides are rejected", () => {
  const options = environment(["/agent/codex.js"], {
    env: { OMNITERM_CODEX_BIN: "/agent/codex.js" },
  });
  assert.deepEqual(resolveCodex(options), {
    executable: "/tools/node",
    args: ["/agent/codex.js"],
  });
  assert.throws(
    () =>
      resolveCodex({ ...options, env: { OMNITERM_CODEX_BIN: "codex.cmd" } }),
    /not a .cmd/,
  );
});

test("Windows lookup handles Path casing and prefers executables", () => {
  const options = environment(["C:\\tools\\ssh.exe", "C:\\tools\\ssh.cmd"], {
    platform: "win32",
    env: { Path: "C:\\tools" },
  });
  assert.equal(findExecutable("ssh", options), "C:\\tools\\ssh.exe");
  assert.equal(findExecutable("missing", options), undefined);
});

test("SSH and SFTP pass native options and remote expressions through unchanged", () => {
  const options = environment(["/tools/ssh", "/tools/sftp"]);
  for (const command of ["ssh", "sftp"]) {
    const args = ["-i", "/my key", "user@host", "printf '%s' '$HOME'"];
    assert.deepEqual(planCommand(command, args, options), {
      executable: `/tools/${command}`,
      args,
    });
  }
});

test("tunnels require forwarding and add the correct no-command/failure flags", () => {
  const options = environment(["/tools/ssh"]);
  assert.throws(
    () => planCommand("tunnel", ["host"], options),
    /requires -L, -R, or -D/,
  );
  for (const args of [
    ["-L", "8080:localhost:80", "host"],
    ["-R9090:localhost:90", "host"],
    ["-D", "1080", "host"],
  ]) {
    assert.deepEqual(planCommand("tunnel", args, options), {
      executable: "/tools/ssh",
      args: ["-N", "-o", "ExitOnForwardFailure=yes", ...args],
    });
  }
});

test("Unix shell uses SHELL or /bin/sh, and explicit arguments replace interactive flags", () => {
  const options = environment(["/bin/zsh", "/bin/sh"], {
    env: { SHELL: "/bin/zsh" },
  });
  assert.deepEqual(resolveShell(options), {
    executable: "/bin/zsh",
    args: ["-i"],
  });
  assert.deepEqual(resolveShell({ ...options, env: {} }), {
    executable: "/bin/sh",
    args: ["-i"],
  });
  assert.deepEqual(planCommand("shell", ["-c", "exit 4"], options), {
    executable: "/bin/zsh",
    args: ["-c", "exit 4"],
  });
});

test("Windows shells prefer PowerShell and fall back to COMSPEC", () => {
  const options = environment(
    ["C:\\tools\\pwsh.exe", "C:\\Windows\\System32\\cmd.exe"],
    {
      platform: "win32",
      env: { Path: "C:\\tools", ComSpec: "C:\\Windows\\System32\\cmd.exe" },
    },
  );
  assert.deepEqual(resolveShell(options), {
    executable: "C:\\tools\\pwsh.exe",
    args: ["-NoLogo"],
  });
  assert.deepEqual(
    resolveShell({ ...options, env: { ComSpec: options.env.ComSpec } }),
    { executable: options.env.ComSpec, args: [] },
  );
});

test("serial falls back to py -3 on Windows and preserves device and baud arguments", () => {
  const options = environment(["C:\\tools\\py.exe"], {
    platform: "win32",
    env: { Path: "C:\\tools" },
  });
  const python = resolvePython(options);
  assert.deepEqual(python, { executable: "C:\\tools\\py.exe", args: ["-3"] });
  assert.deepEqual(planCommand("serial", ["COM3", "115200"], options), {
    executable: python.executable,
    args: ["-3", "-m", "serial.tools.miniterm", "COM3", "115200"],
    python,
  });
});

test("Windows Python prefers the activated virtual environment over PATH and py", () => {
  const python = "C:\\my project\\.venv\\Scripts\\python.exe";
  const options = environment(
    [python, "C:\\tools\\python.exe", "C:\\tools\\py.exe"],
    {
      platform: "win32",
      env: { Path: "C:\\tools", VIRTUAL_ENV: "C:\\my project\\.venv" },
    },
  );
  assert.deepEqual(resolvePython(options), { executable: python, args: [] });
});

test("Windows Python uses PATH python before the global py launcher", () => {
  const options = environment(
    ["C:\\venv\\Scripts\\python.exe", "C:\\Windows\\py.exe"],
    {
      platform: "win32",
      env: { Path: "C:\\venv\\Scripts;C:\\Windows" },
    },
  );
  assert.deepEqual(resolvePython(options), {
    executable: "C:\\venv\\Scripts\\python.exe",
    args: [],
  });
});

test("missing command dependencies produce actionable failures", async () => {
  let error = "";
  const options = {
    ...environment([]),
    errorOutput: (message) => {
      error += message;
    },
  };
  assert.equal(await main(["ssh", "host"], options), 1);
  assert.match(error, /Install the OpenSSH client/);
  error = "";
  assert.equal(await main(["serial", "COM3"], options), 1);
  assert.match(error, /Install Python 3 and pyserial/);
});

test("pyserial is checked before launching a serial connection", async () => {
  let error = "";
  let launches = 0;
  const status = await main(["serial", "/dev/ttyUSB0", "115200"], {
    ...environment(["/tools/python3"]),
    probe: () => ({ status: 1 }),
    spawnChild: () => {
      launches++;
    },
    errorOutput: (message) => {
      error += message;
    },
  });
  assert.equal(status, 1);
  assert.equal(launches, 0);
  assert.match(error, /pip install pyserial/);
});

test("doctor checks all capabilities, reports missing ones, and never opens connections", async () => {
  let output = "";
  const options = {
    ...environment([
      "/tools/codex",
      "/bin/sh",
      "/tools/ssh",
      "/tools/sftp",
      "/tools/python3",
    ]),
    probe: () => ({ status: 0 }),
    output: (message) => {
      output += message;
    },
    spawnChild: () => {
      throw new Error("doctor must not launch interactive commands");
    },
  };
  assert.equal(await main(["doctor"], options), 0);
  assert.match(output, /OK\s+Agent/);
  assert.match(output, /OK\s+Serial/);
  output = "";
  assert.equal(await main(["doctor"], { ...options, ...environment([]) }), 1);
  assert.match(output, /MISSING\s+Agent/);
});

test("help and version work without external dependencies", () => {
  for (const args of [[], ["--help"], ["--version"]]) {
    const result = spawnSync(process.execPath, [cliPath, ...args], {
      encoding: "utf8",
      env: { ...process.env, PATH: "" },
    });
    assert.equal(result.status, 0, result.stderr);
    assert.match(result.stdout, /omniTerm/);
  }
});

test("actual subprocess exit codes and literal arguments reach the caller", () => {
  const env = { ...process.env, OMNITERM_CODEX_BIN: process.execPath };
  const exit = spawnSync(
    process.execPath,
    [cliPath, "agent", "-e", "process.exit(17)"],
    { encoding: "utf8", env },
  );
  assert.equal(exit.status, 17, exit.stderr);
  const literal = "$(echo unexpected); spaces & quotes ' \"";
  const echo = spawnSync(
    process.execPath,
    [
      cliPath,
      "agent",
      "-e",
      "process.stdout.write(process.argv[1])",
      "--",
      literal,
    ],
    { encoding: "utf8", env },
  );
  assert.equal(echo.status, 0, echo.stderr);
  assert.equal(echo.stdout, literal);
});

test("child inherits stdio without a shell; process signals are forwarded and listeners cleaned up", async () => {
  const child = new EventEmitter();
  const signals = new EventEmitter();
  const kills = [];
  child.kill = (signal) => {
    kills.push(signal);
    child.killed = true;
  };
  const result = runChild(
    { executable: "/tools/ssh", args: ["host"] },
    {
      env: { PATH: "/tools" },
      platform: "linux",
      signals,
      spawnChild: (executable, args, options) => {
        assert.equal(executable, "/tools/ssh");
        assert.deepEqual(args, ["host"]);
        assert.deepEqual(options, {
          stdio: "inherit",
          shell: false,
          env: { PATH: "/tools" },
        });
        return child;
      },
    },
  );
  signals.emit("SIGINT");
  signals.emit("SIGINT");
  signals.emit("SIGTERM");
  assert.deepEqual(kills, ["SIGINT", "SIGINT", "SIGTERM"]);
  child.emit("exit", null, "SIGTERM");
  assert.equal(await result, 143);
  assert.equal(signals.listenerCount("SIGTERM"), 0);
});

test("Windows leaves console SIGINT to the child but forwards termination and cleans listeners", async () => {
  const child = new EventEmitter();
  const signals = new EventEmitter();
  const kills = [];
  child.kill = (signal) => {
    kills.push(signal);
    child.killed = true;
  };
  const result = runChild(
    { executable: "C:\\tools\\codex.exe", args: [] },
    {
      platform: "win32",
      signals,
      spawnChild: () => child,
    },
  );
  signals.emit("SIGINT");
  signals.emit("SIGINT");
  assert.deepEqual(kills, []);
  signals.emit("SIGTERM");
  assert.deepEqual(kills, ["SIGTERM"]);
  child.emit("exit", 0, null);
  assert.equal(await result, 0);
  for (const signal of ["SIGINT", "SIGTERM", "SIGHUP"]) {
    assert.equal(signals.listenerCount(signal), 0);
  }
});

test(
  "a real Unix child that handles SIGINT still receives a later SIGTERM",
  {
    skip: process.platform === "win32",
    timeout: 10000,
  },
  async (context) => {
    const source = `
    process.on("SIGINT", () => process.send("interrupt"));
    process.on("SIGTERM", () => process.exit(0));
    process.send("ready");
    setInterval(() => {}, 1000);
  `;
    const child = spawn(process.execPath, ["-e", source], {
      stdio: ["ignore", "ignore", "ignore", "ipc"],
      shell: false,
    });
    context.after(() => {
      if (child.exitCode === null && child.signalCode === null)
        child.kill("SIGKILL");
    });
    const signals = new EventEmitter();
    const result = runChild(
      { executable: process.execPath, args: [] },
      {
        platform: process.platform,
        signals,
        spawnChild: () => child,
      },
    );
    const deadline = AbortSignal.timeout(5000);
    const [ready] = await once(child, "message", { signal: deadline });
    assert.equal(ready, "ready");
    const interrupted = once(child, "message", { signal: deadline });
    signals.emit("SIGINT");
    assert.equal((await interrupted)[0], "interrupt");
    assert.equal(child.killed, true);
    const exited = once(child, "exit", { signal: deadline });
    signals.emit("SIGTERM");
    await exited;
    assert.equal(await result, 0);
    assert.equal(signals.listenerCount("SIGTERM"), 0);
  },
);

test("spawn errors clean up signal listeners and become a nonzero CLI exit", async () => {
  const signals = new EventEmitter();
  let error = "";
  const result = main(["ssh", "host"], {
    ...environment(["/tools/ssh"]),
    signals,
    errorOutput: (message) => {
      error += message;
    },
    spawnChild: () => {
      const child = new EventEmitter();
      queueMicrotask(() =>
        child.emit(
          "error",
          Object.assign(new Error("executable disappeared"), {
            code: "ENOENT",
          }),
        ),
      );
      return child;
    },
  });
  assert.equal(await result, 127);
  assert.match(error, /executable disappeared/);
  assert.equal(signals.listenerCount("SIGINT"), 0);
});
