# Linux Folder directory sync — development stage

Linux 桌面开发版现已接入共享 Rust 目录接收器。它是显式选择的单向 **Android → Linux**
模式，旧 Folder 不会自动转换。此次没有更新 USB 手机、VPS、已安装的桌面程序或真实配置。

## 设置

1. Linux **Add Folder** → 选择已有本地目录 → 打开 **Directory sync**。
   如希望自动检查，可同时打开 Automatically Receive。
2. 在 Connection/Pairing 中创建新的服务器 Folder，保留此设置窗口；Android 输入邀请代码。
   比较两端验证码，在 Linux **Codes match — confirm** 后继续。已有确认好的、尚未初始化
   目录同步的独立服务器 Folder 也可以使用其 scoped URL 和 receiver token。
3. 点击 **Review directory…**，后台扫描并校验目录，检查服务端状态；此时不注册 Folder，
   不创建接收任务、不发布目录、不修改目标文件。取消即可退出。
4. 检查确认框的目标路径、文件数量/大小和隐私说明，点击 **Initialize**。应用会重新扫描、
   复查目录身份和服务器状态；有变化则拒绝旧预览。确认后先持久化本地绑定，再发布路径、
   文件大小和 SHA-256 清单，不上传 Linux 文件内容。
5. Android 打开 Directory sync，选择源目录 → Preview changes → Initialize sync。
6. Linux 点击 **Receive**，或让 Automatically Receive 在应用打开时约每 60 秒检查一次。
   关闭设置窗口会恢复后续自动检查；不建立常驻系统服务，也不添加开机启动。

发布响应丢失或失败时，本地 Folder/接收状态仍保留，并提示用 Receive 重试；不要重复新建
或删除状态。若源文件先以旧投递方式进入服务器，必须先用原投递接收器处理完，不能让目录
初始化静默遗留这些任务。

## 界面与状态

- 继续使用简洁的 Folder 左侧栏、黑色重点控件、原文件筛选和成对方向排序。
- 目录列表显示实际接收过的**最新路径版本**，不是文件管理器的全目录浏览。
  初始化时相同而未传输的文件、Linux 独有文件不会伪造接收记录。
- 下载、校验、确认使用现有 Dash Ring 阶段动画，并保持进行中的文件优先。
- 已完成的正常文件不永久显示 Received；尚未确认的文件显示 Awaiting receipt。
- Conflict copy kept 表示本地不同内容已保留。旁边的历史图标可查看当前版本保留副本的
  完整路径；只展示路径，不自动执行或打开收到的文件。冲突会进入 Needs attention 筛选。
- 非冲突的正常更新也保留上个版本，仍可通过历史图标查看。当前未提供“已阅冲突”、
  完整历史时间线、自动回滚或自动清理；更早的副本留在磁盘上。
- 刷新只读取配置和接收账本，不恢复日志、不改目标文件、不请求网络。Receive 才会恢复
  未完成写入、接收、核验、发送回执及更新清单。

## 状态与兼容性

- Config 新增可选 `directory_sync = true`，旧配置省略此字段时仍为投递模式。
  旧版本程序会拒绝未知字段；新版本的旧 `mirelay sync`/壁纸 retry 也拒绝目录模式。
- 目录状态位于 `storage.state_file` 路径加 `.directory` 后缀的独立目录；不写旧投递账本，
  不运行壁纸命令。该目录、配置和本地文件应一起备份。
- 接收账本绑定服务器 Folder、目标路径、设备号和 inode。缺失、损坏、路径被替换或
  状态不匹配时拒绝接收，不能通过重建空状态“修好”。
- 模式和本地路径创建后固定；目录模式的服务器 URL 也固定。当前没有旧投递 Folder 的
  原地转换或 CLI 状态导入向导。迁移需保留旧数据和状态，建立新配对；不要重置现有远端清单。
- Session Token 只在本次进程内保留，配置只存环境变量名。重开应用需重新填写接收令牌，
  或在启动环境提供该配置指明的变量；尚未接入系统密钥环。
- 文件大小、目录扫描、符号链接、原子写入、冲突保留等限制继承
  [目录核心](directory-sync.md#limits-and-safety-boundaries)。仍需关注全量哈希和历史/缓存占用。
- 单向新增/修改同步，不联动删除、重命名删除、空目录、权限或时间戳；不是双向镜像或备份保证。
  不自动修复已经确认、后来又被外部删除的文件，也没有新增端到端加密。

## 验证

```bash
cargo test --locked --features desktop --lib --tests -j 2 -- --test-threads=2
cargo clippy --locked --features desktop --all-targets -j 2 -- -D warnings

# 需要独立图形会话；只启动临时配置的 GTK 子进程。
cargo test --locked --features desktop --test desktop_directory -- --ignored --test-threads=1
cargo test --locked --features desktop --test desktop_resilience -- --ignored --test-threads=1 --nocapture

# 创建新的纯本地显示样例，不接触服务器；不要复用含真实配置的目录。
cargo run --locked --features desktop --example frontend-fixture -- /absolute/new/preview --directory
target/debug/mirelay-desktop --new-instance --registry /absolute/new/preview/folders.toml --file-smoke-test
```

新测试覆盖：预览取消/无副作用、源变化/目录替换、远端已有状态、旧队列未清、发布失败恢复、
相对路径和任意二进制、同内容不同路径、更新/冲突/删除不联动、暂停自动检查、丢失状态、
改绑拦截、旧配置兼容、历史路径防越界、ACK 失败恢复，以及真实 GTK 自动接收和重启。

图形测试曾发现关闭设置后的标记未清理，现已修复并加入断言。测试诊断失败也改为返回普通
错误码，避免在 GTK 回调中因断言而 abort。一次无浏览器的 Broadway 主窗口截图没有生成
渲染节点；改用隔离配置的 X11 窗口检查原始截图，不把失败截图当作通过。

### 本轮结果（2026-09-05）

- 默认 Rust 回归：**194 通过、0 失败、7 项默认跳过**。新增 10 项桌面目录逻辑测试。
- 额外执行 5 项图形测试通过：真实 tus → 本地中继 → GTK Auto Receive，并重开 GTK 接收
  下一版本；另外覆盖损坏配置隔离、4 个启动时机的 SIGKILL、空闲采样、截图失败正常退出。
  默认跳过中的图形 5 项已单独执行；其余是显式 JNI smoke 和由父测试调用的崩溃子进程辅助项。
- GTK 文件交互和压力检查通过：包含模式开关/取消、关闭编辑器、历史副本弹窗、状态筛选、
  排序、活动文件优先、停止动画、旧流程兼容。另有 25 个非法动作目标、2 个陈旧索引及
  重复任务拒绝断言。
- 空闲 X11/debug GTK、两个 Folder、无传输，单次 8 秒采样：CPU 约 **0.25% 单核**，
  RSS 前后均为 **186,240 KiB**。不是后台耗电或跨机器性能保证。
- Broadway/debug 的通用历史列表压力测试（不含后续帧绘制）：10,000 条记录首屏名称排序
  中位数 **2.04 ms**；50,000 条为 **7.30 ms**，交替改变方向 **17.93 ms**。
  10,000 条下 100 次切换方向，RSS 增加 **32 KiB**；不据此宣称没有内存泄漏。
  目录模式本身仍受 5,000 路径上限限制，不支持 50,000 文件目录初始化。
- `cargo clippy --features desktop --all-targets -- -D warnings`、格式和 diff 检查通过。
- 纯显示样例与原始截图保留在 `/tmp/mirelay-linux-ui-dXmjMx/`，其中副本和 ACK 状态是明确
  标注的 UI 样例；实际协议与接收结论来自独立真实本地服务器测试，而不是这些静态截图。

尚未在本轮重新运行 Android 模拟器、实际手机或远程 VPS；没有部署新版，也没有自动迁移
真实 Folder。现阶段下一步是备份、部署前检查和真实跨端联测，不应直接覆盖现有状态。

后续已完成 [升级前检查与三条迁移演练](directory-upgrade-preflight-2026-09-05.md)，
固定构建产物已准备；真实部署仍需确认维护窗口和新鲜备份。

此后已获准并完成 [VPS 与手机升级](directory-deployment-2026-09-05.md)，原 Folder 没有自动转换。
