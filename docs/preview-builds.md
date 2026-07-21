# LumaDrobe 测试安装包

v0.5 的目标是让 LumaDrobe 可以在真实 macOS 和 Windows 环境中安装、诊断和反馈问题。当前默认生成无签名测试包，参考 [CC Switch 当前 Windows 工作流](https://github.com/farion1231/cc-switch/blob/613fef70bc7d5e35299b4131935f738c85765b35/.github/workflows/release.yml)的 unsigned Authenticode 策略；SignPath Foundation 就绪后可由仓库变量显式切换为签名模式。macOS 产物仍为无签名测试包。

## 构建产物

`Preview packages` GitHub Actions 工作流生成三个 Artifact：

| Artifact                | Runner         | 安装包   |
| ----------------------- | -------------- | -------- |
| `LumaDrobe-macOS-arm64` | `macos-15`     | DMG      |
| `LumaDrobe-Windows-x64` | `windows-2025` | NSIS EXE |

每个 Artifact 还包含：

- `BUILD-INFO.json`：版本、构建提交、平台、架构、文件大小和哈希
- `SHA256SUMS.txt`：安装包 SHA-256

PR、`release` 分支 push 和手动 `workflow_dispatch` 都会构建，且不使用 npm 依赖缓存。`SIGNPATH_ENABLED` 未设为精确的 `true` 时，Windows 包按 unsigned 模式构建和发布；设置为 `true` 后，`release` push 先签名主程序、再打包并签名 NSIS 安装器。两个目标平台完成后会在独立的 release gate 中汇总并复验版本、提交、实际签名声明和 SHA-256；最终 Artifact 保留 14 天，送签输入仅保留 1 天。

每次 PR 合并到 `release` 后，工作流都会创建 `v<版本>-preview.<工作流编号>` GitHub Pre-release。unsigned 模式会在 Release Notes 和 `BUILD-INFO.json` 明确声明未签名，不会伪装成可信发布者；Release Notes 只使用 release gate 从已验证元数据输出的签名状态，不会在重跑 publish job 时重新读取可变仓库配置。SignPath 模式需要每次发布人工批准；批准后工作流验证 Windows 主程序与安装器的签名、发布者和时间戳，签名被拒绝、超时或配置缺失时不会降级发布未签名替代包。Assets 中的安装包使用可直接识别的平台文件名：

- `LumaDrobe-v<版本>-Windows-x64-Setup.exe`
- `LumaDrobe-v<版本>-macOS-arm64.dmg`

每个平台还提供对应的 `BUILD-INFO.json` 和 `SHA256SUMS.txt`。同一工作流重跑时会覆盖原 tag 的资产和说明，不会创建重复版本。

## 校验下载内容

先在 GitHub Actions 页面确认工作流来自本仓库，再核对 `BUILD-INFO.json` 中的 `buildCommit` 与目标提交一致。

macOS：

```bash
shasum -a 256 LumaDrobe_*.dmg
cat SHA256SUMS.txt
```

Windows PowerShell：

```powershell
Get-FileHash .\LumaDrobe_*.exe -Algorithm SHA256
Get-Content .\SHA256SUMS.txt
Get-AuthenticodeSignature .\LumaDrobe_*.exe | Format-List Status, StatusMessage, SignerCertificate, TimeStamperCertificate
```

若 `BUILD-INFO.json` 的 `signed` 为 `true`，Windows Release 的 `Status` 必须是 `Valid`，签名者名称必须严格等于 `SignPath Foundation`，并且必须存在时间戳证书。若 `signed` 为 `false`，`Get-AuthenticodeSignature` 通常显示 `NotSigned`；SHA-256 只能证明下载内容与发布资产一致，不能建立发布者身份或避免 Smart App Control 拦截。

## 平台签名状态

- macOS Gatekeeper 可能阻止首次打开。仅在确认仓库、提交和 SHA-256 后，通过“系统设置 → 隐私与安全性”对该应用选择“仍要打开”。不要全局关闭 Gatekeeper。
- 当前 Windows 预览版默认没有 Authenticode 签名，可能被 Smart App Control 直接阻止。CC Switch 的 unsigned Windows 包可能已因实际分发积累云端判断结果，但微软没有公开声誉阈值，复制其构建方式不能保证新文件立即放行。
- SignPath 模式一旦启用，Windows Release 必须带 `SignPath Foundation` 签名且门禁失败关闭。有效签名可以建立发布者信任，但 Microsoft 的云端声誉仍由系统评估，项目无法在 GitHub Actions 中直接写入或伪造。

SignPath 申请与仓库配置见 [SignPath 配置指南](./signpath-setup.md)。macOS 正式发行前仍需要 Apple Developer ID 与公证，自动更新也需要独立设计和签名。

## 实机反馈

安装后打开“运行诊断”，依次测试：

1. 发现官方 Codex。
2. 安装并应用一套主题。
3. 热切换另一套主题。
4. 暂停并恢复主题。
5. 退出并重开 LumaDrobe，确认运行或暂停状态恢复。
6. 恢复原生并重启 Codex。

如有问题，直接提供持久运行日志 `runtime.jsonl`；无需再手动导出诊断 JSON。日志达到 1 MiB 后轮换为同目录的 `runtime.previous.jsonl`。运行诊断页面会显示日志的完整路径。

日志自动记录客户端版本、构建提交、主题标识、Codex/CDP 身份、路径、端口、目标数量、失败阶段，以及受数量和长度限制的 DOM 结构指纹。每个目标单独限时，某个页面无响应时仍会保留目标列表和其他已完成结果。结构指纹只包含标签名、元素 ID、角色、测试 ID、class 名和布尔标记；不采集页面标题、文本、表单值、URL 查询参数、存储内容或对话。日志也不写入主题 CSS/图片、API Key 或 `auth.json`。macOS/Linux 日志文件固定为仅所有者可读写的 `0600`；日志仍可能包含本机路径和 CDP 目标标识，公开提交前请先检查并移除不希望披露的信息。

主题应用使用分阶段日志定位渲染器故障：`cdp_theme_stage_started` 记录元数据、CSS、图片的载荷大小、摘要和分片数，`cdp_theme_metadata_transferred` 与 `cdp_theme_css_transferred` 记录前两类数据传输完成，`cdp_theme_art_progress` 记录图片分片里程碑，`cdp_theme_install_queued` 表示异步安装已经排队，`cdp_theme_install_phase` 记录元数据校验、CSS 校验、图片组装校验和挂载阶段，最终由 `cdp_theme_install_completed` 或 `cdp_theme_install_failed` 收口。若安装排队后渲染器失去响应，`cdp_theme_install_terminal` 或 `watcher_install_terminal` 会带有 `retrySuppressed: true`，表示客户端已主动停止自动回退和重复注入。
