import type { ColorRule } from "./log-colors";
export type LogKind =
  | "error"
  | "warning"
  | "success"
  | "dms"
  | "adas"
  | "tsr"
  | "timing"
  | "network"
  | "metric"
  | "timestamp";
export type LogPart = {
  text: string;
  kind?: LogKind;
  color?: string;
  match: boolean;
};
export type LogEvent = {
  seq: number;
  direction: string;
  text: string;
  decoded?: string;
};
export type LogLine<T extends LogEvent> = T & {
  content: string;
  lastSeq: number;
};

export function stripAnsi(text: string): string {
  return text
    .replace(/\x1b\[[0-?]*[ -/]*[@-~]/g, "")
    .replace(/\x1b\][^\x07]*(?:\x07|\x1b\\)/g, "");
}

// Serial packets are not log lines. Join split RX lines before classification/search.
export function logLines<T extends LogEvent>(events: T[]): LogLine<T>[] {
  const lines: LogLine<T>[] = [];
  let pending: LogLine<T> | undefined;
  let previousSeq: number | undefined;
  for (const event of events) {
    if (previousSeq !== undefined && event.seq > previousSeq + 1)
      pending = undefined;
    previousSeq = event.seq;
    const text = event.decoded ?? event.text;
    if (event.direction !== "rx") {
      lines.push({ ...event, content: stripAnsi(text), lastSeq: event.seq });
      continue;
    }
    const pieces = text.split(/\r\n|\n|\r/);
    for (let i = 0; i < pieces.length; i++) {
      if (pieces[i]) {
        if (!pending) {
          pending = { ...event, content: "", lastSeq: event.seq };
          lines.push(pending);
        }
        pending.content += pieces[i];
        pending.lastSeq = event.seq;
      }
      if (i < pieces.length - 1) pending = undefined;
    }
  }
  return lines
    .map((line) => ({ ...line, content: stripAnsi(line.content) }))
    .filter((line) => line.content.length > 0);
}

const hardError =
  /\b(?:fatal|panic|oops)\b|May Be Dead|Abnormal AP exit|write error|Invalid argument|\berror\b(?!\s*[=:]\s*0\b)/i;
const warning =
  /\b(?:warn(?:ing)?|failed|failure|retry|retries)\b|\btime[ -]?out\b(?!\s*[=:]\s*0\b)|invalid register|CRC.*(?:mismatch|fail)|wait stop/i;

export function severity(text: string): "error" | "warning" | "normal" {
  if (hardError.test(text)) return "error";
  const rate = /detectRate\[(\d+)\]\s*\/\s*(\d+)/i.exec(text);
  if (warning.test(text) || (rate && Number(rate[1]) < Number(rate[2])))
    return "warning";
  return "normal";
}

const tokenRules: [LogKind, RegExp][] = [
  [
    "error",
    /May Be Dead|Abnormal AP exit|write error|Invalid argument|\b(?:fatal|panic|oops|error)\b(?!\s*[=:]\s*0\b)/gi,
  ],
  [
    "warning",
    /invalid register\s+\w+|\b(?:warn(?:ing)?|failed|failure|retry|retries)\b|\btime[ -]?out\b(?!\s*[=:]\s*0\b)|wait stop/gi,
  ],
  [
    "success",
    /\b(?:success(?:ful(?:ly)?)?|succeed(?:ed)?|init ok|stop ok|OK)\b/gi,
  ],
  ["dms", /\[(?:DMS|FR|HMS|HOD)\]|\b(?:DMS|FR|HMS|HOD)\b/gi],
  ["adas", /\[(?:ADAS|HWS)\]|\b(?:ADAS|HWS)\b/gi],
  ["tsr", /\[(?:HW_TSR|TSR|TD|td_detect)\]|\b(?:HW_TSR|TSR|td_detect)\b/gi],
  ["timing", /\[AI_TIMING\]|\b(?:AI_TIMING|kflow_ai|npu_status)\b/gi],
  [
    "network",
    /\b(?:TryFindHost|TryFind|PowerOn|PowerOff|GPS|Gps|4G|USB|RTSP|connecting|register|mcumng|netmng|HowenHardware)\b/gi,
  ],
  [
    "metric",
    /\b(?:calls|mean_us|max_us|frame|objects?|category|score|total_us|infer_us|views|speed|ret|res|nRet|rtt|acc)\s*[=:]?\s*-?\d+(?:\.\d+)?(?:ms)?|\b(?:Detect|yuv|chn)\s*[([]\d+[)\]]|\bdetectRate\[\d+\]\s*\/\s*\d+/gi,
  ],
  ["timestamp", /\[\d{8}-\d{6}\]|\b\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\b/gi],
];

// Return text segments, never HTML. Rendering uses textContent for device-controlled data.
export function logParts(
  text: string,
  search = "",
  options: { rules?: ColorRule[]; builtins?: boolean } = {},
): LogPart[] {
  const ranges: {
    start: number;
    end: number;
    kind?: LogKind;
    color?: string;
    match?: boolean;
    priority: number;
  }[] = [];
  for (const [priority, [kind, pattern]] of (options.builtins === false
    ? []
    : tokenRules
  ).entries()) {
    pattern.lastIndex = 0;
    for (const match of text.matchAll(pattern)) {
      let actualKind = kind;
      if (kind === "metric" && /^detectRate/i.test(match[0])) {
        const values = match[0].match(/\d+/g)!;
        if (Number(values[0]) < Number(values[1])) actualKind = "warning";
      }
      ranges.push({
        start: match.index!,
        end: match.index! + match[0].length,
        kind: actualKind,
        priority,
      });
    }
  }
  for (const [index, rule] of (options.rules ?? []).entries()) {
    const keyword = rule.keyword.trim();
    if (!rule.enabled || !keyword || !/^#[0-9a-f]{6}$/i.test(rule.color))
      continue;
    // Keywords are literal text, including regex metacharacters such as [DMS].
    const escaped = keyword.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    const pattern = new RegExp(escaped, rule.caseSensitive ? "gu" : "giu");
    for (const match of text.matchAll(pattern)) {
      ranges.push({
        start: match.index!,
        end: match.index! + match[0].length,
        color: rule.color,
        priority: index - 1000,
      });
    }
  }
  if (search) {
    const lower = text.toLowerCase(),
      needle = search.toLowerCase();
    for (
      let index = lower.indexOf(needle);
      index >= 0;
      index = lower.indexOf(needle, index + needle.length)
    ) {
      ranges.push({
        start: index,
        end: index + needle.length,
        match: true,
        priority: -1,
      });
    }
  }
  const boundaries = [
    ...new Set([0, text.length, ...ranges.flatMap((r) => [r.start, r.end])]),
  ].sort((a, b) => a - b);
  const parts: LogPart[] = [];
  for (let i = 0; i < boundaries.length - 1; i++) {
    const start = boundaries[i],
      end = boundaries[i + 1];
    if (start === end) continue;
    const active = ranges
      .filter((r) => r.start <= start && r.end >= end)
      .sort((a, b) => a.priority - b.priority);
    const colored = active.find((r) => r.kind || r.color);
    parts.push({
      text: text.slice(start, end),
      kind: colored?.kind,
      color: colored?.color,
      match: active.some((r) => r.match),
    });
  }
  return parts;
}
