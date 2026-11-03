import test from "node:test";
import assert from "node:assert/strict";
import { logLines, logParts, severity } from "../src/log-format.ts";

test("reassembles packet-split UTF-8 text and ANSI tags before highlighting", () => {
  const lines = logLines([
    { seq: 1, direction: "rx", text: "\x1b[3" },
    { seq: 2, direction: "rx", text: "5m[AI_TIMING] AD" },
    { seq: 3, direction: "rx", text: "AS calls=120\r\n[HW_TSR] frame=12" },
    { seq: 4, direction: "rx", text: "700 objects=1\r\n" },
  ]);
  assert.deepEqual(
    lines.map((l) => l.content),
    ["[AI_TIMING] ADAS calls=120", "[HW_TSR] frame=12700 objects=1"],
  );
  assert.equal(lines[0].seq, 1);
  assert.equal(lines[0].lastSeq, 3);
  assert(
    logParts(lines[0].content).some(
      (p) => p.kind === "adas" && p.text === "ADAS",
    ),
  );
});

test("does not join chunks across an omitted history range", () => {
  const lines = logLines([
    { seq: 1, direction: "rx", text: "prefix" },
    { seq: 5, direction: "rx", text: "different line\n" },
  ]);
  assert.equal(lines.length, 2);
});

test("normal modem cycles and zero status fields are not faults", () => {
  for (const text of [
    "4G PowerOff / PowerOn USB re-enumeration",
    "ret=0 res=0 error=0 timeout=0 TimeOut1=0, TimeOut2=0",
    "[CAiBase] DMS detectRate[100] / 100",
  ])
    assert.equal(severity(text), "normal", text);
  assert.equal(severity("TryFind CH3 Host:[IP] failed"), "warning");
  assert.equal(severity("invalid register repsond!"), "warning");
  assert.equal(severity("[CAiBase] TSR detectRate[25] / 120"), "warning");
  assert.equal(severity("AiVideo[1] May Be Dead"), "error");
  assert.equal(severity("sh: write error: Invalid argument"), "error");
});

test("module, timing, counters and a search across token boundaries retain original text", () => {
  const text = "[AI_TIMING] ADAS calls=121 mean_us=13700 max_us=35668";
  const parts = logParts(text, "ADAS calls=121");
  assert.equal(parts.map((p) => p.text).join(""), text);
  assert.equal(
    parts
      .filter((p) => p.match)
      .map((p) => p.text)
      .join(""),
    "ADAS calls=121",
  );
  assert(parts.some((p) => p.kind === "timing"));
  assert(parts.some((p) => p.kind === "adas"));
  assert(parts.some((p) => p.kind === "metric"));
});

test("untrusted device text stays text rather than HTML", () => {
  const text = '<img src=x onerror="alert(1)"> [DMS] Detect(57001)';
  assert.equal(
    logParts(text)
      .map((p) => p.text)
      .join(""),
    text,
  );
});
