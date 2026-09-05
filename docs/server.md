# MiRelay 中心服务器

`mirelay-server` 是 HTTP API v1 的 Rust 参考实现。当前 MVP 面向单个 Linux 目标设备：Bearer token 唯一标识该设备。服务端支持 tus 1.0 可恢复上传；仓库中的 Linux 发送器作为 Android 客户端的协议测试替身。

## 本地启动

先生成并保存一个随机设备令牌：

```bash
export MIRELAY_SERVER_TOKEN="$(openssl rand -hex 32)"
export MIRELAY_TOKEN="$MIRELAY_SERVER_TOKEN"
```

启动服务器：

```bash
cargo run --bin mirelay-server -- \
  --data-dir /tmp/mirelay-server \
  --device-id linux \
  serve --listen 127.0.0.1:8080
```

另一个终端上传文件。状态文件在上传确认完成前一直保留：

```bash
cargo run --bin mirelay-upload -- ./wallpaper.png \
  --server-url http://127.0.0.1:8080 \
  --state-file /tmp/mirelay-upload.json \
  --allow-insecure-http
```

显式模拟一次中断并恢复：

```bash
cargo run --bin mirelay-upload -- ./wallpaper.png \
  --server-url http://127.0.0.1:8080 \
  --state-file /tmp/mirelay-upload.json \
  --chunk-size-bytes 4096 \
  --max-chunks 1 \
  --allow-insecure-http

# 使用相同参数恢复，但移除 --max-chunks
cargo run --bin mirelay-upload -- ./wallpaper.png \
  --server-url http://127.0.0.1:8080 \
  --state-file /tmp/mirelay-upload.json \
  --chunk-size-bytes 4096 \
  --allow-insecure-http
```

另一个终端初始化并运行 Linux 客户端：

```bash
cargo run -- --config /tmp/mirelay-client/config.toml init \
  --data-dir /tmp/mirelay-client/data \
  --server-url http://127.0.0.1:8080 \
  --allow-insecure-http

cargo run -- --config /tmp/mirelay-client/config.toml sync
```

查看服务端队列状态：

```bash
cargo run --bin mirelay-server -- \
  --data-dir /tmp/mirelay-server \
  --device-id linux \
  status
```

校验 pending 对象并清理 MiRelay 拥有的无引用对象：

```bash
cargo run --bin mirelay-server -- \
  --data-dir /tmp/mirelay-server \
  reconcile
```

若存在被引用但丢失或损坏的对象，命令会打印统计并以非零状态退出；无法识别的目录或文件只报告、不删除。

## 存储和可靠性

- 元数据保存在 `mirelay-server.sqlite3`，数据库启用 WAL、`synchronous=FULL` 和 5 秒 busy timeout。
- 未完成 tus 上传保存在 `uploads/`：已提交偏移写入原子替换的 JSON 记录，数据文件先 `fsync`，再推进偏移。若进程在两步之间终止，下次 `HEAD`/`PATCH` 会把未提交尾部截回记录偏移。
- 上传达到声明长度后会重新验证大小、SHA-256 和按内容识别的媒体类型，再以 upload UUID 作为 delivery ID 幂等入队；响应丢失或完成阶段重试不会生成重复任务。无效内容返回 `422`，服务端删除对应 tus 记录和暂存数据。
- 文件先经过大小、SHA-256 和内容类型校验，再 `fsync` 并原子提交到 `content/<sha-prefix>/<sha>.<ext>`。未知二进制使用 `application/octet-stream` 和 `.bin`；原文件名保留在元数据中。
- SQLite 记录只会在内容可靠落盘后提交，因此 pending 任务不会指向尚未完成的文件。
- 分页 cursor 固定首次请求时的队列高水位；扫描期间新增的任务留到下一次同步，不会扰乱当前分页。
- ACK 在 SQLite 事务中持久化并保持幂等。最后一个 pending 引用被确认后，服务端内容对象自动删除，ACK 元数据继续保留。
- `reconcile` 会重新哈希所有 pending 对象，清理崩溃暂存与无引用的合法内容对象。HTTP 服务启动时也会执行一次，并对缺失或损坏的引用发出警告。
- 数据目录在 Unix 上限制为 `0700`，数据库为 `0600`。令牌只从环境变量读取，不进入数据库或命令行参数。

## 网络边界

服务器本身只提供 HTTP，默认监听 `127.0.0.1:8080`。非回环监听默认被拒绝；生产环境应让 Caddy、Nginx 或其他反向代理提供 HTTPS，并把请求转发到回环地址。

只有隔离测试网络才应使用：

```bash
mirelay-server serve --listen 0.0.0.0:18080 --allow-public-http
```

健康检查为 `GET /healthz`，无需鉴权且不会返回队列信息。tus 上传及投递 API 的完整请求和响应格式见 [HTTP API v1](http-api-v1.md)。

## systemd 示例

独立部署所需的低权限服务单元、HTTPS 配置模板和验收脚本见
[deploy/README.md](../deploy/README.md)。模板默认关闭 HTTP/3，避免与占用 UDP 443
的 Hysteria 冲突；不要直接覆盖现有代理配置。

令牌文件 `/etc/mirelay/server.env` 应由 root 持有并设置为 `0600`；系统级
systemd 在切换到运行用户前读取 `EnvironmentFile`，服务用户无需直接读取该文件：

```ini
MIRELAY_SERVER_TOKEN=replace-with-a-random-token
```

服务单元示例：

```ini
[Unit]
Description=MiRelay delivery server
After=network.target

[Service]
Type=simple
User=mirelay
Group=mirelay
EnvironmentFile=/etc/mirelay/server.env
ExecStart=/usr/local/bin/mirelay-server --data-dir /var/lib/mirelay-server --device-id linux serve --listen 127.0.0.1:8080
Restart=on-failure
RestartSec=3
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=/var/lib/mirelay-server

[Install]
WantedBy=multi-user.target
```

公开部署前还需要配置 HTTPS 反向代理、防火墙和备份策略。不要把测试 token 复用到生产环境。
