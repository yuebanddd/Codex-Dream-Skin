# LumaDrobe Desktop

LumaDrobe 是跨平台 Codex 主题衣橱。桌面壳采用 Tauri 2，前端采用 React，订阅、下载、缓存和后续 CDP 生命周期统一由 Rust 实现。

## 当前阶段

v0.5 已进入可安装预览阶段：

- 添加公开 GitHub 仓库作为皮肤订阅源
- 支持 `?ref=branch-or-tag` 固定分支、标签或提交
- 读取 `dreamskin-source.json` 和皮肤清单
- 在主题衣橱中浏览、选择和预览主题
- 下载并校验 PNG、JPEG、WebP 与可选 CSS
- 限制资源体积，阻止 CSS 远程加载、`@import` 与危险协议
- 记录本地资源 SHA-256，并以原子 JSON 文件维护安装状态
- 取消订阅后仍可浏览已安装主题，并支持安全删除
- 发现并验证 macOS 官方签名应用或 Windows Store 包身份
- 只在 `127.0.0.1` 启动 CDP，并验证端口监听进程、浏览器身份与页面路径
- 用 Rust WebSocket 客户端注入声明式主题，并在 Renderer 重载后自动重注入
- 支持“应用并启动”、会话内热切换以及“恢复原生并重启”
- 持久化最小运行状态，客户端重启后重新验证身份再恢复监护
- 支持不退出 Codex 的皮肤暂停与恢复，并持久化暂停状态
- 提供只读运行诊断，检查官方安装、进程、回环监听、CDP 会话和渲染页
- 可由 GitHub Actions 构建 macOS DMG 与 Windows NSIS 无签名测试安装包
- 失败时自动记录安全的 CDP/DOM 结构指纹，可一键打开日志位置，无需手动导出诊断 JSON
- 构建产物记录版本与提交，安装包附带 SHA-256

主题包导出、本地选图创建和更多 Codex 版本实机兼容性将在后续迭代接入。

## 本地运行

```bash
cd client
npm install
npm run dev
```

只预览前端：

```bash
npm run dev:web
```

## 检查

```bash
npm run check
npm run check:version
npm run build
cd src-tauri
cargo fmt --check
cargo test
```

Rust 注入引擎当前仅支持 macOS 与 Windows。Linux 可以运行纯函数测试，但不能启动 Codex Desktop。

测试安装包的获取、校验与安全提示见 [`../docs/preview-builds.md`](../docs/preview-builds.md)。

协议定义见 [`../docs/skin-source-spec-v1.md`](../docs/skin-source-spec-v1.md)。
