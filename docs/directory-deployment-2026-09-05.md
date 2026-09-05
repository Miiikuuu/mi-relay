# Directory sync — VPS 与手机升级（2026-09-05）

用户确认维护窗口后，已将 [预检中的固定产物](directory-upgrade-preflight-2026-09-05.md)
部署到原 VPS 和指定 USB 手机。本记录更新前阶段“尚未部署”的状态，不表示真实 Folder
已经自动变成目录同步。

## VPS：升级与数据核验通过

- MiRelay 后端于 **14:42:38 UTC / 22:42:38 北京时间**重新启动，数据库 **2→3**。
  新进程 PID 25398，`NRestarts=0`。仅替换 MiRelay 后端，没有重启 Caddy、SSH 或其他代理。
  systemd 的停止/启动事件在同一个日志秒内；这不是端到端请求零中断保证。
- 新二进制 SHA-256：
  `548c778c5017fd98fa4c341659d94aab0793576e8a1d8e106b97e9c15bd4c4c9`。
- 新鲜、私有、落盘的生产恢复备份：
  **`/var/backups/mirelay/upgrade-2l_lzqau`**，位于 VPS。
  含旧二进制、环境文件及完整升级前数据；不应公开，也不能拿旧程序直接打开升级后的数据库。
- 升级前后逐项比较了所有旧表、环境文件、对象文件和 tus 状态文件；原来的 **1 个 Folder、
  2 个待接收文件、8 个已确认文件**全部保留。备份内的数据库、文件、凭据也与升级前快照一致。
- SQLite integrity 和外键检查通过；新目录表初始为空，旧 Folder 没有隐式初始化。
- 可信 HTTPS、未认证请求的 401、loopback-only 后端检查通过。没有关闭证书校验。
- Caddy/SSH/Hysteria/Shadowsocks 原进程身份及启动标记保持不变；Caddy 二进制、配置和两份
  MiRelay systemd unit 哈希未变。没有改动防火墙、SSH 或手机 Clash 设置。

## 手机：原地安装与迁移通过

- 对已授权的 USB 手机先校验原 APK 和签名，停止 MiRelay 后备份整个应用私有目录。
  没有卸载、清空数据、修改目录授权或打开新的 Auto 源。
- `adb install -r --no-streaming` 成功，安装后的 APK 哈希与固定产物一致：
  `b2e38028da136db0e29ad6d286a70f3f2b1e96ca50d511873ac96df094780f78`。
- 首次启动完成数据库 **3→4**，integrity/外键检查通过。**1 个 Folder、3 条传输记录、
  1 个 Auto 源**保留；Folder 身份、连接信息、加密令牌、原 Auto 设置和文件历史没有改变。
- 原 Auto 仍启用。首次严格逐列比较发现 `auto_sources.last_scan` 更新，检查后确认这是
  唯一差异；没有将它误判为数据丢失，也没有回滚数据库。验证器仅允许已启用源的这个运行时
  时间戳向前推进，其他旧列仍精确比较。另用合成记录验证配置/令牌变化及时间戳倒退仍被拒绝。
- 新目录表为空，旧源的 `directory_sync=0`，没有自动转换或开启目录同步。
- 最终启动成功，确认 `io.mirelay.android/.MainActivity` 为前台 Activity。
- 完成并关闭了 60 秒 MiRelay-UID 限定观察：4 次采样主进程 PID 均为 26597，
  收集 1,948 字节日志，未捕获 Java/native/JNI fatal 标记。PSS 样本为 77,050～150,914 KiB；
  只是短时启动后观察，不是长期无崩溃或低功耗保证。报告在
  `android/.local/phone-monitor-zcxhalt_/`。此前迁移检查期间的一轮观察没有采到运行中进程，
  不将那一轮空日志作为稳定性证据。

手机私有备份和验证脚本保留在本地：
`target/deployment/directory-rollout-sCjwh3/phone/`。
`before.tar`、`after.tar` 和旧 APK 已落盘；不要把这些文件提交 Git 或用于公开排障。
**Android Keystore 私钥不能导出**，这个备份不保证能把凭据恢复到另一部手机，不能通过卸载
来“修复”更新。这里验证了加密字段保留，不单独证明所有旧凭据已在手机上重新解密并联网。

## 独立 HTTPS 目录传输：通过

在升级后的真实 VPS 新建了独立 **Directory upgrade QA 2026-09-05** Folder。
发送端和接收端均是本次固定的 Linux directory CLI，所有文件都是合成样例。

- 配对确认前拒绝目录访问，确认后发布接收端清单；预览和初始化不发送文件。
- 两个各 **256 KiB**、内容相同但路径不同的二进制文件均按原路径到达，其中包含
  `Documents/更新.bin`；没有因为相同 SHA-256 丢掉其中一个路径。
- 原本相同的文件不重传，Linux 独有文件保留，本地不同内容保存冲突副本。
- 修改同一路径后接收新版本，并保存上个版本；真实 Linux 接收器校验、落盘后才发送 ACK。
- 再次发送/接收没有新增记录，最终 **3 个最新路径、4 个已接收版本、0 个待处理测试版本**。
- 测试结束撤销了合成发送凭据。保留 1 个空闲诊断 Folder 及 4 条已确认的合成版本元数据；
  没有删除真实 Folder，也没有从真实旧队列下载或确认文件。
- 测试后再次核对：升级前全部旧记录/对象/上传状态仍保留，真实待接收数仍为 **2**。
  服务端总数变为 2 个 Folder、12 个已确认版本，其中增加的 4 个来自上述测试。

完整私有测试记录：`target/deployment/directory-rollout-sCjwh3/https-directory/`。
其中 `private-folder.json` 含接收凭据，不能公开整个目录。服务端升级核验副本也在
`target/deployment/directory-rollout-sCjwh3/`；VPS 侧证据在
`/root/mirelay-directory-upgrade-VdF1xd/`。

## 范围与下一步

这次完成了真实 VPS 部署、真实手机原地升级及 **Linux→VPS→Linux** 的目录模式验收。
**没有将此测试冒充真实手机→VPS→Linux的目录初始化与后台送达测试**：该环节仍需要用户
选择新配对的源/目标目录并确认初始化。

- 手机新版已经可以打开使用；原来的投递 Folder 和 Auto 继续保持原模式。
- 按 [Linux 设置流程](desktop-directory-sync.md#设置) 建立新 paired Folder，选择已有目标目录、
  打开 Directory sync 并确认初始化；再在手机选择源目录、预览并确认。
- 用新配对和专用测试目录先跑通真实手机链路，不复用诊断 CLI 的接收状态，不自动转换旧 Folder。
- 本轮没有替换或启动用户默认 Linux 桌面配置，以免其 Auto 消费那 2 个真实待接收文件。
  最新桌面构建仍在 `target/release/mirelay-desktop`，固定副本在
  `target/deployment/directory-ready-TL9k3q/mirelay-desktop`。
- 没有常驻监控、Git 提交或推送；未来的证书续期、长期后台调度仍需后续观察。

后续已按用户请求完成 [真实手机 → VPS → Linux 已有目录初始化验收](phone-directory-init-2026-09-05.md)。
该次使用新增测试 Folder 和合成文件，原 Folder、真实源目录仍未自动转换。
