# Serial Workbench · 串口工作台

**一个串口连接，你和 AI 一起用。**

独立后台持有串口，网页、CLI 和 MCP 共用日志与命令队列。你可以持续看设备输出，也能查看 AI 究竟发送了什么命令、收到了什么回复。

版本 **0.1.0** · [English](README.md) · [下载](https://github.com/sinkinrin/serial-workbench/releases) · [完整使用说明](docs/WORKBENCH_ZH.md)

## 功能

- **后台持续采集**：关闭网页或断开 MCP 不停止录制，原始字节和带时间戳的事件轮转落盘。
- **一屏调试界面**：实时日志、交互终端、文本/HEX、搜索、暂停、历史查询与导出；支持自定义关键词与颜色、匹配优先级和大小写规则。
- **共享快捷脚本**：单条或多条命令，配置行尾、间隔和响应等待；网页、CLI、MCP 使用同一份脚本，修改时检查版本。
- **可追溯的协作记录**：卡片显示命令、来源、端口和状态；详情展示发送/等待/间隔步骤及实际 TX/RX，重启后仍可查看。
- **人工接管**：接管后阻止新的 AI/CLI 发送并取消排队任务，释放后恢复协作。所有命令串行执行，日志继续采集。
- **适合 AI 的访问方式**：默认 JSON 的 CLI、15 个 MCP 工具、互不消耗的日志游标；大量历史日志可以直接读本地文件。

## 下载与启动

从 [v0.1.0](https://github.com/sinkinrin/serial-workbench/releases/tag/v0.1.0) 下载 **Windows x64** 成品包 `serial-workbench-0.1.0-windows-x86_64.zip`，解压后双击 `start-workbench.cmd`。

启动器会在后台运行服务，并打开 `http://127.0.0.1:8765`。无需安装 Rust、Node.js 或额外的网页服务器；USB 串口适配器仍需对应的 Windows 驱动。

也可以在 PowerShell 中直接启动：

```powershell
.\serial-workbench.exe serve
```

打开网页，点击「新建连接」，选择真实端口和设备参数。程序不会在启动时自动打开串口。「打开模拟设备」是独立标注的试用入口，真实调试请选择物理端口。

使用网页、CLI 或 MCP 时保持后台运行。默认仅监听本机；更改端口或数据目录：

```powershell
.\serial-workbench.exe serve --port 8766 --data-dir C:\SerialLogs
.\serial-workbench.exe --url http://127.0.0.1:8766 status
```

## CLI：直接调用共享后台

以下命令都访问同一个后台，默认输出 JSON：

```powershell
serial-workbench ports
serial-workbench sessions
serial-workbench open --port COM6 --baud 115200
serial-workbench events <session-id> --after 0 --limit 100
serial-workbench send <session-id> --data uptime --newline crlf --wait --details
serial-workbench task <task-id> --details
serial-workbench scripts list
serial-workbench scripts show <script-id>
serial-workbench scripts run <script-id> <session-id> --revision 1 --wait --details
```

按设备调整端口和命令；已有连接时复用 session ID。日志返回的 `next_cursor` 作为下次查询的 `after`，各查看者互不影响。

`--wait` 等待主机任务结束。没有响应检查时，单次发送显示 `sent`，脚本显示 `completed`；所有等待条件匹配后显示 `succeeded`。这些不是设备命令的退出码，取消也不能撤销已发送字节。

```powershell
serial-workbench takeover <session-id>
serial-workbench release <session-id>
serial-workbench cancel <task-id>
serial-workbench stop
```

`stop` 会关闭整个后台和所有会话；仅退出 CLI 或 MCP 客户端不会。

## MCP 接入

先启动后台，再让 MCP 客户端运行适配器：

```json
{
  "mcpServers": {
    "serial-workbench": {
      "command": "C:\\Tools\\SerialWorkbench\\serial-workbench.exe",
      "args": ["mcp"]
    }
  }
}
```

这是通用示例，配置位置取决于客户端；Linux/macOS 使用自行构建的可执行文件路径。MCP 不是必需的，有本地命令执行能力的 AI 也可以直接调用 CLI。

| 工具 | 用途 |
| --- | --- |
| `serial_ports`、`serial_sessions`、`serial_open` | 查看端口，复用或建立会话 |
| `serial_events` | 按游标读取日志，不消耗其他查看者的数据 |
| `serial_send`、`serial_macro` | 排队执行命令或 JSON 宏 |
| `serial_task`、`serial_cancel` | 查询命令、回包和状态，或取消任务 |
| `serial_scripts`、`serial_script_get`、`serial_script_save` | 列出、查看和保存共享脚本 |
| `serial_script_run`、`serial_script_delete` | 按版本运行或删除脚本 |
| `serial_control`、`serial_close` | 显式操作控制线或关闭会话 |

`serial_task` 默认返回详情，只轮询状态时传 `details:false`。AI 应先查看脚本内容，再带 `revision` 运行。你持有控制权时，AI 的新发送会被拒绝。

## 保存位置与升级

启动器将数据保存在程序旁的 `workbench-data/`。直接执行 `serve` 时相对于当前工作目录保存，也可通过 `--data-dir` 指定。

| 路径 | 内容 |
| --- | --- |
| `scripts.json` | 共享脚本库 |
| `tasks/<task-id>.json` | 命令、执行结果和有界的收发详情 |
| `<session-id>/session.json` | 端口参数、状态和计数 |
| `<session-id>/rx.log*` | 原始接收字节，每文件 10 MiB，每会话最多 5 个 |
| `<session-id>/events.jsonl*` | 带时间戳的事件，每文件 20 MiB，每会话最多 5 个 |

任务详情最多保留 64 KiB / 512 个事件，采集到结束后一秒或下一任务开始为止，可能包含设备主动输出的后台日志。缺口和截断会明确标注，较长回复请查会话历史。异常中断的任务不会自动重放。

旧浏览器脚本首次使用时迁入共享库，原本地备份保留；配色偏好仍按浏览器保存。升级时保留 `workbench-data`，更换可执行文件前停止后台。历史会话目录和任务文件不会自动清理。

日志可能包含凭据。「隐藏输入」会隐藏 TX 内容和任务详情中的收发窗口，但设备回显仍可能进入原始 RX 日志，分享前需要检查。

## 源码构建与验证

建议使用当前稳定版 Rust；此版本在 Windows / Rust 1.94.1 下验证。仓库已包含编译好的前端，普通构建无需 Node.js。

```bash
git clone https://github.com/sinkinrin/serial-workbench.git
cd serial-workbench
cargo build --release --locked --bins
cargo run --locked -- serve
```

修改前端需要 Node.js 22+：

```bash
cd web
npm ci
npm run check
npm test
npm run build
cd ..
cargo fmt --all --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
python -m pip install websockets
python scripts/smoke-workbench.py target/release/serial-workbench.exe
```

Linux/macOS 使用不带 `.exe` 的路径，Linux 可能需要系统的 libudev 开发包。本次只提供 Windows x64 成品包。自动冒烟测试使用隔离模拟设备；另外已验证真实 COM6/CH340 的采集和只读命令组。USB 拔插恢复及电气行为不在这些验证范围内。

## 来源与兼容性

基于 [Adancurusul/serial-mcp-server](https://github.com/Adancurusul/serial-mcp-server)，保留原始提交历史、MIT 许可和作者署名。Serial Workbench 从 **0.1.0** 开始独立版本系列。

兼容保留 `serial-mcp-server` 直接串口程序和 `serial_mcp_server` Rust 库名。直接模式不要打开工作台已占用的端口，详见[旧版 CLI/MCP](docs/LEGACY_CLI_ZH.md)；原有 `skills/serial-debug` 也使用该直接模式。宏 JSON 的 `schema_version` 继续为 `0.3`，与产品版本独立。

[MIT 许可证](LICENSE) · [更新记录](CHANGELOG.md)
