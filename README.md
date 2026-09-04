# MiRelay

MiRelay 是一个隐私优先、可自托管的跨设备文件投递工具。当前仓库包含 Linux 端 Rust CLI 和 Rust 中心服务器 MVP，实现任意非空文件的可靠暂存、接收、校验和确认送达；图片还可以自动进入桌面壁纸工作流。

## 当前可用功能

- XDG 兼容的配置与数据目录
- Rust 中心服务器：SQLite 持久队列、稳定快照分页和 ACK 后内容回收
- tus 1.0 分块上传：Linux 测试发送器可在进程退出后通过 `HEAD` 偏移续传
- 真实 HTTP 服务端源：Bearer 鉴权、稳定游标分页、条件下载与幂等 ACK
- 有界 HTTP 自动重试：指数退避、抖动、`Retry-After` 和中断下载整次重启
- 本地文件收件箱，用于离线开发和验证完整流程
- 原生 GTK 4 Linux 桌面端：Folder 导航、属性与活动视图、Folder 设置、手动接收和可选自动接收
- 流式大小限制、SHA-256 校验和基于内容的媒体类型识别
- 临时文件 `fsync`、内容寻址存储、无覆盖原子提交
- 持久化投递状态和幂等 `(delivery_id, sha256)` ACK
- 跨进程状态锁与媒体库锁，避免并发同步、清理或重复导入
- 仅面向图片、无 shell 的可配置壁纸命令、超时和不确定结果保护
- 损坏 manifest 隔离：单个坏任务不会阻断其他任务

MiRelay 对 PNG、JPEG、GIF、WebP、AVIF、PDF、ZIP、gzip、7z、RAR、MP3、WAV、FLAC、Ogg、MP4 和 WebM 做文件签名识别；合法 UTF-8 文本保存为 `text/plain`，其他非空内容安全回退为 `application/octet-stream`。因此未知格式也可以投递，但不会仅凭原文件扩展名获得受信任的媒体类型。非图片文件正常落盘并 ACK，壁纸状态记为 `not_applicable`，不会调用壁纸命令。

## 快速开始

```bash
cargo build
cargo run -- init
```

默认配置位于 `~/.config/mirelay/config.toml`，数据位于 `~/.local/share/mirelay/`。也可以使用隔离目录体验完整流程：

```bash
cargo run -- --config /tmp/mirelay/config.toml init \
  --data-dir /tmp/mirelay/data

cargo run -- --config /tmp/mirelay/config.toml mock enqueue ./wallpaper.png
cargo run -- --config /tmp/mirelay/config.toml sync
cargo run -- --config /tmp/mirelay/config.toml status
cargo run -- --config /tmp/mirelay/config.toml list
```

`mock enqueue` 会先发布 payload，再原子发布 manifest，模拟服务端“任务可见时内容已经完整”的约束。

### Linux 桌面端

桌面端使用 GTK 4 与 libadwaita，并作为可选 feature 构建，因此服务器和 CLI 的默认无桌面构建不会依赖 GUI 库。Debian/Ubuntu 开发环境先安装：

```bash
sudo apt install libgtk-4-dev libadwaita-1-dev
```

启动：

```bash
cargo run --features desktop --bin mirelay-desktop
```

桌面端可以管理多个 Folder，每个 Folder 对应一份独立配置、状态和受管目录。左侧新增或选择 Folder，右侧查看目录、连接、自动接收规则和最近传输。自动接收按 Folder 独立启用，默认关闭；启用后，应用打开期间每 60 秒检查一次。`--config /path/to/config.toml` 会导入并选择已有 CLI 配置；`--registry /path/to/folders.toml` 可隔离桌面 Folder 注册表。Token 不写入配置或注册表：留空时读取配置指定的环境变量（默认 `MIRELAY_TOKEN`），手动输入时只保留在当前桌面进程内存中。更多说明见 [Linux 桌面端文档](docs/desktop.md)。

### 连接 HTTP 服务端

令牌只从环境变量读取，不写进配置文件：

```bash
export MIRELAY_TOKEN='replace-with-device-token'

cargo run -- --config /tmp/mirelay/config.toml init \
  --data-dir /tmp/mirelay/data \
  --server-url https://mirelay.example

cargo run -- --config /tmp/mirelay/config.toml sync
```

对应配置如下：

```toml
[server]
kind = "http"
base_url = "https://mirelay.example"
token_env = "MIRELAY_TOKEN"
request_timeout_seconds = 120
page_size = 50
retry_max_attempts = 3
retry_base_delay_milliseconds = 250
retry_max_delay_milliseconds = 5000
allow_insecure_http = false
```

生产环境默认强制 HTTPS。本地联调 HTTP 服务时，需要在 `init` 增加 `--allow-insecure-http`。服务端实现遵循 [HTTP API v1](docs/http-api-v1.md)。

## 中心服务器

生成测试 token 并启动回环 HTTP 服务：

```bash
export MIRELAY_SERVER_TOKEN="$(openssl rand -hex 32)"

cargo run --bin mirelay-server -- \
  --data-dir /tmp/mirelay-server serve --listen 127.0.0.1:8080
```

另一个终端可用 Linux tus 发送器代替尚未实现的 Android 客户端：

```bash
export MIRELAY_TOKEN="$MIRELAY_SERVER_TOKEN"

cargo run --bin mirelay-upload -- ./wallpaper.png \
  --server-url http://127.0.0.1:8080 \
  --state-file /tmp/mirelay-upload.json \
  --allow-insecure-http
```

要显式验证跨进程断点续传，第一次增加 `--chunk-size-bytes 4096 --max-chunks 1`，随后用相同文件、服务器和状态文件重新执行且去掉 `--max-chunks`。完成后任务进入 pending 队列，Linux 接收 CLI 的 `sync` 会下载、校验并 ACK。

服务器默认拒绝公网明文监听；生产部署应在前面放置 HTTPS 反向代理。详细命令、上传链路、存储模型与 systemd 示例见 [中心服务器文档](docs/server.md)。`mirelay-upload` 是 Android 上传协议的 Linux 测试替身；Android 应用与多设备令牌管理仍是后续阶段。

## 壁纸命令

初始化后编辑配置中的 `wallpaper.command`。命令以参数数组执行，不经过 shell；`{path}` 必须是独立参数。还支持 `{id}` 和 `{sha256}` 文本替换。

例如，使用 `swww`：

```toml
[wallpaper]
command = ["swww", "img", "{path}"]
timeout_seconds = 30
```

使用 `feh`：

```toml
[wallpaper]
command = ["feh", "--bg-fill", "{path}"]
timeout_seconds = 30
```

壁纸命令只会在受支持的图片可靠落盘并 ACK 后执行；其他文件永远跳过。如果 CLI 在命令执行期间中断，该任务会进入 `uncertain` 状态，不会自动重跑。确认重复执行安全后使用：

```bash
cargo run -- retry --include-uncertain
```

积压任务按服务端创建时间从旧到新导入；对于直接设置当前壁纸的命令，最新投递会最后执行。

## 投递顺序与崩溃恢复

```text
扫描待处理投递元数据
→ 流式复制到 MiRelay 专用 staging 目录
→ 校验大小、SHA-256 和真实媒体类型
→ fsync 临时文件
→ 原子提交到 <sha256-prefix>/<sha256>.<ext>
→ fsync 目标目录
→ 持久化 ack_pending 状态
→ 幂等 ACK
→ 持久化 acknowledged 状态
→ 若为图片，执行壁纸命令
```

连接失败、超时、响应体中断、`408`、`429` 和可恢复的 `5xx` 会在配置的预算内自动重试。退避包含抖动并遵守受最大等待时间约束的 `Retry-After`；摘要、ETag 或媒体格式错误不会重试。

下次 `sync` 会继续重试 `ack_pending`，并复用已经校验过的内容寻址文件。如果 `ack_pending` 对应的本地对象已经损坏，但服务端仍保留该 pending 投递，CLI 会隔离损坏对象并完整重新下载。壁纸失败不会触发重新下载或重复 ACK。

## 开发检查

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```
