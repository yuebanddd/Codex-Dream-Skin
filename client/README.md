# LumaDrobe Desktop

LumaDrobe 是跨平台 Codex 主题衣橱。桌面壳采用 Tauri 2，前端采用 React，订阅、下载、缓存和后续 CDP 生命周期统一由 Rust 实现。

## 当前阶段

v0.2 已打通“订阅—浏览—安全安装—离线保留—删除”的本地主题库闭环：

- 添加公开 GitHub 仓库作为皮肤订阅源
- 支持 `?ref=branch-or-tag` 固定分支、标签或提交
- 读取 `dreamskin-source.json` 和皮肤清单
- 在主题衣橱中浏览、选择和预览主题
- 下载并校验 PNG、JPEG、WebP 与可选 CSS
- 限制资源体积，阻止 CSS 远程加载、`@import` 与危险协议
- 记录本地资源 SHA-256，并以原子 JSON 文件维护安装状态
- 取消订阅后仍可浏览已安装主题，并支持安全删除

主题包导出、本地选图创建、Rust CDP 注入和恢复入口将在后续里程碑接入。

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
npm run build
cd src-tauri
cargo fmt --check
cargo test
```

协议定义见 [`../docs/skin-source-spec-v1.md`](../docs/skin-source-spec-v1.md)。
