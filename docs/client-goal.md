# LumaDrobe Desktop · 产品目标

## 产品定位

LumaDrobe 是一个去中心化的 Codex 主题衣橱：任何人都可以用公开 GitHub 仓库发布皮肤，用户通过仓库地址订阅、浏览、导入、应用、导出和更新主题。

“Luma”代表光与氛围，“Drobe”来自 wardrobe。LumaDrobe 是独立产品名；`CODEX THEME ATELIER` 仅作为当前适配目标的产品副标题。

项目不依赖中心服务，不包含账户、支付或商业授权系统。

## 目标产品形态

- 深色侧栏承载主题衣橱、导入主题和恢复原生三个主入口
- 暖白主工作区展示主题系列
- 右侧固定“试衣镜”展示当前主题与主题色
- 主动作是“应用并启动”，同时提供导出和删除
- 底部持续展示皮肤引擎、平台和 CDP 状态
- 前端负责浏览、预览和交互；Rust 统一承接跨平台系统能力

## 技术边界

| 层               | 职责                                                        |
| ---------------- | ----------------------------------------------------------- |
| React UI         | 主题衣橱、订阅源、试衣镜、导入导出、诊断状态                |
| Tauri Commands   | 稳定、最小的前后端 IPC 接口                                 |
| Rust Services    | GitHub 订阅、主题缓存、安装、更新、Codex 发现、CDP 生命周期 |
| Renderer Payload | 随客户端发布的 JavaScript 适配器和 CSS 注入模板             |
| Theme Repository | 声明式清单、图片、可选 CSS，不允许下发 JavaScript           |

## 里程碑

### M1 · 客户端基础与订阅浏览

- Tauri 2 + React + Rust 工程
- GitHub 仓库订阅源协议 v1
- 添加、刷新、删除订阅源
- 主题衣橱和试衣镜
- 本地原子缓存

### M2 · 本地主题库

- [x] 下载和校验主题资源
- [x] 安装、更新和删除主题
- [x] 自定义 CSS 静态安全检查
- [x] 本地安装状态与离线主题目录
- [ ] 导出主题包
- [ ] 本地选图生成主题

### M3 · Rust 注入引擎

- [x] macOS / Windows Codex 发现与身份验证
- [x] Rust CDP WebSocket 客户端
- [x] 启动、热应用和恢复原生
- [x] Renderer 重载和路由变化自动重注入
- [x] 迁移现有脚本中的进程身份、端口和会话安全保护
- [x] 不退出 Codex 的临时暂停入口
- [x] 官方安装、进程、CDP 与渲染页只读诊断
- [ ] macOS / Windows 多版本实机兼容性矩阵

### M4 · 发布质量

- [x] Windows 与 macOS 无签名测试安装包
- [x] 构建元数据、提交号与 SHA-256
- [ ] Apple / Windows 正式签名
- [ ] 自动更新
- [ ] 兼容性矩阵和截图验收
- [ ] `release` 分支自动构建 GitHub Release

## 非目标

- 不修改 Codex 官方安装包、`app.asar` 或代码签名
- 不提供任意远程 JavaScript 执行能力
- 不实现 DRM、账户、支付或中心化皮肤市场
- 第一阶段不支持私有 GitHub 仓库
