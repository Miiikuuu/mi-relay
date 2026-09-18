# 已有目录初始化：真实手机 → VPS → Linux 验收

2026-09-05，用户请求代为准备并执行初始化测试。本轮使用已连接的 vivo 手机、现有 HTTPS
VPS，以及正在运行的 Linux GTK 应用；不是模拟器或 Linux 发送端替代测试。

## 范围

- 只新增一个独立服务器 Folder，Linux 名称为 `Init test`，Android 名称为 `Init-test`。
- Linux 目标：用户 Documents 下的隔离测试目录（个人主目录路径已隐去）。
- 手机源：`Documents/MiRelay-init-test-rNrCzF`，由系统文件选择器授权此测试目录。
- 所有文件都是合成的小文本，不初始化真实 Pixiv 目录，不转换、删除或重置 `test02`。
- 服务器 Folder 通过 HTTPS 管理 API 创建；Android 使用普通添加界面认领邀请，核对两端
  实际显示的验证码后，由 Linux 界面确认。目录预览、初始化、检查、接收均走两端应用界面。
- 管理员凭据仅在创建时私下读取，不写入测试报告或手机。新测试 Folder 的私有接收凭据
  留在权限受限的诊断目录中，以便恢复仅保存在 Linux 会话内的令牌；不要公开整个目录。

## 验证结果

| 场景 | 结果 |
| --- | --- |
| 两端同路径、同内容 `same.txt` | 预览为 identical，没有上传版本或伪造接收记录 |
| 手机独有 `sub/new.txt` | 按原文件名和子目录到达，字节一致 |
| 同路径不同内容 `different.txt` | 接收手机版本，原 Linux 内容保留为冲突历史副本 |
| Linux 独有 `linux-only.txt` | 内容保持不变，没有上传到手机 |
| 初始化后的新增 `after-init.txt` | 手机检查后上传，Linux 正常收到 |
| 初始化后的同路径修改 | 新版本收到，手机第一版和原 Linux 版两个历史副本都仍存在 |
| 重复手机检查和 Linux Receive | 服务端版本、接收账本均无变化，没有重复传输 |
| 真实送达回执 | 上传后先显示 Waiting for Linux；Linux 校验落盘、ACK 后手机显示 Synced |

Linux 首次预览为 **3 个已有文件 / 150 B**，只发布目录清单；手机预览为
**1 identical / 1 missing / 1 different / 1 Linux-only**。确认手机预览前，服务端没有上传版本。

总共接收 **4 个版本、3 个最新路径**；最终最新版本全部 ACK，手机显示 3 条 Synced，旧版
`different.txt` 显示 Superseded。通过实际手机文件读取、Linux 文件字节、保留副本、接收账本
及 HTTPS 目录状态交叉核验。上传的合成文件内容共 **172 B**，不包括协议和 TLS 流量。

`test02` Linux 配置的测试前后 SHA-256 完全一致。原 Folder 没有被重新配对，原有目录没有
被本轮操作读入同步清单或写入测试文件。

## 收尾与留存

- 手机测试期间临时允许此 Folder 使用移动网络；结束后已恢复 **Unmetered network only**，
  并确认 **Paused · version history kept / 0 waiting / 0 need attention**。
- Linux 测试 Folder 的 Automatically Receive 保持关闭；两端测试目录、配对、历史及回执保留。
- 没有卸载、清空数据、重启服务器、修改 Clash、防火墙或现有 Folder 的 Auto 策略。
- 私有诊断目录：`target/phone-directory-test-rNrCzF/`，包括初次和更新核验记录、最终摘要，
  以及 `phone-preview.png`、`phone-synced.png`、`phone-paused.png`。其中
  `private-folder.json` 含测试凭据，不能提交 Git 或公开分享。

本轮只证明小目录的初始化、单向新增/修改、冲突保留和回执链路。后续修改测试使用 Check now
触发，并经历稳定性复查；没有将它当作熄屏后的 30 分钟系统调度、长时间后台、性能或崩溃测试。
当前模式仍不传播删除，不是双向镜像。测试工具遇到输入法转换、辅助控制及轮询字段错误后
进行了调整；未将这些失败尝试计为通过，也未因此重建真实 Folder 或跳过应用的配对/目录确认。
