import {
  SCRIPT_STORAGE_KEY,
  MAX_SCRIPTS,
  commandLines,
  parseScripts,
  scriptRequest,
  validateScript,
} from "./quick-scripts";
import type { QuickScript, ScriptNewline } from "./quick-scripts";

type Context = {
  getSession: () => { id: string; state: string } | undefined;
  getComposer: () => {
    commands: string;
    encoding: string;
    newline: string;
    waitFor: string;
    timeoutMs: number;
    sensitive: boolean;
  };
  request: (path: string, body: unknown) => Promise<unknown>;
  notify: (message: string, error?: boolean) => void;
  showTasks: () => void;
  showScripts: () => void;
};

export function initQuickScripts(context: Context): () => void {
  const el = <T extends HTMLElement = HTMLElement>(id: string) =>
    document.getElementById(id) as T;
  const value = (id: string) => (el(id) as HTMLInputElement).value;
  const dialog = el<HTMLDialogElement>("script-dialog");
  const list = el("script-list");
  let scripts: QuickScript[] = [];
  try {
    scripts = parseScripts(localStorage.getItem(SCRIPT_STORAGE_KEY));
  } catch {
    /* Show save failures when the user saves. */
  }
  let editingId: string | undefined;
  const pending = new Set<string>();

  function connectionState() {
    const connected = context.getSession()?.state === "connected";
    for (const run of Array.from(
      list.querySelectorAll<HTMLButtonElement>("button[data-run]"),
    ))
      run.disabled = !connected || pending.has(run.dataset.run!);
    el("script-target").textContent = connected
      ? "运行到当前会话 · 任务可在协作中取消"
      : "可离线编辑，连接设备后运行";
  }
  function persist(next: QuickScript[]): boolean {
    try {
      localStorage.setItem(
        SCRIPT_STORAGE_KEY,
        JSON.stringify({ version: 1, scripts: next }),
      );
      scripts = next;
      render();
      return true;
    } catch {
      context.notify(
        "保存失败：浏览器存储不可用或空间已满，编辑内容仍保留在窗口中。",
        true,
      );
      return false;
    }
  }
  function openEditor(script?: QuickScript, fromComposer = false) {
    const composer = fromComposer ? context.getComposer() : undefined;
    if (composer?.sensitive) {
      context.notify("当前是隐藏输入，不保存为快捷脚本。", true);
      return;
    }
    if (composer && composer.encoding !== "utf8") {
      context.notify("快捷脚本使用文本命令，请先切换到 UTF-8。", true);
      return;
    }
    if (composer && !composer.commands.trim()) {
      context.notify("先在发送框中输入要保存的命令。", true);
      return;
    }
    editingId = script?.id;
    el("script-dialog-title").textContent = script
      ? "编辑快捷脚本"
      : "新建快捷脚本";
    const fields: Record<string, string> = {
      "script-name": script?.name ?? "",
      "script-commands": script?.commands ?? composer?.commands ?? "",
      "script-newline": script?.newline ?? composer?.newline ?? "crlf",
      "script-interval": String(script?.intervalMs ?? 200),
      "script-wait-for": script?.waitFor ?? composer?.waitFor ?? "",
      "script-timeout": String(
        script?.timeoutMs ?? composer?.timeoutMs ?? 5000,
      ),
    };
    for (const [id, text] of Object.entries(fields))
      (el(id) as HTMLInputElement).value = text;
    el<HTMLDetailsElement>("script-response-options").open =
      !!fields["script-wait-for"];
    updateCount();
    dialog.showModal();
    el<HTMLInputElement>("script-name").focus();
  }
  async function runScript(script: QuickScript) {
    const session = context.getSession();
    if (!session || session.state !== "connected") {
      context.notify("请先选择已连接的会话。", true);
      return;
    }
    pending.add(script.id);
    connectionState();
    try {
      await context.request(
        `/sessions/${session.id}/macro`,
        scriptRequest(script),
      );
      context.showTasks();
      context.notify(
        `“${script.name}”已加入队列，共 ${commandLines(script.commands).length} 条命令`,
      );
    } catch (error) {
      context.notify((error as Error).message, true);
    } finally {
      pending.delete(script.id);
      connectionState();
    }
  }
  function render() {
    list.replaceChildren();
    el("script-count").textContent = String(scripts.length);
    el<HTMLButtonElement>("new-script").disabled =
      scripts.length >= MAX_SCRIPTS;
    if (!scripts.length) {
      const empty = document.createElement("div");
      empty.className = "script-empty";
      const title = document.createElement("strong");
      title.textContent = "把常用操作存成一个按钮";
      const description = document.createElement("p");
      description.textContent =
        "单条命令或多条命令都可以。先新建脚本，填好名称与命令，之后一键运行。";
      empty.append(title, description);
      list.append(empty);
    }
    for (const script of scripts) {
      const card = document.createElement("article");
      card.className = "script-card";
      const title = document.createElement("h3");
      title.textContent = script.name;
      const lines = commandLines(script.commands);
      const meta = document.createElement("p");
      meta.className = "script-meta";
      meta.textContent = `${lines.length} 条命令 · ${script.newline === "none" ? "无行尾" : script.newline.toUpperCase()} · ${script.intervalMs} ms${script.waitFor ? " · 等待响应" : ""}`;
      const preview = document.createElement("pre");
      preview.textContent =
        lines.slice(0, 3).join("\n").slice(0, 240) +
        (lines.length > 3 ? "\n…" : "");
      const actions = document.createElement("div");
      actions.className = "script-card-actions";
      const run = document.createElement("button");
      run.className = "button primary small";
      run.textContent = "▶ 运行";
      run.dataset.run = script.id;
      run.onclick = () => void runScript(script);
      const edit = document.createElement("button");
      edit.className = "text-button";
      edit.textContent = "编辑";
      edit.onclick = () => openEditor(script);
      const remove = document.createElement("button");
      remove.className = "text-button delete-script";
      remove.textContent = "删除";
      remove.onclick = () => {
        if (persist(scripts.filter((s) => s.id !== script.id)))
          context.notify("脚本已删除");
      };
      actions.append(run, edit, remove);
      card.append(title, meta, preview, actions);
      list.append(card);
    }
    connectionState();
  }
  function updateCount() {
    el("script-line-count").textContent =
      `${commandLines(value("script-commands")).length} 条命令 · 空行忽略`;
  }
  el("script-commands").addEventListener("input", updateCount);
  el("new-script").addEventListener("click", () => openEditor());
  el("save-as-script").addEventListener("click", () =>
    openEditor(undefined, true),
  );
  for (const id of ["close-script-dialog", "cancel-script-editor"])
    el(id).addEventListener("click", () => dialog.close());
  el("script-form").addEventListener("submit", (event) => {
    event.preventDefault();
    if (!editingId && scripts.length >= MAX_SCRIPTS) {
      context.notify("最多保存 100 个脚本。", true);
      return;
    }
    const script: QuickScript = {
      id: editingId ?? crypto.randomUUID(),
      name: value("script-name").trim(),
      commands: value("script-commands"),
      newline: value("script-newline") as ScriptNewline,
      intervalMs: Number(value("script-interval")),
      waitFor: value("script-wait-for"),
      timeoutMs: Number(value("script-timeout")),
    };
    const error = validateScript(script);
    if (error) {
      context.notify(error, true);
      return;
    }
    const next = [...scripts],
      index = next.findIndex((s) => s.id === script.id);
    if (index >= 0) next[index] = script;
    else next.push(script);
    if (persist(next)) {
      dialog.close();
      context.showScripts();
      context.notify("脚本已保存，刷新页面后仍可使用");
    }
  });
  window.addEventListener("storage", (event) => {
    if (event.key === SCRIPT_STORAGE_KEY) {
      scripts = parseScripts(event.newValue);
      render();
    }
  });
  render();
  return connectionState;
}
