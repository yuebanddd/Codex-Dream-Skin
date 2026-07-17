# LumaDrobe 测试安装包

v0.5 的目标是让 LumaDrobe 可以在真实 macOS 和 Windows 环境中安装、诊断和反馈问题。当前产物是无签名测试包，不是正式发行版。

## 构建产物

`Preview packages` GitHub Actions 工作流生成三个 Artifact：

| Artifact                | Runner         | 安装包   |
| ----------------------- | -------------- | -------- |
| `LumaDrobe-macOS-arm64` | `macos-15`     | DMG      |
| `LumaDrobe-Windows-x64` | `windows-2025` | NSIS EXE |

每个 Artifact 还包含：

- `BUILD-INFO.json`：版本、构建提交、平台、架构、文件大小和哈希
- `SHA256SUMS.txt`：安装包 SHA-256

PR、`release` 分支 push 和手动 `workflow_dispatch` 都会构建测试包，且不使用 npm 依赖缓存。macOS arm64 与 Windows x64 完成后还会在独立的 release gate 中汇总并复验发布资产；Artifact 保留 14 天。

每次 PR 合并到 `release` 后，工作流会在两个目标平台构建全部成功并重新校验版本、提交和 SHA-256 后，自动创建 `v<版本>-preview.<工作流编号>` GitHub Pre-release。Release 正文会列出下载说明，Assets 中的安装包使用可直接识别的平台文件名：

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
```

只有哈希完全一致时才继续安装。

## 无签名提示

- macOS Gatekeeper 可能阻止首次打开。仅在确认仓库、提交和 SHA-256 后，通过“系统设置 → 隐私与安全性”对该应用选择“仍要打开”。不要全局关闭 Gatekeeper。
- Windows SmartScreen 可能显示未知发布者。仅在确认仓库、提交和 SHA-256 后，为这个安装包单次选择继续运行。不要全局关闭 SmartScreen。

正式发行前仍需要 Apple Developer ID、公证、Windows 代码签名和自动更新签名。

## 实机反馈

安装后打开“运行诊断”，依次测试：

1. 发现官方 Codex。
2. 安装并应用一套主题。
3. 热切换另一套主题。
4. 暂停并恢复主题。
5. 退出并重开 LumaDrobe，确认运行或暂停状态恢复。
6. 恢复原生并重启 Codex。

如有问题，直接提供持久运行日志 `runtime.jsonl`；无需再手动导出诊断 JSON。日志达到 1 MiB 后轮换为同目录的 `runtime.previous.jsonl`。运行诊断页面会显示日志的完整路径。

日志自动记录客户端版本、构建提交、主题标识、Codex/CDP 身份、路径、端口、目标数量、失败阶段，以及受数量和长度限制的 DOM 结构指纹。结构指纹只包含标签名、元素 ID、角色、测试 ID、class 名和布尔标记；不采集页面标题、文本、表单值、URL 查询参数、存储内容或对话。日志也不写入主题 CSS/图片、API Key 或 `auth.json`。macOS/Linux 日志文件固定为仅所有者可读写的 `0600`；日志仍可能包含本机路径和 CDP 目标标识，公开提交前请先检查并移除不希望披露的信息。
