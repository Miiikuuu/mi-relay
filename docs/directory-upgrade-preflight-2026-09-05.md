# Directory sync — 升级前检查（2026-09-05）

本轮完成发布产物准备和隔离升级演练，**没有部署到 VPS 或手机**，没有替换已安装桌面程序，
没有迁移真实 Folder。下一步需要用户确认一次带备份的短暂停机升级。

后续用户已确认，VPS 和手机升级现已完成，见 [实际部署与验收记录](directory-deployment-2026-09-05.md)。
下文保留当时的预检证据，不代表当前仍未部署。

## 实际环境：只读检查

- 测试 VPS（主机别名已隐去）：MiRelay 和 HTTPS 代理均 active，检查前后 PID 分别保持 24271、22966，
  `NRestarts=0`，后端二进制哈希未变，loopback `/healthz` 成功。
- 服务端数据库 schema **2**，integrity 为 `ok`、外键错误为 0。本次快照有 **1 个 Folder、
  2 个待接收文件、8 个已确认文件**；没有读取文件正文、发送回执或清理队列。
- VPS 文件系统可用约 5.3 GiB；这不是完整备份容量验收，正式升级前还需检查实际状态大小。
- USB 手机在线，当前安装 APK 哈希仍是
  `954be0e3c2fb3c2454d5dcbc3ed4a33390cbd39f5c38443f48d3084fcba1fd20`。
  只读取包信息并拉取 APK，没有停止应用、导出应用私有数据、改变 Auto 或选择源目录。
- 新旧 APK 均通过签名检查，证书 SHA-256 一致：
  `cd032d879b4ee4dda9c8cfc73742eb8417993e97cf141de1832a752aec3e84a1`。
  这证明原地更新的签名兼容，不等于本次已经完成手机数据库迁移。

上述状态只代表检查当时；正式升级前应再次检查，不能据此假设队列一直不变。

## 本轮修改与测试

`deploy/rehearse-upgrade.py` 不再把源/目标 schema 写死为 1→2，默认验证 2→3，仍支持显式
1→2 和 1→3。升级前在一次只读事务内快照全部旧表，升级后逐行比较，并检查完整性和外键。
演练的旧版 2 二进制是从实际 VPS 只读复制并核对哈希的；数据全部为合成样例。

三条升级路径全部通过，覆盖：

- 错误校验和不停止服务；备份复制/fsync 失败后恢复原来的后端。
- 先完成独立私有备份，再升级；已有 legacy/admin 令牌不轮换。
- 2→3 保留已确认和未认领的配对，并保留 scoped tus 上传的 65,536 字节偏移。
- 旧投递数据/普通 tus 继续可读和续传，旧 Folder 不隐式初始化为目录模式。
- 新 Folder 完成配对后发布目录清单；非图片 `Documents/更新.bin` 在中途重启后续传，
  检查原路径、版本、字节和 SHA-256；旧投递列表不消费目录版本。
- 目录回执拒绝错误角色/哈希，重复回执不抹掉冲突标记；确认目录版本不会删掉其他队列
  仍引用的相同内容。
- 新版启动后健康检查失败不回退数据库；用旧二进制和备份在**另一目录**恢复旧数据库、
  配对和 tus 偏移。

这是真实二进制、文件系统、SQLite 和 HTTP 的隔离演练，但 systemctl 由子进程控制器代替。
目录 ACK 是验证字节后由测试接收器发送，不冒充手机或 GTK 的实际跨设备送达。
演练中的 `/var/backups/mirelay/upgrade-*` 只存在于临时 namespace，随演练销毁，
**不是本轮 VPS 生产备份**。

另外：

- 部署脚本测试 **12 通过**，包括新增的 schema 参数防呆、精确记录对比、只读/缺失数据库、
  外键错误阻断和不可变产物副本测试。
- Linux GTK / directory CLI / legacy CLI Release 构建成功；Release GTK 文件交互检查通过，
  使用上一阶段的隔离 Folder 样例，没有访问真实配置。
- x86_64 musl 静态 PIE 服务端 Release 构建成功。
- Android ARM64 / x86_64 JNI、APK 和测试 APK 构建成功；JVM **39 通过、0 失败**。
- Android lint 无错误，保留 10 条既有建议（9 条依赖提示、1 条 KTX 建议）；SDK XML 和
  apksigner/JDK 有工具版本提示，没有在本轮升级工具链。
- 更新 Android 初始化提示：现在引导用户先在 Linux Folder 界面初始化目标目录，
  不再声称目录接收器只能使用开发 CLI。
- `cargo fmt --all -- --check` 和 `git diff --check` 通过。

最初一次 musl 构建因项目内交叉编译工具链不在 PATH 而失败；显式指定原有工具链后成功。
演练最初发现 `/tmp` 中输入被隔离挂载遮住，现改为独立校验副本并只读挂载；目录样例还曾
因 filename 与路径末段不一致被产品正确拒绝，已修正测试元数据。最终三个完整演练退出码均为 0。
本轮未重跑 Android 模拟器完整测试和全部 Rust 回归；前阶段结果分别见
[Android 记录](android-directory-sync.md) 和 [Linux 记录](desktop-directory-sync.md)。

## 固定产物

已将本次构建的独立副本保留在忽略 Git 的本地目录：
`target/deployment/directory-ready-TL9k3q/`。
目录还包含读取的旧服务端、当前手机 APK 和演练日志；它不包含本轮真实应用数据备份。

| 文件 | SHA-256 |
| --- | --- |
| `mirelay-server` | `548c778c5017fd98fa4c341659d94aab0793576e8a1d8e106b97e9c15bd4c4c9` |
| `mirelay-desktop` | `ddecaf8d045ee09d16e487445d9a8861c098c5ff0db2fe55dffc204751918ba9` |
| `mirelay-directory` | `08a363f162458aa12f5eb9e655c17e6c3a98d06c1cb4f05f28e1398edf4f6fba` |
| `mirelay` | `66d7c6816ff9e7cd1a82e8c443d919b1ada2e8924abc13530c01c533087abe48` |
| `app-debug.apk` | `b2e38028da136db0e29ad6d286a70f3f2b1e96ca50d511873ac96df094780f78` |

Android 仍为开发签名且 versionCode=1 / versionName=0.1.0-dev；后续必须检查 APK 哈希，
不能只看版本显示来判断是否更新。新构建若改变哈希，应重新核对产物，不盲目套用本记录。

## 获准后的升级顺序（尚未执行）

1. 确认维护窗口，检查剩余磁盘，短暂停止 MiRelay 发送端并采集新鲜、私有、落盘的备份。
   手机备份不能导出 Android Keystore；绝不能卸载或清空数据来绕过原地安装失败。
2. 记录 VPS 旧表/凭据指纹和服务身份，校验固定二进制，经 `setup.py upgrade --apply`
   仅停止/替换 MiRelay 后端。SSH、Clash/代理、Caddy 和防火墙不改动。
3. 核对 schema 2→3、旧数据/令牌/断点保留、可信 HTTPS 和鉴权；新版开始收数据后不盲目回退。
4. 带备份原地安装手机 APK，核对 schema 3→4、原 Folder/凭据/Auto/传输记录；不自动启用目录同步。
   不复用 `target/deployment/upgrade-phone.py` 或旧 audit 脚本中的旧哈希/schema 常量。
5. 启动新的 Linux 桌面版本，用**新配对和专用测试目录**测试真实手机→VPS→Linux，
   覆盖已有文件初始化、新增、修改、冲突副本和真实回执。现有 2 个待接收文件不纳入测试。
6. 最后再由用户选择实际源/目标目录并确认初始化；不原地转换旧投递 Folder。

没有创建常驻监控、启动模拟器、提交或推送 Git。
