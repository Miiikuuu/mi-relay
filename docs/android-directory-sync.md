# Android directory sync — development stage

Android 开发版已接入单向目录同步，保留原来的临时投递和 Delivery Auto。
本阶段只使用隔离模拟器和本地服务器，未更新 USB 手机或正式 VPS。

后续状态：已获准完成 [真实手机与 VPS 的带备份升级](directory-deployment-2026-09-05.md)。
以下保留开发阶段的测试记录；真实源目录仍需用户在新配对中显式初始化。

## 使用流程

1. 创建并确认一个 **paired Folder**。旧的共享令牌/根 URL Folder 不会自动转换。
2. Linux 用 [Folder 界面的 Directory sync](desktop-directory-sync.md) 初始化已有目标目录并发布清单。
   也可使用 [Rust 目录 CLI](directory-sync.md#isolated-cli-workflow)；不要给同一个 Folder 建立两份独立接收状态。
3. Android 打开该 Folder → Auto → Directory sync → Choose source directory。
   已有 Delivery Auto 必须先暂停；未完成的旧传输需先完成。
4. 点击 **Preview changes**，查看相同、缺失、不同和 Linux 独有文件。
   只预览不会启用 Auto 或创建上传任务；预览状态可在应用重启后恢复。
5. 点击 **Initialize sync**。应用重新检查两端清单，任何变化都要求重新预览。
   确认后才允许缺失/不同文件及以后的新增/修改进入自动队列。
6. Linux 再次运行接收命令。Android 的 **Refresh receipts** 可手动读取送达状态；
   后续目录检查也会刷新。上传完成显示 Waiting for Linux，真实回执后才显示 Synced。

原文件名和子目录来自 SAF 展示名称，而不是 document ID。相同字节放在两个路径，
会保留为两个文件。同大小、同时间戳但字节变化的文件也会被发现。
这里是单向内容同步，不是双向镜像：删除不联动，重命名视为新路径，旧目标保留；
Linux 独有文件不删除。冲突保留两份，Linux CLI 输出历史副本路径。

## 调度、恢复与安全

- 保留 SAF **只读**权限，不新增全盘权限、常驻监听或永远运行的服务。
- 常规检查约每 30 分钟一次，Android 可推迟；Check now 也受网络、电量、空间限制。
- 默认只用 unmetered 网络。更改此策略前先 Pause sync，再 Resume sync。
- 每次最多暂存 20 个文件、总计 100 MiB。有成功入队且仍有待处理文件时，
  才接续受约束的短批次；每轮最多 250 次。无进展不持续循环，仅保留一次稳定性复查。
- 新发现的内容通常需间隔至少 10 秒的稳定观察；显式确认的初始化清单可立即入队。
- Pause sync 保留路径清单、版本号、暂存数据和 tus 偏移。Resume sync 更新任务所有权，
  旧 worker 不能回写状态，也不会重新建立忽略历史的 baseline。
- 初始化后不能把同一 Folder 指向另一源目录；需要新 Folder。权限丢失可选择原目录
  Restore source access 后恢复。目录任务的单文件重试不会绕过暂停或网络策略。
- SQLite v3→v4 保留旧 Folder、加密令牌、配对状态、Auto baseline 和上传记录。
  路径状态、版本分配与传输插入在同一事务内提交；暂存内容上传前再次核对记录的 SHA-256。
- 回执绑定 Folder、路径、版本与哈希。被后续版本取代的记录显示 Superseded，
  不冒充该旧版本实际到达 Linux。

## 当前限制

- 每文件非空且不超过 100 MiB；目录最多 5,000 个节点、16 层深度，不再限制目录总字节数。
  哈希复用一个 64 KiB 缓冲区，预览/确认在连续 90 秒无读取进展时取消；后台目录任务另有
  8 分钟绝对时限，超时不接受部分清单。普通 Auto/导入仍保留原有绝对时限。
  每批暂存最多 100 MiB。目录路径历史最多 5,000 条，不通过清空历史偷偷绕过限制。
- 虚拟文件、缺少大小/修改时间、空文件、超限、不完整/循环列表会拒绝完整扫描。
  路径不做静默重命名：拒绝越界、分隔符、保留的 `.mirelay*` 名称等。
- 当前对目录全量哈希，尚无大目录增量扫描优化。只验证了 API 36 隔离模拟器，
  不能据此保证各手机厂商的省电策略、所有 SAF/provider 或长时间后台表现。
- 大预览只展示前 50 条改变路径，同时显示全部数量；元数据总大小有 8 MiB 上限。
- 只同步文件内容及路径，不同步空目录、时间戳、权限或 ACL。不提供已 ACK 文件丢失后的自动修复。
- Linux 历史副本与本地对象缓存尚无自动保留期清理，需要关注磁盘空间。
- 本次未增加端到端加密。HTTPS 和 Folder 令牌保护传输/权限，中继仍能看到文件及清单。

## 验证

构建与自动化命令：

```bash
bash android/scripts/build.sh :app:assembleDebug :app:assembleDebugAndroidTest :app:testDebugUnitTest :app:lintDebug
cargo build --locked --bin mirelay-directory --bin mirelay-server --bin mirelay
python3 android/scripts/device-tests.py
```

测试脚本只允许 `MiRelay_API36_QA` / `emulator-5580`，会清空这个专用 AVD 的应用数据，
不会连接物理手机。测试专用 RPC 只存在于本地 QA 代理中，不加入产品服务器；它使用隔离
Folder 的接收凭据，在当次报告的临时目录内调用真实 Linux CLI。

首轮模拟器测试报告：`android/.local/device-qa-x1wu_55_/`。
4 个新增 ART/界面测试通过，其中一项完成“已有源目录 → JNI/tus → 中继 → Linux 原路径”
链路，并验证同大小修改、冲突副本、删除不联动和暂停后的自动补传。

### 最终验证（2026-09-05）

完整报告：`android/.local/device-qa-xc7sr6a8/`，测试进程退出码为 0。
测试 APK 的 SHA-256：
`3cc526c097051d92a1d1fbc64af4d24138adfb38689f4416f995608223c0b048`。

- Android JVM：**39 通过，0 失败**，其中 14 项目录状态/迁移/回执/哈希保护测试。
- Android API 36 模拟器：**54 通过，0 失败**，包括目录同步 5 项、原运行时 13 项、
  原 Auto 12 项、界面 23 项、拒绝通知权限 1 项。
- 两轮额外 SIGKILL 恢复通过：分别在普通投递和 Delivery Auto 上传中杀进程，重启后
  续传并核对持久回执；这不是目录模式专属的进程断电测试。
- 新增批量测试：45 个已有文件在初始化后自动分批接续，无需手动 Check now，
  45 个版本唯一且全部收到 Linux ACK。
- 实际 Linux 接收：49 个同步路径（45 个批量路径 + 4 个多版本路径），以及 11 个旧共享
  令牌投递、1 个 paired 投递文件；按原路径、字节、SHA-256 和回执检查通过。
- 已查看原始 `visual/directory-preview.png`、`visual/directory-synced.png`，确认预览内容、
  初始化按钮和冲突回执可读。日志检索未发现 FATAL EXCEPTION、Fatal signal 或 ANR in；
  不能据此推断所有设备、后台时长及性能都已覆盖。
- ARM64/x86_64 Rust JNI、APK 与测试 APK 构建通过；Rust 回归 184 通过、6 项跳过；
  JNI Clippy（warnings denied）通过。Android lint 0 错误，保留 9 条依赖更新提示与
  1 条 KTX 写法建议；SDK 工具另有 XML 版本提示，未在本次功能中升级依赖工具链。

手机和 VPS 未更新，也未自动转换任何真实 Folder。后续 [Linux 界面阶段](desktop-directory-sync.md)
已接入目录接收模式；手机/服务器仍需另行安排带备份的升级。不要把模拟器通过等同于物理手机已经可用。
