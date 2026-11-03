import test from "node:test";
import assert from "node:assert/strict";
import {
  commandLines,
  parseScripts,
  scriptRequest,
  validateScript,
} from "../src/quick-scripts.ts";
const script = (extra = {}) => ({
  id: "test-script",
  name: "设备状态",
  commands: "date\nuptime",
  newline: "crlf",
  intervalMs: 200,
  waitFor: "",
  timeoutMs: 5000,
  ...extra,
});

test("one or multiple commands preserve text and use the selected line ending", () => {
  assert.deepEqual(commandLines("  date\r\n\r\nuptime  \n"), [
    "  date",
    "uptime  ",
  ]);
  const steps = scriptRequest(script({ commands: "  date\n\nuptime  " })).pack
    .macros[0].steps;
  assert.deepEqual(steps, [
    { type: "send", data: "  date\r\n", encoding: "utf8" },
    { type: "delay", ms: 200 },
    { type: "send", data: "uptime  \r\n", encoding: "utf8" },
  ]);
  assert.deepEqual(
    scriptRequest(script({ commands: "help", newline: "none", intervalMs: 0 }))
      .pack.macros[0].steps,
    [{ type: "send", data: "help", encoding: "utf8" }],
  );
});
test("optional response waits follow every command, including the last", () => {
  const steps = scriptRequest(script({ waitFor: "demo> ", intervalMs: 0 })).pack
    .macros[0].steps;
  assert.deepEqual(
    steps.map((s) => s.type),
    ["send", "expect", "send", "expect"],
  );
  assert.equal(steps[1].data, "demo> ");
  assert.equal(steps[1].timeout_ms, 5000);
});
test("saved scripts round-trip and malformed or duplicated records are skipped", () => {
  const s = script();
  assert.deepEqual(
    parseScripts(
      JSON.stringify({
        version: 1,
        scripts: [s, s, { ...s, id: "bad", commands: "\n" }],
      }),
    ),
    [s],
  );
  assert.deepEqual(parseScripts("invalid"), []);
  assert.equal(validateScript(script()), undefined);
  assert(validateScript(script({ intervalMs: -1 })));
  assert(validateScript(script({ commands: "\n \n" })));
});
