# Serial Workbench — 共享串口工作台

基于 [Adancurusul/serial-mcp-server](https://github.com/adancurusul/serial-mcp-server) 的独立开发分支，保留原项目 MIT 许可、版权及提交历史。原有 `serial-mcp-server` CLI/MCP 与本地后台录制功能继续保留。

`serial-workbench` 是新的共享模式：一个后台进程持有串口，网页、CLI 和 MCP 连接同一个会话。关闭网页、CLI 退出或 MCP 断开不停止采集。主动关闭会话会影响所有使用者。

## 快速启动

本地构建要求 Rust 1.85+。修改前端时需要 Node.js 22+；编译后的前端已提交，普通 Cargo 构建不需要 Node.js 或联网加载网页资源。

```powershell
cargo build --release --bin serial-workbench
.\target\release\serial-workbench.exe serve
```

打开 `http://127.0.0.1:8765`。也可以运行项目根目录的 `start-workbench.cmd`，它在必要时构建程序、在后台启动并打开页面。

首次启动不会连接任何物理设备。点击“打开模拟设备”即可试用实时日志、发送命令、响应等待、终端及协作记录。实际设备必须显式选择端口、波特率、帧格式和流控后连接。RTS/DTR 不在连接时主动改变；应用控制线需要显式操作，底层驱动打开端口时的行为仍取决于设备/驱动。

## 架构与进程

```text
serial-workbench serve        独立后台：串口唯一读取者、采集、任务队列、HTTP/WebSocket、网页
serial-workbench <command>    CLI 客户端：请求后台，不直接打开串口
serial-workbench mcp          MCP stdio 适配器：请求同一个后台，退出不会关闭串口
```

原来的 `serial-mcp-server serve/read/write/...` 是直接串口模式，不会自动连接共享后台。共享场景请统一使用新入口，以免两个独立程序争抢物理串口。

后台仅监听 IPv4 loopback。默认地址 `http://127.0.0.1:8765`，可以用 `serve --port 8766 --data-dir C:\logs\serial` 更改。CLI/MCP 使用全局 `--url http://127.0.0.1:8766` 指向它。一个后台可管理最多 16 个同时连接的设备；同一端口重复打开会返回已有会话 ID。

## CLI：默认输出 JSON

```powershell
$wb = '.\target\release\serial-workbench.exe'
& $wb ports
& $wb open --demo
# 将返回的 id 作为下方 <session-id>；真实连接示例：
& $wb open --port COM5 --baud 115200 --data-bits 8 --stop-bits 1 --parity none --flow-control none
& $wb sessions
& $wb events <session-id> --after 0 --limit 100
& $wb send <session-id> --data uptime --newline crlf --wait
& $wb send <session-id> --data help --newline crlf --wait-for 'demo>' --timeout-ms 5000 --wait
& $wb task <task-id> --wait
& $wb cancel <task-id>
& $wb events <session-id> --history --contains ERROR
& $wb macro <session-id> .\pack.json boot --wait
& $wb takeover <session-id>
& $wb release <session-id>
& $wb close <session-id>
# 停止整个后台（会关闭所有会话并落盘）
& $wb stop
```

- `send` 不自动追加行尾，使用 `--newline lf|cr|crlf` 明确指定；非 UTF-8 编码不得追加文本行尾。
- `--file payload.txt` 可代替 `--data`，文件内容作为指定编码的输入，避免复杂 shell 转义或密码进入命令行参数。
- 不加 `--wait` 时立即返回任务 ID；后台继续执行。`--wait` 只等待结果，客户端退出不会取消任务。
- `sent` 仅确认已发送；`succeeded` 表示配置的等待条件均匹配；`failed/cancelled` 不表示设备已回滚。等待失败/取消时 CLI 退出码为 2，调用错误为 1。
- `cancel` / `takeover` 只取消主机任务，无法撤回已发送字节，也不会自动发送 Ctrl+C。界面有独立“发送 Ctrl+C”按钮。
- 点击接管或启用终端直接输入后，人持有控制权，CLI/MCP 的新发送会被拒绝；点击“释放控制权”或 `release` 后恢复。页面关闭不会自动释放，以免半条命令被另一方接着写入；重新打开界面可手动释放。
- 同一会话的任务按提交顺序执行，整个宏占用发送队列；采集、日志查看和历史查询继续运行。
- `--sensitive` 只隐藏 TX 日志和任务标题，设备回显仍可能含敏感信息；原始 RX 文件需要按实际内容保护。

宏沿用上游 `schema_version: "0.3"` 格式，支持 send / expect / delay 和 assembly。共享模式复用原来的规划、验证及执行器，并把宏读取适配成独立游标，避免从设备抢读。

## MCP

先启动后台，再把 MCP 客户端指向新可执行文件：

```json
{
  "mcpServers": {
    "serial-workbench": {
      "command": "C:\\Tools\\serial-workbench\\serial-workbench.exe",
      "args": ["mcp"]
    }
  }
}
```

以上是通用 MCP 客户端示例，不会自动修改任何已安装客户端的配置。

工具：`serial_ports`、`serial_sessions`、`serial_open`、`serial_events`、`serial_send`、`serial_task`、`serial_cancel`、`serial_macro`、`serial_control`、`serial_close`。

推荐先查 `serial_sessions` 并复用已有连接。`serial_events` 返回 `next_cursor`，下次用它作为 `after`。每个客户端自己的游标互不影响。`gap: true` 表示游标之前存在缺口，`first_available` 是当前最早事件；不要把缓存结果当作完整历史。仅退出 MCP 时不要调用 `serial_close`。

## 网页

- 实时日志、方向过滤、关键字高亮、文本/HEX 显示、暂停显示、历史文件查询及原始 RX 导出。
- xterm.js 交互终端，默认不直接发送键盘输入；需要交互时显式启用。发送输入进入同一队列。
- 显示来源为你 / CLI / AI-MCP 的操作记录和任务状态。
- 设置行尾、等待文本和超时；上传 JSON 宏包后运行。
- 原始接收时间以 UTC 存储，界面显示浏览器本地时间；设备自身输出的时间保持原样。
- 页面实时窗口最多保留 4000 个事件、渲染最近 600 个匹配事件。暂停显示不会暂停采集，较早内容可查历史文件。

## 本地日志与留存

默认 `workbench-data/<session-uuid>/`：

| 文件 | 内容 |
| --- | --- |
| `session.json` | 端口参数、启动/结束时间、计数、状态、目录 |
| `rx.log`、`.1`…`.4` | 未修改的原始接收字节，每文件 10 MiB，最多 5 个 |
| `events.jsonl`、`.1`…`.4` | 带递增序号、UTC 时间、方向、来源、任务 ID、原始字节 Base64 的事件，每文件 20 MiB，最多 5 个 |

同一组文件后缀越大越旧。文件轮转只作用于当前会话，历史会话目录不会自动删除。每秒 flush 并更新会话快照；这是进程级刷出，不是断电持久性的承诺。进程异常结束后，之前仍标记 connected 的历史会话显示 interrupted，不会自动重连设备。

原始文件可直接用 `rg` / 脚本查询，无需通过 MCP 搬运整段日志：

```powershell
rg -n -C 8 'May Be Dead|Oops|ERROR' .\workbench-data\<session-id>\rx.log*
```

HTTP/MCP 中的 `contains` 按事件块匹配，不保证跨块匹配；跨行/跨块的完整搜索请直接读文件。实时缓存限制为 4096 个事件且约 4 MiB，溢出会明确报告缺口。`rx_offset` 是原始接收流累计偏移，需结合轮转保留范围理解。

设备日志可能包含凭据。`workbench-data/`、运行产物和 `.env` 已被 Git 忽略，导出或分享前需要按实际内容处理。

## 开发与验证

```powershell
cd web
npm ci
npm run check
npm run build
cd ..
cargo fmt --all --check
cargo test --locked
cargo clippy --all-targets -- -D warnings
cargo build --release --bin serial-workbench
python scripts/smoke-workbench.py target/release/serial-workbench.exe
```

Python 冒烟测试使用 `websockets` 包，只创建自己的模拟会话和临时日志目录，不访问物理串口。覆盖 CLI、网页静态资源、WebSocket 同源多读者与重连、MCP 发现和操作、MCP 退出不关闭连接、超时退出码、落盘与导出。Rust 测试覆盖队列互斥、取消、宏与采集共存、游标溢出和历史读取。

当前没有实际硬件验收，也不宣称已验证 USB 拔插、电平变化或设备重启后的恢复行为。自动重连、图形化宏编辑和桌面托盘不在这一版中。
