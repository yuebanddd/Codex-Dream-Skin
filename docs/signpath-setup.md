# SignPath Foundation 配置指南

这个文档记录仓库所有者在 SignPath Foundation 审核通过后需要完成的外部配置。仓库中的发布工作流已经按“失败关闭”设计：配置未完成前，`release` push 的 Windows job 会失败，不会发布未知发布者安装包。

## 1. 申请免费开源签名

1. 在 [SignPath Foundation](https://signpath.org/) 提交 LumaDrobe 申请。
2. 项目主页填写 `https://github.com/yuebanddd/Codex-Dream-Skin`。
3. 指向根目录的 [LICENSE](../LICENSE)、[PRIVACY.md](../PRIVACY.md) 与 [SIGNING_POLICY.md](../SIGNING_POLICY.md)。
4. 说明发布物为 Tauri 2 / Rust 桌面客户端，签名目标仅为本仓库构建的 Windows 主程序与 NSIS 安装器。
5. 所有维护者应为 GitHub 与 SignPath 账户启用多因素认证。

SignPath Foundation 会人工审核项目资格与声誉，仓库变更不能代替这一审核，也不能直接修改 Microsoft 云端声誉。

## 2. 连接可信构建系统

1. 在 SignPath 组织中添加预定义的 `GitHub.com` Trusted Build System。
2. 安装 SignPath GitHub App，并仅授权这个仓库。
3. 将 SignPath Project 连接到该 Trusted Build System。
4. 限制签名来源为 `.github/workflows/preview-packages.yml` 的 `release` 分支 push，以及 GitHub-hosted runner。
5. 使用需要人工批准的 release signing policy。免费 Foundation 证书要求每次发布由 Approver 批准。

工作流顶层只授予 `contents: read`，并在执行 PR 控制代码的 job 中禁用 checkout 凭据持久化。SignPath 通过只授权本仓库的 GitHub App 验证 Actions 来源并读取送签 Artifact；发布 Release 的 `contents: write` 只存在于独立的最终 publish job。

## 3. Artifact Configuration

GitHub `upload-artifact` 会把输入包装为 ZIP，因此 Artifact Configuration 根节点必须是 `<zip-file>`。将仓库中的 [`.signpath/artifact-configuration.xml`](../.signpath/artifact-configuration.xml) 导入 SignPath；它要求 ZIP 中恰好一个 `*.exe`，并仅对该文件应用 Authenticode 签名。工作流分别提交：

- `lumadrobe.exe`：打包前的应用主程序；
- `LumaDrobe-Setup.exe`：包含已签名主程序的 NSIS 安装器。

按照 SignPath Foundation 条件，配置对 PE 元数据施加强制限制：产品名必须是 `LumaDrobe`，产品版本必须等于工作流从 `package.json` 读取并传入的版本。不要在 SignPath 界面放宽或移除这些约束，也不要把订阅主题、Codex、旧版脚本或第三方可执行文件加入签名范围。

## 4. GitHub 配置

在仓库 Settings → Secrets and variables → Actions 中配置：

| 类型     | 名称                                   | 值                              |
| -------- | -------------------------------------- | ------------------------------- |
| Secret   | `SIGNPATH_API_TOKEN`                   | SignPath Submitter API Token    |
| Variable | `SIGNPATH_ORGANIZATION_ID`             | SignPath Organization ID        |
| Variable | `SIGNPATH_PROJECT_SLUG`                | LumaDrobe Project slug          |
| Variable | `SIGNPATH_SIGNING_POLICY_SLUG`         | Release signing policy slug     |
| Variable | `SIGNPATH_ARTIFACT_CONFIGURATION_SLUG` | EXE artifact configuration slug |

API Token 只授予提交指定项目和策略签名请求所需的最小权限。不要把 Token 写入变量、日志、文档或仓库文件。

## 5. 首次发布验证

合并版本 PR 后，在 Actions 中打开 `Preview packages`：

1. 等待 SignPath 请求出现，核对源仓库、工作流、`release` 提交 SHA 与版本。
2. 在 SignPath 中人工批准主程序请求；主程序签名完成后，工作流会打包并创建安装器请求。
3. 核对并批准安装器请求。
4. 确认 GitHub job 对两个文件都报告 `Valid`、`SignPath Foundation` 和时间戳。
5. 下载最终 Release，在干净 Windows 环境执行：

```powershell
Get-AuthenticodeSignature .\LumaDrobe-v0.5.5-Windows-x64-Setup.exe |
  Format-List Status, StatusMessage, SignerCertificate, TimeStamperCertificate
```

只在签名状态为 `Valid`、发布者严格等于 `SignPath Foundation` 且时间戳存在时分发。若 SignPath 尚未批准项目，请不要临时关闭签名门禁发布未签名的新版本。

如果首次合并时 SignPath 申请或仓库变量尚未完成，Windows job 会按预期失败。配置完成后，重新运行该次 `release` push 的失败 job；不要使用手动 `workflow_dispatch` 代替，因为手动运行只生成无签名 CI Artifact，不会发布 Release。
