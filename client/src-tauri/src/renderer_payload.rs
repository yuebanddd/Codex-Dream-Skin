use crate::error::{AppError, AppResult};
use crate::models::InstalledSkin;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

const IMAGE_LIMIT: u64 = 32 * 1024 * 1024;
const CSS_LIMIT: u64 = 1024 * 1024;

const BASE_CSS: &str = r#"
html.lumadrobe-theme {
  color-scheme: dark;
  --lumadrobe-panel: rgba(10, 12, 18, .84);
  --lumadrobe-line: rgba(255, 255, 255, .10);
}
html.lumadrobe-theme body {
  background: #090b10 var(--lumadrobe-art) center / cover fixed no-repeat !important;
}
html.lumadrobe-theme aside.app-shell-left-panel {
  background: linear-gradient(180deg, rgba(12, 13, 19, .96), rgba(12, 13, 19, .88)) !important;
  border-color: var(--lumadrobe-line) !important;
  backdrop-filter: blur(18px) saturate(112%) !important;
}
html.lumadrobe-theme main.main-surface {
  background:
    linear-gradient(115deg, rgba(8, 10, 15, .94) 0%, rgba(8, 10, 15, .70) 42%, rgba(8, 10, 15, .22) 100%),
    var(--lumadrobe-art) center / cover no-repeat !important;
}
html.lumadrobe-theme main.main-surface [role="main"] {
  background: transparent !important;
}
html.lumadrobe-theme .composer-surface-chrome {
  border-color: var(--lumadrobe-line) !important;
  background: var(--lumadrobe-panel) !important;
  backdrop-filter: blur(18px) saturate(116%) !important;
}
"#;

pub fn build_payload(installed: &InstalledSkin) -> AppResult<String> {
    let background = read_verified_asset(
        installed,
        Path::new(&installed.background_path),
        IMAGE_LIMIT,
        "背景图",
    )?;
    let mime = image_mime(&background)?;
    let art_data_url = format!("data:{mime};base64,{}", STANDARD.encode(background));
    let custom_css = if let Some(path) = &installed.css_path {
        let bytes = read_verified_asset(installed, Path::new(path), CSS_LIMIT, "主题 CSS")?;
        std::str::from_utf8(&bytes)
            .map_err(|_| AppError::Runtime("已安装的主题 CSS 不是 UTF-8".into()))?
            .to_owned()
    } else {
        String::new()
    };
    let css = format!("{BASE_CSS}\n{custom_css}");
    let theme = json!({
        "key": format!(
            "{}:{}@{}",
            installed.source_id, installed.skin_id, installed.version
        ),
        "id": installed.skin_id,
        "name": installed.name,
        "version": installed.version,
    });
    let theme_json = serde_json::to_string(&theme)?;
    let css_json = serde_json::to_string(&css)?;
    let art_json = serde_json::to_string(&art_data_url)?;

    Ok(format!(
        r#"((theme, cssText, artDataUrl) => {{
  const STATE_KEY = "__LUMADROBE_RUNTIME__";
  const STYLE_ID = "lumadrobe-theme-style";
  const previous = window[STATE_KEY];
  if (previous?.themeKey === theme.key && previous?.ensure) {{
    previous.ensure();
    return {{ installed: true, themeKey: theme.key, reused: true }};
  }}
  previous?.cleanup?.();

  const comma = artDataUrl.indexOf(",");
  const binary = atob(artDataUrl.slice(comma + 1));
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) bytes[index] = binary.charCodeAt(index);
  const mime = artDataUrl.slice(5, comma).split(";")[0];
  const artUrl = URL.createObjectURL(new Blob([bytes], {{ type: mime }}));
  const touched = new Set();

  const ensure = () => {{
    const root = document.documentElement;
    if (!root) return;
    root.classList.add("lumadrobe-theme");
    root.dataset.lumadrobeTheme = theme.key;
    root.style.setProperty("--lumadrobe-art", `url("${{artUrl}}")`);
    let style = document.getElementById(STYLE_ID);
    if (!style) {{
      style = document.createElement("style");
      style.id = STYLE_ID;
      (document.head || root).appendChild(style);
    }}
    if (style.textContent !== cssText) style.textContent = cssText;
    for (const node of document.querySelectorAll("main.main-surface")) {{
      node.classList.add("lumadrobe-surface");
      touched.add(node);
    }}
  }};

  const observer = new MutationObserver(() => queueMicrotask(ensure));
  observer.observe(document.documentElement, {{ childList: true, subtree: true }});
  const timer = setInterval(ensure, 5000);
  const cleanup = () => {{
    observer.disconnect();
    clearInterval(timer);
    document.documentElement?.classList.remove("lumadrobe-theme");
    if (document.documentElement?.dataset.lumadrobeTheme === theme.key) {{
      delete document.documentElement.dataset.lumadrobeTheme;
    }}
    document.documentElement?.style.removeProperty("--lumadrobe-art");
    document.getElementById(STYLE_ID)?.remove();
    for (const node of touched) node.classList?.remove("lumadrobe-surface");
    URL.revokeObjectURL(artUrl);
    if (window[STATE_KEY]?.themeKey === theme.key) delete window[STATE_KEY];
    return true;
  }};
  window[STATE_KEY] = {{ themeKey: theme.key, ensure, cleanup, observer, timer, artUrl }};
  ensure();
  return {{ installed: true, themeKey: theme.key, reused: false }};
}})({theme_json}, {css_json}, {art_json})"#
    ))
}

fn read_verified_asset(
    installed: &InstalledSkin,
    path: &Path,
    limit: u64,
    label: &str,
) -> AppResult<Vec<u8>> {
    let root = PathBuf::from(&installed.install_dir).canonicalize()?;
    let asset = path.canonicalize()?;
    if !asset.starts_with(&root) {
        return Err(AppError::Runtime(format!("{label} 已离开主题安装目录")));
    }
    let metadata = asset.metadata()?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > limit {
        return Err(AppError::Runtime(format!("{label} 文件大小无效")));
    }
    let name = asset
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| AppError::Runtime(format!("{label} 文件名无效")))?;
    let expected = installed
        .asset_hashes
        .get(name)
        .ok_or_else(|| AppError::Runtime(format!("{label} 缺少安装时的 SHA-256")))?;
    let bytes = std::fs::read(&asset)?;
    let actual = format!("{:x}", Sha256::digest(&bytes));
    if !actual.eq_ignore_ascii_case(expected) {
        return Err(AppError::Runtime(format!("{label} 已在安装后被修改")));
    }
    Ok(bytes)
}

fn image_mime(bytes: &[u8]) -> AppResult<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Ok("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Ok("image/jpeg")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Ok("image/webp")
    } else {
        Err(AppError::Runtime(
            "已安装背景图不再是受支持的 PNG、JPEG 或 WebP".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::SkinManifest;
    use std::collections::BTreeMap;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn fixture() -> (PathBuf, InstalledSkin) {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "lumadrobe-payload-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let background = root.join("background.png");
        let bytes = b"\x89PNG\r\n\x1a\nfixture";
        std::fs::write(&background, bytes).unwrap();
        let mut hashes = BTreeMap::new();
        hashes.insert(
            "background.png".into(),
            format!("{:x}", Sha256::digest(bytes)),
        );
        let skin = InstalledSkin {
            source_id: "source".into(),
            source_name: "Source".into(),
            skin_id: "night".into(),
            name: "Night".into(),
            version: "1.0.0".into(),
            author: "Tester".into(),
            installed_at: "now".into(),
            install_dir: root.to_string_lossy().into_owned(),
            background_path: background.to_string_lossy().into_owned(),
            preview_path: None,
            css_path: None,
            asset_hashes: hashes,
            manifest: SkinManifest {
                schema_version: 1,
                id: "night".into(),
                name: "Night".into(),
                version: "1.0.0".into(),
                description: None,
                author: "Tester".into(),
                engine_version: None,
                platforms: vec![],
                preview: None,
                background: "background.png".into(),
                css: None,
                colors: BTreeMap::new(),
            },
        };
        (root, skin)
    }

    #[test]
    fn builds_a_static_payload_from_verified_assets() {
        let (root, skin) = fixture();
        let payload = build_payload(&skin).unwrap();
        assert!(payload.contains("__LUMADROBE_RUNTIME__"));
        assert!(payload.contains("data:image/png;base64"));
        assert!(payload.contains("source:night@1.0.0"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_assets_changed_after_install() {
        let (root, skin) = fixture();
        std::fs::write(&skin.background_path, b"\x89PNG\r\n\x1a\nchanged").unwrap();
        assert!(build_payload(&skin).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
