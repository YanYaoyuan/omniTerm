import test from "node:test";
import assert from "node:assert/strict";
import { hasOutputLine, printParts } from "../scripts/pty-output.mjs";

test("PTY output markers survive bracketed-paste controls and CRLF", () => {
  const output = "shell$ printf '%s%s\\n' '__OMNITERM_' 'PTY_READY__'\r\n\x1b[?2004l\r__OMNITERM_PTY_READY__\r\n\x1b[?2004hshell$ ";
  assert.equal(hasOutputLine(output, "__OMNITERM_PTY_READY__"), true);
  assert.equal(hasOutputLine("\x1b[32m37 112\x1b[0m\r\n", "37 112"), true);
});

test("PTY command echoes and incomplete markers cannot prove execution", () => {
  const marker = "__OMNITERM_PTY_READY__";
  assert.equal(hasOutputLine(`shell$ printf '${marker}\\n'\r\n`, marker), false);
  assert.equal(hasOutputLine("__OMNITERM_PTY_", marker), false);
  for (const windows of [false, true]) {
    const command = printParts(["__OMNITERM_", "PTY_READY__"], windows);
    assert.equal(command.includes(marker), false);
    // A narrow terminal may wrap anywhere in the command echo.
    for (let split = 1; split < command.length; split++) {
      assert.equal(hasOutputLine(`${command.slice(0, split)}\r\n${command.slice(split)}\r\n`, marker), false);
    }
  }
});
