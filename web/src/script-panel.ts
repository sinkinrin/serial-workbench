import {
  SCRIPT_STORAGE_KEY,
  MAX_SCRIPTS,
  commandLines,
  parseScripts,
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
  get: (path: string) => Promise<unknown>;
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
  let editingId: string | undefined;
  let editingRevision = 0;
  let loading = false;
  let initialized = false;
  let lastLoaded = 0;
  const migrationKey = "serial-workbench-quick-scripts-shared-migrated-v1";
  const pending = new Set<string>();

  function connectionState() {
    if (!loading && Date.now() - lastLoaded > 2500) void loadShared();
    const connected = context.getSession()?.state === "connected";
    for (const run of Array.from(
      list.querySelectorAll<HTMLButtonElement>("button[data-run]"),
    ))
      run.disabled = !connected || pending.has(run.dataset.run!);
    el("script-target").textContent = connected
      ? "运行到当前会话 · 任务可在协作中取消"
      : "可离线编辑，连接设备后运行";
  }
  async function loadShared() {
    if (loading) return;
    loading = true;
    try {
      let remote = (await context.get("/scripts")) as QuickScript[];
      if (!initialized) {
        let legacy: QuickScript[] = [],
          migrate = false;
        try {
          migrate = localStorage.getItem(migrationKey) !== "1";
          if (migrate)
            legacy = parseScripts(localStorage.getItem(SCRIPT_STORAGE_KEY));
        } catch {
          /* Shared scripts work without browser storage. */
        }
        if (legacy.length) {
          for (const script of legacy) {
            if (remote.some((s) => s.id === script.id)) continue;
            const saved = (await context.request("/scripts", {
              ...script,
              source: "human",
              revision: 0,
            })) as QuickScript;
            remote.push(saved);
          }
          context.notify(`旧脚本已迁移到共享后台，本地备份保留`);
        }
        if (migrate) {
          try {
            localStorage.setItem(migrationKey, "1");
          } catch {
            /* Keeping legacy backup makes a retry safe. */
          }
        }
        initialized = true;
      }
      const changed = JSON.stringify(remote) !== JSON.stringify(scripts);
      scripts = remote;
      lastLoaded = Date.now();
      el("script-storage-status").textContent =
        "已保存到本机共享后台 · 网页、CLI 和 AI 共用";
      if (changed || !list.childElementCount) render();
    } catch (error) {
      lastLoaded = Date.now();
      el("script-storage-status").textContent =
        `共享脚本读取失败，原有本地备份未删除：${(error as Error).message}`;
    } finally {
      loading = false;
    }
  }
  async function save(script: QuickScript): Promise<boolean> {
    try {
      const saved = (await context.request("/scripts", {
        ...script,
        source: "human",
      })) as QuickScript;
      const index = scripts.findIndex((s) => s.id === saved.id);
      if (index < 0) scripts.push(saved);
      else scripts[index] = saved;
      render();
      return true;
    } catch (error) {
      context.notify(
        `保存失败，编辑内容仍保留：${(error as Error).message}`,
        true,
      );
      void loadShared();
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
    editingRevision = script?.revision ?? 0;
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
      await context.request(`/scripts/${script.id}/run`, {
        session_id: session.id,
        source: "human",
        revision: script.revision,
      });
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
      meta.textContent = `v${script.revision ?? 1} · ${lines.length} 条命令 · ${script.newline === "none" ? "无行尾" : script.newline.toUpperCase()} · ${script.intervalMs} ms${script.waitFor ? " · 等待响应" : ""}`;
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
      remove.onclick = async () => {
        try {
          await context.request(`/scripts/${script.id}/delete`, {
            revision: script.revision,
          });
          scripts = scripts.filter((s) => s.id !== script.id);
          render();
          context.notify("共享脚本已删除");
        } catch (error) {
          context.notify((error as Error).message, true);
          void loadShared();
        }
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
  el("script-form").addEventListener("submit", async (event) => {
    event.preventDefault();
    if (!editingId && scripts.length >= MAX_SCRIPTS) {
      context.notify("最多保存 100 个脚本。", true);
      return;
    }
    const script: QuickScript = {
      id: editingId ?? crypto.randomUUID(),
      revision: editingRevision,
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
    const submit = el<HTMLButtonElement>("save-shared-script");
    submit.disabled = true;
    try {
      if (await save(script)) {
        dialog.close();
        context.showScripts();
        context.notify("脚本已保存，网页和 AI 都可以使用");
      }
    } finally {
      submit.disabled = false;
    }
  });
  window.addEventListener("focus", () => void loadShared());
  render();
  void loadShared();
  return connectionState;
}
