import { visibleText } from "./command-text";
import { stripAnsi } from "./log-format";
export type TaskSummary = {
  id: string;
  session_id: string;
  port: string;
  source: string;
  label: string;
  status: string;
  created_at: string;
  finished_at: string | null;
  message: string | null;
  start_cursor: number;
  end_cursor: number;
  commands_preview: string[];
  command_count: number;
  sensitive: boolean;
};
type TaskDetails = TaskSummary & {
  steps: {
    kind: string;
    data?: string;
    encoding?: string;
    ms?: number;
    timeout_ms?: number;
  }[];
  script?: { id: string; revision: number };
  reconstructed: boolean;
  transcript: {
    events: {
      seq: number;
      time: string;
      direction: string;
      text: string;
      base64: string;
      bytes: number;
    }[];
    bytes_returned: number;
    gap: boolean;
    truncated: boolean;
    finalized: boolean;
    hidden: boolean;
    next_cursor: number;
  };
};
type Context = {
  get: (path: string) => Promise<unknown>;
  post: (path: string, body: unknown) => Promise<unknown>;
  selectedSession: () => string;
  notify: (message: string, error?: boolean) => void;
  labels: Record<string, string>;
};
export function initTaskPanel(
  context: Context,
): (tasks: TaskSummary[]) => void {
  const el = <T extends HTMLElement = HTMLElement>(id: string) =>
    document.getElementById(id) as T;
  const dialog = el<HTMLDialogElement>("task-dialog");
  let latest: TaskSummary[] = [],
    selectedId = "",
    generation = 0,
    timer: number | undefined,
    detail: TaskDetails | undefined;
  function text(tag: string, value: string, className = "") {
    const node = document.createElement(tag);
    node.className = className;
    node.textContent = value;
    return node;
  }
  function render(tasks: TaskSummary[]) {
    latest = tasks;
    const selected =
      el<HTMLSelectElement>("task-scope").value === "session"
        ? tasks.filter((t) => t.session_id === context.selectedSession())
        : tasks;
    el("task-count").textContent = String(selected.length);
    const list = el("tasks");
    list.replaceChildren();
    if (!selected.length) {
      list.append(
        text(
          "p",
          "你、CLI 和 AI 的命令、回包及执行状态会保留在这里。",
          "muted empty-tasks",
        ),
      );
      return;
    }
    for (const task of selected.slice(0, 80)) {
      const card = text("article", "", "task-card");
      const top = text("div", "", "task-top");
      top.append(
        text(
          "span",
          `${context.labels[task.source] || task.source} · ${task.port || "串口"} · ${new Date(task.created_at).toLocaleTimeString("zh-CN", { hour12: false })}`,
        ),
        text(
          "span",
          context.labels[task.status] || task.status,
          `task-status ${task.status}`,
        ),
      );
      card.append(top, text("div", task.label || "串口操作", "task-label"));
      const commands = task.commands_preview || [];
      if (commands.length) {
        const preview = text("div", "", "task-command-preview");
        preview.append(text("small", `计划发送 · ${task.command_count} 条`));
        for (const command of commands) preview.append(text("code", command));
        if (task.command_count > commands.length)
          preview.append(
            text(
              "small",
              `另有 ${task.command_count - commands.length} 条，详情中查看`,
            ),
          );
        card.append(preview);
      }
      if (task.message) card.append(text("div", task.message, "message"));
      const actions = text("div", "", "task-actions");
      const open = text(
        "button",
        "命令与回包 →",
        "text-button",
      ) as HTMLButtonElement;
      open.onclick = () => openDetails(task.id);
      actions.append(open);
      if (!task.finished_at) {
        const cancel = text(
          "button",
          "取消任务",
          "text-button",
        ) as HTMLButtonElement;
        cancel.onclick = () =>
          void context
            .post(`/tasks/${task.id}/cancel`, {})
            .catch((e) => context.notify(e.message, true));
        actions.append(cancel);
      }
      card.append(actions);
      list.append(card);
    }
  }
  function renderDetail(record: TaskDetails) {
    detail = record;
    el("task-detail-title").textContent = record.label || "串口操作";
    const elapsed = record.finished_at
      ? `${((Date.parse(record.finished_at) - Date.parse(record.created_at)) / 1000).toFixed(2)} 秒`
      : "运行中";
    el("task-detail-meta").textContent =
      `${record.port || "串口"} · ${context.labels[record.source] || record.source} · ${context.labels[record.status] || record.status} · 总耗时 ${elapsed}`;
    el("task-detail-reference").textContent =
      `任务 ${record.id}${record.script ? ` · 共享脚本 v${record.script.revision}` : ""}`;
    const plan = el("task-plan");
    plan.replaceChildren();
    record.steps.forEach((step, index) => {
      const row = text("div", "", "task-step");
      const label =
        step.kind === "send"
          ? `发送 · ${step.encoding?.toUpperCase()}`
          : step.kind === "delay"
            ? `间隔 ${step.ms} ms`
            : `等待${step.kind === "expect_equals" ? "完全等于" : "包含"} · ${step.timeout_ms} ms`;
      row.append(text("span", `${index + 1}. ${label}`, "step-heading"));
      if (step.data !== undefined)
        row.append(text("code", visibleText(step.data)));
      plan.append(row);
    });
    if (!record.steps.length)
      plan.append(text("p", "未保留原始步骤，请结合接收窗口查看。", "hint"));
    const warnings = [];
    if (record.reconstructed)
      warnings.push("旧记录由实际发送日志恢复，原始延时和等待条件未知。");
    if (record.transcript.hidden)
      warnings.push("此任务启用了隐藏输入，命令与回包已隐藏。");
    if (record.transcript.gap) warnings.push("历史接收记录存在缺口。");
    if (record.transcript.truncated)
      warnings.push("显示已达到 64 KiB / 512 事件上限；完整内容请查会话日志。");
    if (!record.transcript.finalized) warnings.push("正在收集接收窗口…");
    if (record.message) warnings.push(record.message);
    el("task-detail-notice").textContent = warnings.join(" ");
    el("task-detail-notice").classList.toggle("hidden", !warnings.length);
    const transcript = el("task-transcript"),
      scroll = transcript.scrollTop,
      nearBottom =
        transcript.scrollHeight - scroll - transcript.clientHeight < 60;
    transcript.replaceChildren();
    const decoder = new TextDecoder();
    let tx = 0,
      rx = 0;
    const blocks: {
      direction: string;
      seq: number;
      endSeq: number;
      time: string;
      text: string;
    }[] = [];
    for (const event of record.transcript.events) {
      let content = event.text;
      if (event.direction === "rx") {
        rx += event.bytes;
        try {
          content = decoder.decode(
            Uint8Array.from(atob(event.base64), (c) => c.charCodeAt(0)),
            { stream: true },
          );
        } catch {
          /* text fallback */
        }
        const last = blocks.at(-1);
        if (last?.direction === "rx") {
          last.text += content;
          last.endSeq = event.seq;
          continue;
        }
      } else {
        tx++;
        try {
          content = new TextDecoder("utf-8", { fatal: true }).decode(
            Uint8Array.from(atob(event.base64), (c) => c.charCodeAt(0)),
          );
          content = visibleText(content);
        } catch {
          content =
            "HEX: " +
            Array.from(
              Uint8Array.from(atob(event.base64), (c) => c.charCodeAt(0)),
              (n) => n.toString(16).padStart(2, "0"),
            ).join(" ");
        }
      }
      blocks.push({
        direction: event.direction,
        seq: event.seq,
        endSeq: event.seq,
        time: event.time,
        text: content,
      });
    }
    for (const block of blocks) {
      const row = text("div", "", `task-io ${block.direction}`);
      row.append(
        text(
          "small",
          `${block.direction.toUpperCase()} · ${new Date(block.time).toLocaleTimeString("zh-CN", { hour12: false })} · #${block.seq}${block.endSeq === block.seq ? "" : `–${block.endSeq}`}`,
        ),
        text(
          "pre",
          block.direction === "rx" ? stripAnsi(block.text) : block.text,
        ),
      );
      transcript.append(row);
    }
    if (!record.transcript.events.length)
      transcript.append(
        text(
          "p",
          record.transcript.hidden
            ? "隐藏输入任务不展示收发内容"
            : "此窗口尚无收发内容",
          "hint",
        ),
      );
    transcript.scrollTop = nearBottom ? transcript.scrollHeight : scroll;
    el("task-detail-stats").textContent = `实际发送 ${tx} 次 · 接收 ${rx} 字节`;
    el<HTMLButtonElement>("cancel-detail-task").disabled = !!record.finished_at;
  }
  async function loadDetails(id: string, turn: number) {
    try {
      const record = (await context.get(`/tasks/${id}/details`)) as TaskDetails;
      if (turn !== generation || !dialog.open) return;
      renderDetail(record);
      if (!record.finished_at || !record.transcript.finalized)
        timer = window.setTimeout(() => void loadDetails(id, turn), 700);
    } catch (error) {
      if (turn === generation) {
        el("task-detail-notice").textContent = (error as Error).message;
        el("task-detail-notice").classList.remove("hidden");
      }
    }
  }
  function openDetails(id: string) {
    generation++;
    clearTimeout(timer);
    selectedId = id;
    detail = undefined;
    el("task-detail-title").textContent = "正在读取任务详情…";
    el("task-detail-meta").textContent = "";
    el("task-plan").replaceChildren();
    el("task-transcript").replaceChildren();
    el("task-detail-notice").classList.add("hidden");
    if (!dialog.open) dialog.showModal();
    void loadDetails(id, generation);
  }
  el("task-scope").addEventListener("change", () => render(latest));
  el("close-task-dialog").addEventListener("click", () => dialog.close());
  dialog.addEventListener("close", () => {
    generation++;
    clearTimeout(timer);
  });
  el("refresh-task-detail").addEventListener("click", () => {
    clearTimeout(timer);
    void loadDetails(selectedId, ++generation);
  });
  el("cancel-detail-task").addEventListener("click", () => {
    void context
      .post(`/tasks/${selectedId}/cancel`, {})
      .then(() => loadDetails(selectedId, ++generation))
      .catch((e) => context.notify(e.message, true));
  });
  el("copy-task-detail").addEventListener("click", () => {
    if (detail)
      void navigator.clipboard
        .writeText(JSON.stringify(detail, null, 2))
        .then(() => context.notify("任务详情已复制"))
        .catch((e) => context.notify(e.message, true));
  });
  return render;
}
