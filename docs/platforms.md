# 平台对照

## 运行模型（两边相同）

```text
LumaDrobe Desktop（React + Rust）
    │  验证官方应用身份，启动 loopback CDP
    ▼
官方 Codex Desktop（不改 asar / 签名）
    │  Rust CDP 注入内置适配器 + 主题静态 CSS
    ▼
仍使用原生项目、任务、对话与配置
```

订阅仓库只能提供声明式清单、PNG/JPEG/WebP 和通过静态检查的 CSS。Renderer JavaScript 适配器随 LumaDrobe 发布，不从 Git 仓库下载。

## LumaDrobe v0.5 安全边界

- CDP 固定绑定 `127.0.0.1`，仅从平台首选端口后的 100 个端口中选择空闲端口
- 每次注入前验证监听进程属于已验证的官方 Codex
- macOS 校验 `com.openai.codex`、完整代码签名和 Team ID `2DC432GLL2`
- Windows 只接受非开发模式、`SignatureKind=Store` 的 `OpenAI.Codex` Appx 包，并从受验证清单构造 AUMID 后由 Rust 调用系统 MSIX 激活器
- `/json/version` 的浏览器 ID用于锚定会话；身份改变后停止自动重注入
- WebSocket 只接受同端口 loopback 地址和 `/devtools/page/<target-id>` 形状
- 目标必须是已验证浏览器会话中的 `app://` 页面，且页面内再次确认 `app:` 协议与文档就绪；Codex 私有 class 只作为诊断提示，不作为兼容性门槛
- Rust 内置适配器统一写入 v1 订阅 CSS 的兼容根类、背景变量和必要装饰 DOM，订阅源无需携带或启动 renderer 脚本
- 恢复操作在重新验证应用、监听者与浏览器 ID 后才关闭并重启 Codex
- 暂停操作先持久化暂停意图并等待重注入 watcher 退出，再从已验证页面移除主题
- 运行诊断只执行身份、进程与 CDP 只读检查，不修改 Codex 或主题状态
- CDP 或页面兼容失败时自动写入受限结构指纹；每个目标单独限时并保留已完成结果，不记录页面文本、表单值、标题、查询参数或存储内容
- Windows 的 Codex 启动与恢复由 Rust 原生 `IApplicationActivationManager` 完成；系统身份检查由客户端内部执行，辅助进程使用无控制台窗口模式，不依赖外置脚本
- 不修改官方应用包、`app.asar`、代码签名、API Key、Base URL 或 `~/.codex/config.toml`

## 路径速查

| 用途           | macOS                                               | Windows                          |
| -------------- | --------------------------------------------------- | -------------------------------- |
| LumaDrobe 数据 | `~/Library/Application Support/com.dreamskin.codex` | `%APPDATA%\\com.dreamskin.codex` |
| 已安装主题     | 应用数据目录下 `themes/`                            | 应用数据目录下 `themes/`         |
| 订阅缓存       | `sources.json`                                      | `sources.json`                   |
| 本地主题库     | `installed-skins.json`                              | `installed-skins.json`           |
| 活动会话       | `runtime.json`                                      | `runtime.json`                   |
| 运行日志       | `logs/runtime.jsonl`                                | `logs/runtime.jsonl`             |
| 首选 CDP 端口  | `9341`                                              | `9335`                           |

实际应用数据根目录由 Tauri `app_data_dir` 解析；表中路径用于说明平台位置，不应由业务代码手工拼接。

## 客户端能力矩阵

| 功能                    |   macOS   | Windows  |
| ----------------------- | :-------: | :------: |
| 官方应用发现与身份验证  |    ✅     |    ✅    |
| Rust CDP 启动与注入     |    ✅     |    ✅    |
| 会话内热切换            |    ✅     |    ✅    |
| Renderer 重载自动重注入 |    ✅     |    ✅    |
| 无重启暂停与恢复        |    ✅     |    ✅    |
| 只读运行诊断            |    ✅     |    ✅    |
| 无签名测试安装包        | DMG arm64 | NSIS x64 |
| 恢复原生并重启          |    ✅     |    ✅    |
| Git 仓库订阅与本地安装  |    ✅     |    ✅    |
| 实机兼容性矩阵          |  待验收   |  待验收  |

`macos/` 与 `windows/` 下的旧脚本继续保留为兼容性参考，新的桌面产品能力以 `client/` 为准。

测试安装包由 GitHub Actions 在干净 runner 上构建，附带构建提交与 SHA-256；当前不包含 Apple 或 Microsoft 正式签名，不应作为正式发行版传播。

## 不要提交的内容

- API Key、`.codex/auth.json`
- 中转站密钥、服务器私钥
- 含用户隐私的实机截图
- 从远程主题仓库加载的可执行 JavaScript
