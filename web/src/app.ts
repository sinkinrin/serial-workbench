import { initQuickScripts } from "./script-panel";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "./style.css";
import { logLines, logParts, severity } from "./log-format";
import {
  COLOR_STORAGE_KEY,
  MAX_COLOR_RULES,
  defaultColors,
  parseColors,
} from "./log-colors";

type Session = {
  id: string;
  config: {
    port: string;
    baud_rate: number;
    data_bits: string;
    stop_bits: string;
    parity: string;
    flow_control: string;
  };
  demo: boolean;
  state: string;
  started_at: string;
  rx_bytes: number;
  tx_bytes: number;
  last_rx_at: string | null;
  active_task: string | null;
  control_owner: string | null;
  last_error: string | null;
  directory: string;
  next_seq: number;
};
type SerialEvent = {
  seq: number;
  time: string;
  direction: string;
  source: string;
  text: string;
  base64: string;
  bytes: number;
  task_id: string | null;
  decoded?: string;
};
type Page = {
  events: SerialEvent[];
  next_cursor: number;
  gap: boolean;
  first_available: number;
  has_more: boolean;
};
type Task = {
  id: string;
  session_id: string;
  source: string;
  label: string;
  status: string;
  created_at: string;
  finished_at: string | null;
  message: string | null;
};
const $ = <T extends HTMLElement = HTMLElement>(id: string) =>
  document.getElementById(id) as T;
const input = (id: string) => $(id) as HTMLInputElement;
const select = (id: string) => $(id) as HTMLSelectElement;
const button = (id: string) => $(id) as HTMLButtonElement;
const labels: Record<string, string> = {
  connected: "采集中",
  closed: "已关闭",
  interrupted: "后台已中断",
  error: "连接异常",
  queued: "排队中",
  running: "执行中",
  sent: "已发送",
  succeeded: "条件已满足",
  completed: "流程完成",
  failed: "失败",
  cancelled: "已取消",
  human: "你",
  cli: "CLI",
  mcp: "AI / MCP",
  device: "设备",
  daemon: "后台",
};
let sessions: Session[] = [],
  current: Session | undefined,
  selected = localStorage.getItem("serial-session") || "";
let events: SerialEvent[] = [],
  cursor = 0,
  historyCursor = 0,
  historyMode = false,
  paused = false,
  ws: WebSocket | undefined,
  reconnect: number | undefined,
  revision = 0;
let renderPending = false,
  terminalMode = false,
  decoder = new TextDecoder(),
  toastTimer: number | undefined,
  macroPack: any = null;
let updateScriptConnection = () => {};
let colorSettings = defaultColors();
try {
  colorSettings = parseColors(localStorage.getItem(COLOR_STORAGE_KEY));
} catch {
  /* Settings still work for this page if storage is blocked. */
}
let windowDropped = 0,
  pausedCount = 0;
const terminal = new Terminal({
  fontFamily: '"Cascadia Code", Consolas, "Microsoft YaHei", monospace',
  fontSize: 12,
  lineHeight: 1.35,
  cursorBlink: true,
  scrollback: 3000,
  convertEol: false,
  theme: {
    background: "#142126",
    foreground: "#c8dbd4",
    cursor: "#81c6ac",
    selectionBackground: "#426359",
    black: "#142126",
    green: "#83cfa3",
    red: "#e88c7d",
    yellow: "#e2c48b",
  },
});
const fit = new FitAddon();
terminal.loadAddon(fit);
terminal.open($("terminal"));
new ResizeObserver(() => {
  if (terminalMode) fit.fit();
}).observe($("terminal"));
function toast(message: string, error = false) {
  $("toast").textContent = message;
  $("toast").className = `toast${error ? " error" : ""}`;
  clearTimeout(toastTimer);
  toastTimer = window.setTimeout(
    () => $("toast").classList.add("hidden"),
    error ? 9000 : 4000,
  );
}
async function api<T>(path: string, body?: unknown): Promise<T> {
  const response = await fetch(
    `/api${path}`,
    body === undefined
      ? {}
      : {
          method: "POST",
          headers: {
            "Content-Type": "application/json",
            "X-Serial-Workbench": "1",
          },
          body: JSON.stringify(body),
        },
  );
  let value;
  try {
    value = await response.json();
  } catch {
    throw Error(`后台响应异常 (${response.status})`);
  }
  if (!response.ok) throw Error(value.error || `请求失败 (${response.status})`);
  return value;
}
function action(id: string, fn: () => Promise<unknown> | void) {
  button(id).addEventListener("click", () =>
    Promise.resolve()
      .then(fn)
      .catch((e) => toast(e.message, true)),
  );
}
function bytes(n: number) {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1024 / 1024).toFixed(2)} MB`;
}
function time(value: string) {
  return (
    new Date(value).toLocaleTimeString("zh-CN", { hour12: false }) +
    `.${new Date(value).getMilliseconds().toString().padStart(3, "0")}`
  );
}
function notice(text: string) {
  $("notice").textContent = text;
  $("notice").classList.toggle("hidden", !text);
}
function enabled() {
  updateScriptConnection();
  const active = current?.state === "connected";
  for (const id of ["send", "takeover", "disconnect", "set-control", "ctrl-c"])
    button(id).disabled = !active;
  button("export").disabled = !current;
  button("copy-path").disabled = !current;
  button("run-macro").disabled = !active || !macroPack;
  input("direct-input").disabled = !active || historyMode;
  if (current?.control_owner !== "human") input("direct-input").checked = false;
}
function sessionCards() {
  const list = $("sessions");
  list.replaceChildren();
  if (!sessions.length) {
    const p = document.createElement("div");
    p.className = "session-placeholder";
    p.textContent = "还没有会话。连接串口后，实时与历史记录都会保留在这里。";
    list.append(p);
  }
  for (const s of sessions) {
    const b = document.createElement("button");
    b.className = `session-card${selected === s.id ? " active" : ""}`;
    const title = document.createElement("span");
    title.className = "session-card-title";
    title.append(document.createTextNode(s.demo ? "模拟设备" : s.config.port));
    const dot = document.createElement("i");
    dot.className = `session-led ${s.state}`;
    title.append(dot);
    const sub = document.createElement("small");
    sub.textContent = `${s.config.baud_rate} baud · ${labels[s.state] || s.state}`;
    const stamp = document.createElement("small");
    stamp.textContent = new Date(s.started_at).toLocaleString("zh-CN", {
      month: "2-digit",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
    });
    b.append(title, sub, stamp);
    b.onclick = () => choose(s.id);
    list.append(b);
  }
}
function header() {
  if (!current) return;
  const s = current;
  $("session-eyebrow").textContent = s.demo
    ? "SIMULATED DEVICE"
    : "SHARED SERIAL SESSION";
  $("session-name").textContent = s.demo ? "模拟设备" : s.config.port;
  $("session-config").textContent =
    `${s.config.baud_rate} baud / ${s.config.data_bits}${s.config.parity[0].toUpperCase()}${s.config.stop_bits} / 流控 ${s.config.flow_control === "none" ? "无" : s.config.flow_control}`;
  $("session-state").textContent = labels[s.state] || s.state;
  $("session-state").className = `pill ${s.state}`;
  $("rx-total").textContent = bytes(s.rx_bytes);
  $("tx-total").textContent = bytes(s.tx_bytes);
  $("last-received").textContent = s.last_rx_at
    ? new Date(s.last_rx_at).toLocaleTimeString("zh-CN", { hour12: false })
    : "—";
  $("active-task").textContent = s.active_task
    ? "执行中"
    : s.control_owner === "human"
      ? "你持有控制权"
      : "空闲";
  button("takeover").textContent =
    s.control_owner === "human"
      ? "释放控制权 · 允许协作"
      : "接管 · 取消待执行任务";
  $("storage-path").textContent = s.directory.replace(/^\\\\\?\\/, "");
  $("storage-path").title = s.directory;
  $("capture-status").textContent =
    s.state === "connected" ? "● 后台持续采集" : labels[s.state] || s.state;
  enabled();
}
async function refresh() {
  try {
    sessions = await api<Session[]>("/sessions");
    $("service-status").textContent = "后台已连接";
    $("service-dot").classList.add("online");
    sessionCards();
    if (selected && !current && sessions.some((s) => s.id === selected))
      choose(selected);
    else if (current) {
      current = sessions.find((s) => s.id === selected) || current;
      header();
    }
    renderTasks(await api<Task[]>("/tasks"));
  } catch (e) {
    $("service-status").textContent = "后台连接中断";
    $("service-dot").classList.remove("online");
  }
  window.setTimeout(refresh, 1200);
}
function choose(id: string) {
  document.body.classList.remove("sessions-open");
  revision++;
  clearTimeout(keyboardTimer);
  keyboard = "";
  clearTimeout(reconnect);
  if (ws) {
    ws.onclose = null;
    ws.close();
    ws = undefined;
  }
  selected = id;
  localStorage.setItem("serial-session", id);
  current = sessions.find((s) => s.id === id);
  events = [];
  cursor = 0;
  windowDropped = 0;
  pausedCount = 0;
  historyCursor = 0;
  paused = false;
  historyMode = false;
  decoder = new TextDecoder();
  terminal.reset();
  notice("");
  button("pause").textContent = "Ⅱ";
  button("pause").disabled = false;
  $("return-live").classList.add("hidden");
  $("load-more").classList.add("hidden");
  input("direct-input").checked = false;
  header();
  sessionCards();
  render();
  if (current?.state === "connected") connectStream();
  else loadHistory(true).catch((e) => toast(e.message, true));
}
function connectStream() {
  if (!current || historyMode || current.state !== "connected") return;
  const thisRevision = revision;
  const id = current.id;
  ws = new WebSocket(
    `${location.protocol === "https:" ? "wss" : "ws"}://${location.host}/api/sessions/${id}/stream?after=${cursor}`,
  );
  ws.onopen = () => {
    $("stream-state").textContent = "● LIVE";
  };
  ws.onmessage = (message) => {
    if (thisRevision !== revision || historyMode) return;
    const page: Page = JSON.parse(message.data);
    if (page.gap)
      notice(
        `实时缓存缺少序号 ${cursor + 1}–${page.first_available - 1}，可通过“查历史文件”补查。`,
      );
    cursor = page.next_cursor;
    append(page.events);
  };
  ws.onclose = () => {
    if (thisRevision !== revision || historyMode) return;
    $("stream-state").textContent = "连接已断开";
    if (current?.state === "connected")
      reconnect = window.setTimeout(connectStream, 1500);
  };
  ws.onerror = () => {
    $("stream-state").textContent = "重连中…";
  };
}
function raw(event: SerialEvent) {
  try {
    return Uint8Array.from(atob(event.base64), (c) => c.charCodeAt(0));
  } catch {
    return new Uint8Array();
  }
}
function append(items: SerialEvent[]) {
  for (const event of items) {
    if (event.direction === "rx") {
      const data = raw(event);
      event.decoded = decoder.decode(data, { stream: true });
      if (!paused) terminal.write(data);
    } else event.decoded = event.text;
    events.push(event);
  }
  if (events.length > 4000) {
    windowDropped += events.length - 4000;
    events.splice(0, events.length - 4000);
  }
  if (paused) {
    pausedCount += items.length;
    $("event-count").textContent =
      `显示已暂停 · 新增 ${pausedCount} 条 · 后台采集继续`;
    return;
  }
  scheduleRender();
}
function scheduleRender() {
  if (renderPending) return;
  renderPending = true;
  requestAnimationFrame(() => {
    renderPending = false;
    render();
  });
}
function paintLogText(
  payload: HTMLElement,
  content: string,
  direction: string,
  search: string,
  hexMode: boolean,
) {
  payload.replaceChildren();
  if (hexMode) {
    payload.textContent = content;
    return;
  }
  for (const part of logParts(content, search, {
    rules: colorSettings.rules,
    builtins: colorSettings.builtins && direction === "rx",
  })) {
    if (!part.kind && !part.color && !part.match)
      payload.append(document.createTextNode(part.text));
    else {
      const token = document.createElement(part.match ? "mark" : "span");
      token.className = part.color
        ? "custom-token"
        : part.kind
          ? `token-${part.kind}`
          : "";
      if (part.color) token.style.color = part.color;
      token.textContent = part.text;
      payload.append(token);
    }
  }
}
function repaintColors() {
  if (!paused) {
    scheduleRender();
    return;
  }
  // A paused view can change colors without advancing to newly received data.
  const view = $("log-view"),
    scrollTop = view.scrollTop,
    hexMode = select("display").value === "hex";
  for (const row of Array.from(
    view.querySelectorAll<HTMLElement>(".log-row"),
  )) {
    const payload = row.querySelector<HTMLElement>(".payload");
    if (!payload) continue;
    const content = payload.textContent || "",
      direction = row.classList.contains("rx") ? "rx" : "other";
    row.classList.remove("error", "warning", "normal");
    row.classList.add(
      direction === "rx" && !hexMode && colorSettings.builtins
        ? severity(content)
        : "normal",
    );
    paintLogText(payload, content, direction, input("filter").value, hexMode);
  }
  view.scrollTop = scrollTop;
}
function render() {
  if (!current) return;
  const filter = input("filter").value.toLowerCase();
  const direction = select("direction").value;
  const hexMode = select("display").value === "hex";
  const rows = hexMode
    ? events.map((e) => ({
        ...e,
        content:
          e.direction !== "system" && e.base64
            ? Array.from(raw(e), (n) => n.toString(16).padStart(2, "0")).join(
                " ",
              )
            : e.text,
        lastSeq: e.seq,
      }))
    : logLines(events);
  const filtered = rows.filter(
    (e) =>
      (direction === "all" || e.direction === direction) &&
      (!filter || e.content.toLowerCase().includes(filter)),
  );
  const view = $("log-view");
  const bottom = view.scrollHeight - view.scrollTop - view.clientHeight < 80;
  const fragment = document.createDocumentFragment();
  for (const e of filtered.slice(-600)) {
    const row = document.createElement("div");
    row.className = `log-row ${e.direction} ${e.direction === "rx" && !hexMode && colorSettings.builtins ? severity(e.content) : "normal"}`;
    const source =
      e.direction === "system"
        ? labels[e.source] || e.source
        : e.direction === "rx"
          ? "RX / 设备"
          : `TX / ${labels[e.source] || e.source}`;
    for (const [cls, text] of [
      ["seq", String(e.seq)],
      ["time", time(e.time)],
      ["origin", source],
    ]) {
      const span = document.createElement("span");
      span.className = cls;
      span.textContent = text;
      row.append(span);
    }
    const payload = document.createElement("span");
    payload.className = "payload";
    paintLogText(payload, e.content, e.direction, filter, hexMode);
    row.append(payload);
    row.title = `事件 #${e.seq}${e.lastSeq !== e.seq ? `–${e.lastSeq}` : ""} · ${e.time}${e.task_id ? ` · 任务 ${e.task_id}` : ""}`;
    fragment.append(row);
  }
  if (!filtered.length) {
    const empty = document.createElement("div");
    empty.className = "empty-state";
    empty.textContent = events.length ? "没有符合条件的日志" : "等待设备输出…";
    fragment.append(empty);
  }
  view.replaceChildren(fragment);
  if (bottom && !historyMode) view.scrollTop = view.scrollHeight;
  $("event-count").textContent =
    `${historyMode ? "历史文件" : "实时窗口"} · ${filtered.length} 行${filtered.length > 600 ? "（最近 600 行）" : ""}${windowDropped ? " · 更早内容见历史文件" : ""}`;
}
async function loadHistory(reset: boolean) {
  if (!current) return;
  const id = current.id;
  const rev = revision;
  historyMode = true;
  paused = false;
  input("direct-input").checked = false;
  if (ws) {
    ws.onclose = null;
    ws.close();
    ws = undefined;
  }
  clearTimeout(reconnect);
  button("pause").disabled = true;
  $("stream-state").textContent = "HISTORY";
  $("return-live").classList.remove("hidden");
  if (reset) {
    historyCursor = 0;
    events = [];
    decoder = new TextDecoder();
    terminal.reset();
    notice("");
  }
  const filter = input("filter").value;
  const page = await api<Page>(
    `/sessions/${id}/history?after=${historyCursor}&limit=500&contains=${encodeURIComponent(filter)}`,
  );
  if (rev !== revision || !historyMode) return;
  historyCursor = page.next_cursor;
  if (page.gap)
    notice(
      `只显示仍保留的日志，最早事件为 #${page.first_available}。更早文件已轮转。`,
    );
  append(page.events);
  $("load-more").classList.toggle("hidden", !page.has_more);
  render();
}
function renderTasks(tasks: Task[]) {
  const selectedTasks = tasks.filter((t) => t.session_id === selected);
  $("task-count").textContent = String(selectedTasks.length);
  const list = $("tasks");
  list.replaceChildren();
  if (!selectedTasks.length) {
    const p = document.createElement("p");
    p.className = "muted empty-tasks";
    p.textContent = "你、CLI 和 AI 的任务会显示在这里。";
    list.append(p);
  }
  for (const task of selectedTasks.slice(0, 12)) {
    const card = document.createElement("div");
    card.className = "task-card";
    const top = document.createElement("div");
    top.className = "task-top";
    const who = document.createElement("span");
    who.textContent = `${labels[task.source] || task.source} · ${new Date(task.created_at).toLocaleTimeString("zh-CN", { hour12: false })}`;
    const state = document.createElement("span");
    state.className = `task-status ${task.status}`;
    state.textContent = labels[task.status] || task.status;
    top.append(who, state);
    const title = document.createElement("div");
    title.className = "task-label";
    title.textContent = task.label;
    title.title = task.label;
    card.append(top, title);
    if (task.message) {
      const p = document.createElement("div");
      p.className = "message";
      p.textContent = task.message;
      card.append(p);
    }
    if (!task.finished_at) {
      const cancel = document.createElement("button");
      cancel.className = "text-button";
      cancel.textContent = "取消";
      cancel.onclick = () =>
        api(`/tasks/${task.id}/cancel`, {}).catch((e) =>
          toast(e.message, true),
        );
      card.append(cancel);
    }
    list.append(card);
  }
}
async function openDemo() {
  const existing = sessions.find((s) => s.demo && s.state === "connected");
  if (existing) {
    choose(existing.id);
    return;
  }
  const s = await api<Session>("/sessions", {
    port: "DEMO",
    baud_rate: 115200,
    demo: true,
  });
  sessions.unshift(s);
  choose(s.id);
  toast("模拟设备已连接，不涉及物理串口。");
}
async function ports() {
  const items = await api<{ name: string; description: string }[]>("/ports");
  const list = $("port-options");
  list.replaceChildren();
  for (const p of items) {
    const option = document.createElement("option");
    option.value = p.name;
    option.label = p.description;
    list.append(option);
  }
  if (!input("port").value && items.length) input("port").value = items[0].name;
}
async function sendData(data: string, extra: Record<string, unknown> = {}) {
  if (!current || current.state !== "connected") throw Error("请先连接设备");
  return api<Task>(`/sessions/${current.id}/send`, {
    data,
    encoding: "utf8",
    source: "human",
    timeout_ms: 5000,
    ...extra,
  });
}
$("send-form").addEventListener("submit", async (event) => {
  event.preventDefault();
  const command = $("command") as HTMLTextAreaElement;
  const encoding = select("encoding").value;
  let data = command.value;
  if (!data) return;
  data +=
    encoding === "utf8"
      ? { none: "", lf: "\n", cr: "\r", crlf: "\r\n" }[
          select("newline").value
        ] || ""
      : "";
  try {
    await sendData(data, {
      encoding,
      wait_for: input("wait-for").value || null,
      timeout_ms: Number(input("timeout").value),
      sensitive: input("sensitive").checked,
    });
    if (input("sensitive").checked) command.value = "";
    toast("命令已加入共享队列");
  } catch (e) {
    toast((e as Error).message, true);
  }
});
$("connect-form").addEventListener("submit", async (event) => {
  event.preventDefault();
  button("connect-submit").disabled = true;
  try {
    const s = await api<Session>("/sessions", {
      port: input("port").value.trim(),
      baud_rate: Number(select("baud").value),
      data_bits: select("data-bits").value,
      stop_bits: select("stop-bits").value,
      parity: select("parity").value,
      flow_control: select("flow-control").value,
    });
    sessions.unshift(s);
    choose(s.id);
    ($("connect-dialog") as HTMLDialogElement).close();
  } catch (e) {
    toast((e as Error).message, true);
  } finally {
    button("connect-submit").disabled = false;
  }
});
action("new-session", async () => {
  ($("connect-dialog") as HTMLDialogElement).showModal();
  await ports();
});
action("close-dialog", () =>
  ($("connect-dialog") as HTMLDialogElement).close(),
);
action("refresh-ports", ports);
action("demo", openDemo);
action("empty-demo", openDemo);
action("refresh", async () => {
  sessions = await api<Session[]>("/sessions");
  sessionCards();
  toast("会话列表已刷新");
});
action("disconnect", async () => {
  if (current) {
    await api(`/sessions/${current.id}/close`, {});
    current.state = "closed";
    header();
    toast("连接已关闭，历史日志仍可查看");
  }
});
action("takeover", async () => {
  if (current) {
    const owned = current.control_owner === "human";
    if (owned) {
      clearTimeout(keyboardTimer);
      keyboard = "";
      input("direct-input").checked = false;
    }
    await api(`/sessions/${current.id}/${owned ? "release" : "takeover"}`, {});
    current.control_owner = owned ? null : "human";
    header();
    toast(
      owned
        ? "控制权已释放，CLI 和 AI 可继续提交任务"
        : "你已持有控制权；CLI 和 AI 暂停发送，查看日志不受影响",
    );
  }
});
action("ctrl-c", async () => {
  if (current) {
    await api(`/sessions/${current.id}/takeover`, {});
    current.control_owner = "human";
    header();
    await sendData("\x03");
    toast("已请求发送 Ctrl+C");
  }
});
action("set-control", async () => {
  if (current) {
    const rts = select("rts").value,
      dtr = select("dtr").value;
    await api(`/sessions/${current.id}/control`, {
      rts: rts ? rts === "true" : null,
      dtr: dtr ? dtr === "true" : null,
      source: "human",
    });
    toast("控制线已更新");
  }
});
action("pause", () => {
  paused = !paused;
  button("pause").textContent = paused ? "▷" : "Ⅱ";
  button("pause").title = paused ? "恢复显示" : "暂停显示，采集继续";
  if (!paused) {
    pausedCount = 0;
    terminal.reset();
    for (const e of events) if (e.direction === "rx") terminal.write(raw(e));
    render();
  }
});
action("history-search", () => loadHistory(true));
action("load-more", () => loadHistory(false));
action("return-live", () => {
  if (current) choose(current.id);
});
action("export", () => {
  if (current) {
    const a = document.createElement("a");
    a.href = `/api/sessions/${current.id}/export`;
    a.download = "";
    a.click();
  }
});
action("copy-path", async () => {
  if (current) {
    await navigator.clipboard.writeText(current.directory);
    toast("日志目录已复制");
  }
});
for (const id of ["filter", "direction", "display"])
  $(id).addEventListener(id === "filter" ? "input" : "change", () => {
    if (!paused) render();
  });
input("filter").addEventListener("keydown", (event) => {
  if (event.key === "Enter")
    loadHistory(true).catch((e) => toast(e.message, true));
});
function tab(terminalSelected: boolean) {
  terminalMode = terminalSelected;
  $("log-tab").classList.toggle("active", !terminalSelected);
  $("terminal-tab").classList.toggle("active", terminalSelected);
  $("log-view").classList.toggle("hidden", terminalSelected);
  $("terminal-view").classList.toggle("hidden", !terminalSelected);
  $("filter-bar").classList.toggle("hidden", terminalSelected);
  document
    .querySelector(".log-head")!
    .classList.toggle("hidden", terminalSelected);
  if (terminalSelected) {
    requestAnimationFrame(() => {
      fit.fit();
      terminal.focus();
    });
  }
}
action("log-tab", () => tab(false));
action("terminal-tab", () => tab(true));
let keyboard = "",
  keyboardTimer: number | undefined;
input("direct-input").addEventListener("change", async () => {
  if (!current) return;
  const wanted = input("direct-input").checked;
  input("direct-input").checked = false;
  clearTimeout(keyboardTimer);
  keyboard = "";
  const target = current.id;
  try {
    await api(`/sessions/${target}/${wanted ? "takeover" : "release"}`, {});
    if (current?.id === target) {
      current.control_owner = wanted ? "human" : null;
      header();
      input("direct-input").checked = wanted;
      if (wanted) terminal.focus();
    }
  } catch (e) {
    toast((e as Error).message, true);
  }
});
terminal.onData((data) => {
  if (
    historyMode ||
    !input("direct-input").checked ||
    current?.state !== "connected"
  )
    return;
  keyboard += data;
  clearTimeout(keyboardTimer);
  keyboardTimer = window.setTimeout(() => {
    const data = keyboard;
    keyboard = "";
    sendData(data).catch((e) => {
      input("direct-input").checked = false;
      toast(e.message, true);
    });
  }, 25);
});
input("macro-file").addEventListener("change", async () => {
  try {
    const file = input("macro-file").files?.[0];
    if (!file) return;
    macroPack = JSON.parse(await file.text());
    const list = select("macro-name");
    list.replaceChildren();
    for (const [type, items] of [
      ["macro", macroPack.macros || []],
      ["assembly", macroPack.assemblies || []],
    ] as const) {
      for (const item of items) {
        const option = document.createElement("option");
        option.value = `${type}:${item.name}`;
        option.textContent = `${type === "assembly" ? "组合" : "宏"} · ${item.name}`;
        list.append(option);
      }
    }
    list.disabled = !list.options.length;
    enabled();
  } catch (e) {
    macroPack = null;
    enabled();
    toast(`宏文件无效：${(e as Error).message}`, true);
  }
});
action("run-macro", async () => {
  if (!current || !macroPack) return;
  const value = select("macro-name").value;
  const split = value.indexOf(":");
  await api(`/sessions/${current.id}/macro`, {
    pack: macroPack,
    name: value.slice(split + 1),
    assembly: value.slice(0, split) === "assembly",
    source: "human",
  });
  toast("自动化宏已加入共享队列");
});
select("encoding").addEventListener("change", () => {
  select("newline").disabled = select("encoding").value !== "utf8";
});
void refresh();

// Keep the sidebar tools within the viewport rather than stacking tall sections.
function showPanel(name: string) {
  for (const other of ["send", "scripts", "tasks", "tools"]) {
    $(`panel-${other}`).classList.toggle("active", name === other);
    $(`tool-${other}`).classList.toggle("hidden", name !== other);
  }
}
for (const name of ["send", "scripts", "tasks", "tools"])
  action(`panel-${name}`, () => showPanel(name));
action("focus-view", () => {
  const focused = document.body.classList.toggle("focus-mode");
  button("focus-view").textContent = focused ? "退出专注" : "专注日志";
  if (terminalMode) requestAnimationFrame(() => fit.fit());
});
action("show-sessions", () => {
  document.body.classList.toggle("sessions-open");
  document.body.classList.remove("controls-open");
});
action("show-controls", () => {
  document.body.classList.toggle("controls-open");
  document.body.classList.remove("sessions-open");
});

function colorPreview() {
  document
    .querySelector(".log-legend")
    ?.classList.toggle(
      "hidden",
      !colorSettings.builtins ||
        colorSettings.rules.some((r) => r.enabled && r.keyword.trim()),
    );
  const preview = $("color-preview");
  preview.replaceChildren();
  const words = colorSettings.rules
    .filter((r) => r.enabled && r.keyword.trim())
    .slice(0, 5)
    .map((r) => r.keyword.trim());
  const sample = words.length
    ? words.join(" · ")
    : "[ADAS] Detect(57001) 连接失败";
  for (const part of logParts(sample, "", colorSettings)) {
    const span = document.createElement("span");
    if (part.color) span.style.color = part.color;
    else if (part.kind) span.className = `token-${part.kind}`;
    span.textContent = part.text;
    preview.append(span);
  }
}
function saveColors() {
  try {
    localStorage.setItem(COLOR_STORAGE_KEY, JSON.stringify(colorSettings));
    $("color-save-state").textContent = "已保存到当前浏览器 · 所有会话共用";
  } catch {
    $("color-save-state").textContent =
      "浏览器禁止保存，当前页仍生效；刷新后需重新设置。";
  }
  colorPreview();
  repaintColors();
}
function renderColorRules() {
  input("builtin-colors").checked = colorSettings.builtins;
  const list = $("color-rules");
  list.replaceChildren();
  button("add-color-rule").disabled =
    colorSettings.rules.length >= MAX_COLOR_RULES;
  if (!colorSettings.rules.length) {
    const p = document.createElement("p");
    p.className = "hint color-empty";
    p.textContent = "还没有自定义规则，添加后会优先于默认配色。";
    list.append(p);
  }
  colorSettings.rules.forEach((rule, index) => {
    const card = document.createElement("div");
    card.className = `color-rule${rule.enabled ? "" : " disabled"}`;
    const first = document.createElement("div");
    first.className = "color-rule-main";
    const keyword = document.createElement("input");
    keyword.type = "text";
    keyword.maxLength = 128;
    keyword.value = rule.keyword;
    keyword.placeholder = "关键词，例如 ADAS";
    keyword.setAttribute("aria-label", `规则 ${index + 1} 关键词`);
    keyword.addEventListener("input", () => {
      rule.keyword = keyword.value;
      saveColors();
    });
    const color = document.createElement("input");
    color.type = "color";
    color.value = rule.color;
    color.title = "选择文字颜色";
    color.setAttribute("aria-label", `规则 ${index + 1} 颜色`);
    color.addEventListener("input", () => {
      rule.color = color.value;
      saveColors();
    });
    first.append(keyword, color);
    const options = document.createElement("div");
    options.className = "color-rule-options";
    const checkbox = (name: "enabled" | "caseSensitive", text: string) => {
      const label = document.createElement("label"),
        check = document.createElement("input");
      check.type = "checkbox";
      check.checked = rule[name];
      check.addEventListener("change", () => {
        rule[name] = check.checked;
        card.classList.toggle("disabled", !rule.enabled);
        saveColors();
      });
      label.append(check, document.createTextNode(text));
      options.append(label);
    };
    checkbox("enabled", "启用");
    checkbox("caseSensitive", "区分大小写");
    const controls = document.createElement("div");
    controls.className = "color-rule-order";
    const addAction = (
      text: string,
      title: string,
      fn: () => void,
      disabled = false,
    ) => {
      const b = document.createElement("button");
      b.className = "icon-button";
      b.textContent = text;
      b.title = title;
      b.setAttribute("aria-label", `${title}规则 ${index + 1}`);
      b.disabled = disabled;
      b.onclick = fn;
      controls.append(b);
    };
    const move = (offset: number) => {
      const target = index + offset;
      [colorSettings.rules[index], colorSettings.rules[target]] = [
        colorSettings.rules[target],
        colorSettings.rules[index],
      ];
      renderColorRules();
      saveColors();
    };
    addAction("↑", "上移", () => move(-1), index === 0);
    addAction(
      "↓",
      "下移",
      () => move(1),
      index === colorSettings.rules.length - 1,
    );
    addAction("×", "删除", () => {
      colorSettings.rules.splice(index, 1);
      renderColorRules();
      saveColors();
    });
    options.append(controls);
    card.append(first, options);
    list.append(card);
  });
  colorPreview();
}
action("color-settings", () => {
  document.body.classList.remove("focus-mode", "sessions-open");
  document.body.classList.add("controls-open");
  button("focus-view").textContent = "专注日志";
  showPanel("tools");
  $("tool-tools").scrollTop = 0;
  if (terminalMode) requestAnimationFrame(() => fit.fit());
});
action("add-color-rule", () => {
  if (colorSettings.rules.length >= MAX_COLOR_RULES) return;
  colorSettings.rules.push({
    keyword: "",
    color: "#8bc9ff",
    enabled: true,
    caseSensitive: false,
  });
  renderColorRules();
  saveColors();
  $("color-rules")
    .querySelector<HTMLInputElement>(".color-rule:last-child input[type=text]")
    ?.focus();
});
action("reset-colors", () => {
  colorSettings = defaultColors();
  renderColorRules();
  saveColors();
});
input("builtin-colors").addEventListener("change", () => {
  colorSettings.builtins = input("builtin-colors").checked;
  saveColors();
});
window.addEventListener("storage", (event) => {
  if (event.key === COLOR_STORAGE_KEY) {
    colorSettings = parseColors(event.newValue);
    renderColorRules();
    repaintColors();
  }
});
renderColorRules();

updateScriptConnection = initQuickScripts({
  getSession: () => current,
  getComposer: () => ({
    commands: ($("command") as HTMLTextAreaElement).value,
    encoding: select("encoding").value,
    newline: select("newline").value,
    waitFor: input("wait-for").value,
    timeoutMs: Number(input("timeout").value),
    sensitive: input("sensitive").checked,
  }),
  request: (path, body) => api(path, body),
  notify: toast,
  showTasks: () => showPanel("tasks"),
  showScripts: () => showPanel("scripts"),
});
