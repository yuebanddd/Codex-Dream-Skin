# Codex Dream Skin Desktop

跨平台 Codex 主题衣橱。桌面壳采用 Tauri 2，前端采用 React，订阅、缓存、进程控制和 CDP 生命周期统一由 Rust 实现。

## 当前阶段

v0.1 打通第一个纵向切片：

- 添加公开 GitHub 仓库作为皮肤订阅源
- 支持 `?ref=branch-or-tag` 固定分支、标签或提交
- 读取 `dreamskin-source.json` 和皮肤清单
- 在主题衣橱中浏览、选择和预览主题
- 原子缓存订阅源数据

皮肤安装、导出、Rust CDP 注入和恢复入口已在 UI 中预留，将在后续里程碑接入。

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
