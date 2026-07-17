# LumaDrobe

LumaDrobe 是使用 Rust 与 Tauri 构建的跨平台 Codex 主题衣橱。主题来自公开 Git 仓库，客户端负责订阅、校验、安装和运行时注入。

## 下载桌面客户端

前往 **[GitHub Releases](https://github.com/yuebanddd/Codex-Dream-Skin/releases)**，展开最新版本的 **Assets**，按系统下载：

| 系统                | 安装包文件名                              |
| ------------------- | ----------------------------------------- |
| Windows x64         | `LumaDrobe-v<版本>-Windows-x64-Setup.exe` |
| macOS Apple Silicon | `LumaDrobe-v<版本>-macOS-arm64.dmg`       |
| macOS Intel         | `LumaDrobe-v<版本>-macOS-x64.dmg`         |

当前安装包是未签名预览版。每个平台同时提供 `BUILD-INFO.json` 和 `SHA256SUMS.txt`，用于核对构建提交和安装包哈希。

## 项目状态

- 桌面客户端：React + Tauri 2 + Rust
- 主题来源：公开 GitHub 仓库
- 运行时注入：Rust CDP 客户端，不修改 Codex 安装包或签名
- 支持平台：Windows x64、macOS arm64、macOS x64

开发与测试说明见 [client/README.md](client/README.md)，主题源协议见 [docs/skin-source-spec-v1.md](docs/skin-source-spec-v1.md)。
