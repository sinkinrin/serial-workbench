import test from "node:test";
import assert from "node:assert/strict";
import { visibleText } from "../src/command-text.ts";
test("empty-looking control commands and exact line endings remain visible", () => {
  assert.equal(visibleText("\r"), "⟨CR⟩");
  assert.equal(visibleText("date\r\n"), "date⟨CRLF⟩");
  assert.equal(visibleText("\x03"), "⟨0x03⟩");
  assert.equal(visibleText("<tag>"), "<tag>");
});
