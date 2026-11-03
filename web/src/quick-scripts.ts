export type ScriptNewline = "crlf" | "lf" | "cr" | "none";
export type QuickScript = {
  id: string;
  name: string;
  commands: string;
  newline: ScriptNewline;
  intervalMs: number;
  waitFor: string;
  timeoutMs: number;
};
export const SCRIPT_STORAGE_KEY = "serial-workbench-quick-scripts-v1";
export const MAX_SCRIPTS = 100;
export function commandLines(commands: string): string[] {
  return commands.split(/\r\n|\n|\r/).filter((line) => line.trim().length > 0);
}
export function validateScript(script: QuickScript): string | undefined {
  if (!script.name.trim()) return "请填写脚本名称";
  if (script.name.length > 64) return "脚本名称最多 64 个字符";
  if (!commandLines(script.commands).length) return "至少填写一条命令";
  if (commandLines(script.commands).length > 100)
    return "每个脚本最多 100 条命令";
  if (script.commands.length > 65536) return "脚本内容超过 64 KiB 字符限制";
  if (!["crlf", "lf", "cr", "none"].includes(script.newline))
    return "请选择有效的行尾";
  if (
    !Number.isInteger(script.intervalMs) ||
    script.intervalMs < 0 ||
    script.intervalMs > 60000
  )
    return "命令间隔须为 0–60000 毫秒的整数";
  if (script.waitFor.length > 256) return "等待文本最多 256 个字符";
  if (
    !Number.isInteger(script.timeoutMs) ||
    script.timeoutMs < 1 ||
    script.timeoutMs > 300000
  )
    return "响应超时须为 1–300000 毫秒的整数";
}
export function parseScripts(raw: string | null): QuickScript[] {
  if (!raw) return [];
  try {
    const value = JSON.parse(raw);
    if (value?.version !== 1 || !Array.isArray(value.scripts)) return [];
    const scripts: QuickScript[] = [],
      ids = new Set<string>();
    for (const item of value.scripts.slice(0, MAX_SCRIPTS)) {
      if (
        !item ||
        typeof item.id !== "string" ||
        !item.id ||
        item.id.length > 64 ||
        ids.has(item.id)
      )
        continue;
      if (
        typeof item.name !== "string" ||
        typeof item.commands !== "string" ||
        typeof item.waitFor !== "string"
      )
        continue;
      const script: QuickScript = {
        id: item.id,
        name: item.name,
        commands: item.commands,
        newline: item.newline,
        intervalMs: item.intervalMs,
        waitFor: item.waitFor,
        timeoutMs: item.timeoutMs,
      };
      if (validateScript(script)) continue;
      ids.add(script.id);
      scripts.push(script);
    }
    return scripts;
  } catch {
    return [];
  }
}
type MacroStep =
  | { type: "send"; data: string; encoding: "utf8" }
  | { type: "delay"; ms: number }
  | {
      type: "expect";
      op: "contains";
      data: string;
      encoding: "utf8";
      timeout_ms: number;
      idle_ms: number;
      max_bytes: number;
      trim: boolean;
    };
export function scriptRequest(script: QuickScript) {
  const error = validateScript(script);
  if (error) throw Error(error);
  const ending = { crlf: "\r\n", lf: "\n", cr: "\r", none: "" }[script.newline];
  const lines = commandLines(script.commands),
    steps: MacroStep[] = [];
  lines.forEach((line, index) => {
    steps.push({ type: "send", data: line + ending, encoding: "utf8" });
    if (script.waitFor)
      steps.push({
        type: "expect",
        op: "contains",
        data: script.waitFor,
        encoding: "utf8",
        timeout_ms: script.timeoutMs,
        idle_ms: 100,
        max_bytes: 1024 * 1024,
        trim: false,
      });
    if (script.intervalMs && index < lines.length - 1)
      steps.push({ type: "delay", ms: script.intervalMs });
  });
  return {
    source: "human",
    name: script.name.trim(),
    assembly: false,
    pack: {
      schema_version: "0.3",
      name: "workbench-quick-scripts",
      macros: [{ name: script.name.trim(), steps }],
      assemblies: [],
    },
  };
}
