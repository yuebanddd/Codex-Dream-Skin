# Dream Skin Git Repository Source Protocol v1

## 入口

订阅仓库根目录必须包含 `dreamskin-source.json`。客户端可接受：

```text
https://github.com/owner/repository
https://github.com/owner/repository?ref=release
```

未指定 `ref` 时，客户端通过 GitHub Repository API 获取默认分支。

## 订阅源清单

```json
{
  "schemaVersion": 1,
  "id": "example-source",
  "name": "Example Dream Skins",
  "author": "example",
  "description": "Optional description",
  "skins": [
    {
      "id": "portal-night",
      "manifest": "skins/portal-night/skin.json"
    }
  ]
}
```

## 皮肤清单

```json
{
  "schemaVersion": 1,
  "id": "portal-night",
  "name": "Portal Night",
  "version": "1.0.0",
  "description": "Dark portal workspace",
  "author": "example",
  "engineVersion": ">=0.1.0",
  "platforms": ["macos", "windows"],
  "preview": "skins/portal-night/preview.webp",
  "background": "skins/portal-night/background.webp",
  "css": "skins/portal-night/theme.css",
  "colors": {
    "accent": "#7cff46",
    "secondary": "#36d7e8",
    "highlight": "#642a8c"
  }
}
```

所有资源路径均相对于仓库根目录，不允许绝对路径、空路径、反斜杠或 `..`。

## 安全模型

- 订阅源不得包含需要执行的 JavaScript
- Renderer 注入脚本只能随客户端版本发布
- 自定义 CSS 必须经过用户信任确认和静态检查
- 客户端拒绝 CSS 中的远程资源、`@import` 和仓库外路径
- 客户端下载资源后验证清单记录的哈希；哈希字段将在 v1.1 加入
- 第一版只读取公开 GitHub 仓库，不接收或保存 GitHub Token

## 限制

- 单个订阅源最多 500 套皮肤
- `id` 最多 80 个 ASCII 字符，只允许字母、数字、`.`、`_`、`-`
- Git ref 最多 200 个字符
- 资源路径最多 500 个字符
