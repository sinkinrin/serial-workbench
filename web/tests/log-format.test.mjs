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

import { defaultColors, parseColors } from "../src/log-colors.ts";
const rule = (keyword, color, extra = {}) => ({
  keyword,
  color,
  enabled: true,
  caseSensitive: false,
  ...extra,
});

test("custom literal keywords override built-in colors without regex interpretation", () => {
  const options = {
    rules: [rule("[DMS]", "#123456"), rule("speed=0", "#abcdef")],
  };
  const parts = logParts("[DMS] speed=0 DMS", "", options);
  assert.equal(
    parts
      .filter((p) => p.color === "#123456")
      .map((p) => p.text)
      .join(""),
    "[DMS]",
  );
  assert.equal(
    parts
      .filter((p) => p.color === "#abcdef")
      .map((p) => p.text)
      .join(""),
    "speed=0",
  );
  assert(parts.some((p) => p.kind === "dms" && p.text === "DMS"));
});

test("first rule wins overlaps, with per-rule case matching and enabled state", () => {
  const parts = logParts("ADAS adas", "", {
    builtins: false,
    rules: [
      rule("ADAS", "#112233", { caseSensitive: true }),
      rule("adas", "#445566"),
      rule("AD", "#778899"),
    ],
  });
  assert.equal(
    parts
      .filter((p) => p.color === "#112233")
      .map((p) => p.text)
      .join(""),
    "ADAS",
  );
  assert.equal(
    parts
      .filter((p) => p.color === "#445566")
      .map((p) => p.text)
      .join(""),
    "adas",
  );
  assert(!parts.some((p) => p.color === "#778899"));
  assert(
    logParts("ADAS", "", {
      builtins: false,
      rules: [rule("ADAS", "#112233", { enabled: false })],
    }).every((p) => !p.color && !p.kind),
  );
});

test("custom colors coexist with search marks and default colors can be disabled", () => {
  const text = "连接失败 ERROR";
  const parts = logParts(text, "连接失", {
    builtins: false,
    rules: [rule("连接失败", "#ff8800")],
  });
  assert.equal(parts.map((p) => p.text).join(""), text);
  assert.equal(
    parts
      .filter((p) => p.match)
      .map((p) => p.text)
      .join(""),
    "连接失",
  );
  assert.equal(
    parts
      .filter((p) => p.color)
      .map((p) => p.text)
      .join(""),
    "连接失败",
  );
  assert(parts.every((p) => !p.kind));
});

test("color settings survive JSON persistence and invalid entries are rejected", () => {
  const value = {
    version: 1,
    builtins: false,
    rules: [
      rule("ADAS", "#445566"),
      rule("[DMS]", "#aabbcc", { enabled: false, caseSensitive: true }),
    ],
  };
  assert.deepEqual(parseColors(JSON.stringify(value)), value);
  assert.deepEqual(parseColors("broken"), defaultColors());
  assert.deepEqual(
    parseColors(
      JSON.stringify({
        version: 1,
        rules: [rule("x", "url(https://example.com)"), rule("", "#aabbcc")],
      }),
    ).rules,
    [],
  );
});
