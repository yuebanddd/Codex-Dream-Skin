use crate::error::{AppError, AppResult};
use crate::models::InstalledSkin;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

const IMAGE_LIMIT: u64 = 32 * 1024 * 1024;
const CSS_LIMIT: u64 = 1024 * 1024;
pub(crate) const RENDERER_ENGINE_VERSION: u64 = 2;

#[derive(Debug, Clone)]
pub struct RendererPayload {
    theme_key: String,
    theme_json: String,
    css: String,
    art_mime: String,
    art: Vec<u8>,
    engine: String,
}

impl RendererPayload {
    pub fn theme_key(&self) -> &str {
        &self.theme_key
    }

    pub fn theme_json(&self) -> &str {
        &self.theme_json
    }

    pub fn css(&self) -> &str {
        &self.css
    }

    pub fn art_mime(&self) -> &str {
        &self.art_mime
    }

    pub fn art(&self) -> &[u8] {
        &self.art
    }

    pub fn engine(&self) -> &str {
        &self.engine
    }

    pub fn data_bytes(&self) -> usize {
        self.theme_json.len() + self.css.len() + self.art.len()
    }

    #[cfg(test)]
    pub(crate) fn test_fixture(theme_key: &str, css: &str, art_mime: &str, art: Vec<u8>) -> Self {
        Self::test_fixture_with_name(theme_key, "Fixture", css, art_mime, art)
    }

    #[cfg(test)]
    pub(crate) fn test_fixture_with_name(
        theme_key: &str,
        name: &str,
        css: &str,
        art_mime: &str,
        art: Vec<u8>,
    ) -> Self {
        Self {
            theme_key: theme_key.into(),
            theme_json: serde_json::to_string(&json!({
                "key": theme_key,
                "id": "fixture",
                "name": name,
                "version": "1.0.0",
            }))
            .expect("fixture theme is serializable"),
            css: css.into(),
            art_mime: art_mime.into(),
            art,
            engine: "(() => true)()".into(),
        }
    }
}

const BASE_CSS: &str = r#"
html.lumadrobe-theme {
  color-scheme: light dark;
  --lumadrobe-panel: rgba(10, 12, 18, .84);
  --lumadrobe-line: rgba(255, 255, 255, .10);
}
html.lumadrobe-theme body {
  background: #090b10 var(--lumadrobe-art) center / cover fixed no-repeat !important;
}
html.lumadrobe-theme aside {
  background: linear-gradient(180deg, rgba(12, 13, 19, .96), rgba(12, 13, 19, .88)) !important;
  border-color: var(--lumadrobe-line) !important;
  backdrop-filter: blur(18px) saturate(112%) !important;
}
html.lumadrobe-theme main {
  background:
    linear-gradient(115deg, rgba(8, 10, 15, .94) 0%, rgba(8, 10, 15, .70) 42%, rgba(8, 10, 15, .22) 100%),
    var(--lumadrobe-art) center / cover no-repeat !important;
}
html.lumadrobe-theme [role="main"] {
  background: transparent !important;
}
html.lumadrobe-theme .composer-surface-chrome {
  border-color: var(--lumadrobe-line) !important;
  background: var(--lumadrobe-panel) !important;
  backdrop-filter: blur(18px) saturate(116%) !important;
}
"#;

const PAINT_PROBE_CSS: &str = "html.lumadrobe-theme { --lumadrobe-paint-probe: 1 !important; }";

pub fn build_payload(installed: &InstalledSkin) -> AppResult<RendererPayload> {
    let background = read_verified_asset(
        installed,
        Path::new(&installed.background_path),
        IMAGE_LIMIT,
        "背景图",
    )?;
    let mime = image_mime(&background)?;
    let custom_css = if let Some(path) = &installed.css_path {
        let bytes = read_verified_asset(installed, Path::new(path), CSS_LIMIT, "主题 CSS")?;
        std::str::from_utf8(&bytes)
            .map_err(|_| AppError::Runtime("已安装的主题 CSS 不是 UTF-8".into()))?
            .to_owned()
    } else {
        String::new()
    };
    let css = format!("{BASE_CSS}\n{custom_css}\n{PAINT_PROBE_CSS}\n");
    let theme_key = format!(
        "{}:{}@{}",
        installed.source_id, installed.skin_id, installed.version
    );
    let theme = json!({
        "key": theme_key,
        "id": installed.skin_id,
        "name": installed.name,
        "version": installed.version,
    });
    let theme_json = serde_json::to_string(&theme)?;

    let engine_version = RENDERER_ENGINE_VERSION;
    let engine = format!(
        r#"(() => {{
  const ENGINE_KEY = "__LUMADROBE_ENGINE__";
  const ENGINE_VERSION = {engine_version};
  const currentEngine = window[ENGINE_KEY];
  if (currentEngine?.engineVersion === ENGINE_VERSION &&
      typeof currentEngine?.install === "function") {{
    return {{ ready: true, engineVersion: ENGINE_VERSION, reused: true }};
  }}
  const install = (theme, cssText, artBlob) => {{
  const STATE_KEY = "__LUMADROBE_RUNTIME__";
  const RUNTIME_VERSION = 3;
  const STYLE_ID = "lumadrobe-theme-style";
  const CHROME_ID = "codex-dream-skin-chrome";
  const ROOT_CLASSES = ["lumadrobe-theme", "codex-dream-skin"];
  const ART_PROPERTIES = ["--lumadrobe-art", "--dream-art", "--dream-skin-art"];
  const previous = window[STATE_KEY];
  if (previous?.runtimeVersion === RUNTIME_VERSION && previous?.themeKey === theme.key &&
      previous?.ensure && previous?.status) {{
    previous.ensure();
    return {{ ...previous.status?.(), themeKey: theme.key, reused: true }};
  }}
  previous?.cleanup?.();
  window.__CODEX_DREAM_SKIN_STATE__?.cleanup?.();

  if (!(artBlob instanceof Blob) || artBlob.size < 1) throw new Error("LumaDrobe art Blob is invalid");
  const artUrl = URL.createObjectURL(artBlob);
  const touched = new Set();
  const hasChromeCss = cssText.includes(`#${{CHROME_ID}}`);
  const chromeProfile = hasChromeCss && cssText.includes(".dream-skin-brand")
    ? "portal"
    : hasChromeCss && cssText.includes(".dream-brand")
      ? "pink"
      : null;
  const chromeSelector = chromeProfile === "portal" ? ".dream-skin-brand" : ".dream-brand";

  const element = (tag, className, text) => {{
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (text !== undefined) node.textContent = text;
    return node;
  }};

  const appendDots = (parent, className, count) => {{
    const dots = element("div", className);
    for (let index = 0; index < count; index += 1) dots.appendChild(element("i"));
    parent.appendChild(dots);
  }};

  const buildChrome = () => {{
    if (!chromeProfile || !document.body) return null;
    let chrome = document.getElementById(CHROME_ID);
    const reusable = chrome?.dataset.lumadrobeOwned === "true" &&
      chrome.dataset.lumadrobeProfile === chromeProfile &&
      chrome.querySelector(chromeSelector);
    if (!reusable) {{
      chrome?.remove();
      chrome = element("div");
      chrome.id = CHROME_ID;
      chrome.dataset.lumadrobeOwned = "true";
      chrome.dataset.lumadrobeProfile = chromeProfile;
      chrome.setAttribute("aria-hidden", "true");
      if (chromeProfile === "portal") {{
        const brand = element("div", "dream-skin-brand");
        brand.appendChild(element("span", "dream-skin-portal-mark", "◉"));
        const copy = element("span");
        copy.appendChild(element("b", null, theme.name));
        copy.appendChild(element("small", null, "LUMADROBE THEME"));
        brand.appendChild(copy);
        chrome.appendChild(brand);
        const status = element("div", "dream-skin-status");
        status.appendChild(element("i"));
        status.appendChild(element("span", null, "THEME ONLINE"));
        chrome.appendChild(status);
        chrome.appendChild(element("div", "dream-skin-quote", "MAKE SOMETHING WONDERFUL"));
        appendDots(chrome, "dream-skin-particles", 8);
        chrome.appendChild(element("div", "dream-skin-orbit"));
      }} else {{
        const brand = element("div", "dream-brand");
        brand.appendChild(element("span", "dream-note", "♫"));
        const copy = element("span");
        copy.appendChild(element("b", null, theme.name));
        copy.appendChild(element("small", null, "LumaDrobe theme ✦"));
        brand.appendChild(copy);
        chrome.appendChild(brand);
        chrome.appendChild(element("div", "dream-signature", `${{theme.name}} ♡`));
        appendDots(chrome, "dream-sparkles", 6);
        const ribbon = element("div", "dream-ribbon");
        ribbon.appendChild(element("span", null, "♡"));
        ribbon.appendChild(document.createTextNode("🎀"));
        ribbon.appendChild(element("span", null, "✦"));
        chrome.appendChild(ribbon);
        chrome.appendChild(element("div", "dream-polaroid"));
      }}
      document.body.appendChild(chrome);
    }}
    return chrome;
  }};

  const chromeIsReady = () => {{
    if (!chromeProfile) return true;
    const chrome = document.getElementById(CHROME_ID);
    return Boolean(chrome?.isConnected && chrome.dataset.lumadrobeOwned === "true" &&
      chrome.dataset.lumadrobeProfile === chromeProfile && chrome.querySelector(chromeSelector));
  }};

  const readAppearancePreference = () => {{
    const root = document.documentElement;
    const classify = (value) => {{
      const normalized = String(value || "").toLowerCase();
      if (/\b(dark|theme-dark|appearance-dark)\b/.test(normalized)) return "dark";
      if (/\b(light|theme-light|appearance-light)\b/.test(normalized)) return "light";
      if (/\b(system|auto)\b/.test(normalized)) return "system";
      return null;
    }};
    const checked = document.querySelector('input[name="appearance-theme"]:checked');
    const checkedChoice = `${{checked?.getAttribute("aria-label") || ""}} ${{
      checked?.getAttribute("value") || ""
    }}`;
    if (checkedChoice.includes("暗")) return "dark";
    if (checkedChoice.includes("浅")) return "light";
    if (checkedChoice.includes("系统")) return "system";
    const checkedPreference = classify(checkedChoice);
    if (checkedPreference) return checkedPreference;
    const body = document.body;
    const candidates = [
      classify(body?.getAttribute("data-theme")),
      classify(body?.getAttribute("data-appearance")),
      classify(body?.getAttribute("data-color-mode")),
      classify(body?.className),
      classify(root?.getAttribute("data-theme")),
      classify(root?.getAttribute("data-appearance")),
      classify(root?.getAttribute("data-color-mode")),
      classify(root?.className),
    ];
    return candidates.find((value) => value === "dark" || value === "light") ||
      (candidates.includes("system") ? "system" : null);
  }};

  let mediaQuery = null;
  try {{ mediaQuery = matchMedia("(prefers-color-scheme: dark)"); }} catch {{}}
  const readSystemMode = () => mediaQuery?.matches ? "dark" : "light";
  const readInitialComputedMode = () => {{
    const root = document.documentElement;
    try {{
      const scheme = getComputedStyle(root).colorScheme || "";
      if (scheme.includes("dark") && !scheme.includes("light")) return "dark";
      if (scheme.includes("light") && !scheme.includes("dark")) return "light";
    }} catch {{}}
    return null;
  }};

  // The computed style is safe only before adding LumaDrobe classes or CSS.
  // Subsequent refreshes use Codex-owned signals and matchMedia exclusively.
  let previousPreference = readAppearancePreference();
  let previousSystemMode = readSystemMode();
  let shellMode = previousPreference === "dark" || previousPreference === "light"
    ? previousPreference
    : previousPreference === "system"
      ? previousSystemMode
      : readInitialComputedMode() || previousSystemMode;
  const refreshShellMode = () => {{
    const preference = readAppearancePreference();
    const systemMode = readSystemMode();
    if (preference === "dark" || preference === "light") {{
      shellMode = preference;
    }} else if (preference === "system") {{
      shellMode = systemMode;
    }} else if (previousPreference !== null || systemMode !== previousSystemMode) {{
      shellMode = systemMode;
    }}
    previousPreference = preference;
    previousSystemMode = systemMode;
    return shellMode;
  }};

  const ensure = () => {{
    const root = document.documentElement;
    if (!root) return;
    root.classList.add(...ROOT_CLASSES);
    root.dataset.lumadrobeTheme = theme.key;
    root.setAttribute("data-dream-shell", refreshShellMode());
    for (const property of ART_PROPERTIES) {{
      root.style.setProperty(property, `url("${{artUrl}}")`);
    }}
    let style = document.getElementById(STYLE_ID);
    if (!style) {{
      style = document.createElement("style");
      style.id = STYLE_ID;
      (document.head || root).appendChild(style);
    }}
    if (style.textContent !== cssText) style.textContent = cssText;
    const shellMain = document.querySelector("main.main-surface") || document.querySelector("main");
    if (shellMain) {{
      shellMain.classList.add("lumadrobe-surface");
      touched.add(shellMain);
    }}
    const homeIndicator = document.querySelector('[data-testid="home-icon"]');
    const home = homeIndicator?.closest('[role="main"]') ||
      Array.from(document.querySelectorAll('[role="main"]')).find((candidate) =>
        candidate.querySelector('[data-feature="game-source"]')) || null;
    for (const node of document.querySelectorAll(".dream-home, .dream-skin-home")) {{
      if (node !== home) node.classList.remove("dream-home", "dream-skin-home");
    }}
    if (home) {{
      home.classList.add("dream-home", "dream-skin-home");
      touched.add(home);
    }}
    if (shellMain) {{
      shellMain.classList.toggle("dream-home-shell", Boolean(home));
      shellMain.classList.toggle("dream-skin-home-shell", Boolean(home));
    }}
    const chrome = shellMain ? buildChrome() : null;
    if (chrome && shellMain) {{
      const shellBox = shellMain.getBoundingClientRect();
      chrome.style.left = `${{Math.round(shellBox.left)}}px`;
      chrome.style.top = `${{Math.round(shellBox.top)}}px`;
      chrome.style.width = `${{Math.round(shellBox.width)}}px`;
      chrome.style.height = `${{Math.round(shellBox.height)}}px`;
      chrome.classList.toggle("dream-home-shell", Boolean(home));
      chrome.classList.toggle("dream-skin-home-shell", Boolean(home));
      chrome.dataset.dreamShell = shellMode;
    }}
    for (const node of document.querySelectorAll("main.main-surface")) {{
      node.classList.add("lumadrobe-surface");
      touched.add(node);
    }}
  }};

  const status = () => {{
    const root = document.documentElement;
    const style = document.getElementById(STYLE_ID);
    const viewportWidth = Math.max(0, Math.round(window.innerWidth || 0));
    const viewportHeight = Math.max(0, Math.round(window.innerHeight || 0));
    const surfaceReady = Boolean(document.querySelector(
      'main,[role="main"],aside.app-shell-left-panel,.composer-surface-chrome'
    ));
    let paintVerified = false;
    try {{
      paintVerified = getComputedStyle(root)
        .getPropertyValue("--lumadrobe-paint-probe").trim() === "1";
    }} catch {{}}
    const visibilityState = document.visibilityState || "unknown";
    const presentable = visibilityState === "visible" && viewportWidth >= 320 &&
      viewportHeight >= 240 && surfaceReady;
    return {{
      installed: Boolean(window[STATE_KEY]?.runtimeVersion === RUNTIME_VERSION &&
        window[STATE_KEY]?.themeKey === theme.key),
      styleAttached: Boolean(style?.isConnected && style.textContent === cssText),
      rootTagged: Boolean(root && ROOT_CLASSES.every((name) => root.classList.contains(name))),
      artAttached: Boolean(root && ART_PROPERTIES.every((name) => root.style.getPropertyValue(name))),
      chromeAttached: chromeIsReady(),
      paintVerified,
      presentable,
      visibilityState,
      hasFocus: document.hasFocus(),
      viewportWidth,
      viewportHeight,
      surfaceReady,
      navigationEpoch: Number.isFinite(performance.timeOrigin)
        ? Math.round(performance.timeOrigin)
        : null,
    }};
  }};

  const observer = new MutationObserver(() => queueMicrotask(ensure));
  observer.observe(document.documentElement, {{ childList: true, subtree: true }});
  const timer = setInterval(ensure, 5000);
  let mediaHandler = null;
  if (mediaQuery) {{
    mediaHandler = () => queueMicrotask(ensure);
    try {{ mediaQuery.addEventListener("change", mediaHandler); }} catch {{}}
  }}
  const resizeHandler = () => queueMicrotask(ensure);
  window.addEventListener("resize", resizeHandler, {{ passive: true }});
  const cleanup = () => {{
    observer.disconnect();
    clearInterval(timer);
    if (mediaQuery && mediaHandler) {{
      try {{ mediaQuery.removeEventListener("change", mediaHandler); }} catch {{}}
    }}
    window.removeEventListener("resize", resizeHandler);
    document.documentElement?.classList.remove(...ROOT_CLASSES);
    if (document.documentElement?.dataset.lumadrobeTheme === theme.key) {{
      delete document.documentElement.dataset.lumadrobeTheme;
    }}
    document.documentElement?.removeAttribute("data-dream-shell");
    for (const property of ART_PROPERTIES) {{
      document.documentElement?.style.removeProperty(property);
    }}
    document.getElementById(STYLE_ID)?.remove();
    const chrome = document.getElementById(CHROME_ID);
    if (chrome?.dataset.lumadrobeOwned === "true") chrome.remove();
    for (const node of touched) {{
      node.classList?.remove(
        "lumadrobe-surface",
        "dream-home",
        "dream-skin-home",
        "dream-home-shell",
        "dream-skin-home-shell",
      );
    }}
    URL.revokeObjectURL(artUrl);
    if (window[STATE_KEY]?.themeKey === theme.key) delete window[STATE_KEY];
    return true;
  }};
  window[STATE_KEY] = {{
    runtimeVersion: RUNTIME_VERSION,
    themeKey: theme.key,
    ensure,
    status,
    cleanup,
    observer,
    timer,
    mediaQuery,
    mediaHandler,
    resizeHandler,
    artUrl,
  }};
  ensure();
  return {{ ...status(), themeKey: theme.key, reused: false }};
  }};
  window[ENGINE_KEY] = {{ engineVersion: ENGINE_VERSION, install }};
  return {{ ready: true, engineVersion: ENGINE_VERSION, reused: false }};
}})()"#
    );

    Ok(RendererPayload {
        theme_key: theme["key"].as_str().unwrap_or_default().to_string(),
        theme_json,
        css,
        art_mime: mime.to_string(),
        art: background,
        engine,
    })
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
        let root =
            std::env::temp_dir().join(format!("lumadrobe-payload-{}-{unique}", std::process::id()));
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
        assert!(payload.engine().contains("__LUMADROBE_RUNTIME__"));
        assert!(payload.engine().contains("__LUMADROBE_ENGINE__"));
        assert_eq!(RENDERER_ENGINE_VERSION, 2);
        assert!(payload.engine().contains("const ENGINE_VERSION = 2;"));
        assert!(payload.engine().contains("runtimeVersion: RUNTIME_VERSION"));
        assert!(!payload.engine().contains("data:image/png;base64"));
        assert_eq!(payload.theme_key(), "source:night@1.0.0");
        assert!(payload.theme_json().contains("source:night@1.0.0"));
        assert_eq!(payload.art_mime(), "image/png");
        assert_eq!(payload.art(), b"\x89PNG\r\n\x1a\nfixture");
        assert!(payload.engine().len() < 192 * 1024);
        assert!(payload.engine().contains("codex-dream-skin"));
        assert!(payload.engine().contains("--dream-art"));
        assert!(payload.engine().contains("--dream-skin-art"));
        assert!(payload.engine().contains("styleAttached"));
        assert!(payload.engine().contains("rootTagged"));
        assert!(payload.engine().contains("artAttached"));
        assert!(payload.engine().contains("chromeAttached"));
        assert!(payload.engine().contains("paintVerified"));
        assert!(payload.engine().contains("presentable"));
        assert!(payload.engine().contains("--lumadrobe-paint-probe"));
        assert!(payload.engine().contains("codex-dream-skin-chrome"));
        assert!(payload.engine().contains("chromeIsReady"));
        let appearance = payload
            .engine()
            .find("readInitialComputedMode() || previousSystemMode")
            .unwrap();
        let mutation = payload
            .engine()
            .find("root.classList.add(...ROOT_CLASSES)")
            .unwrap();
        assert!(appearance < mutation);
        assert!(payload
            .engine()
            .contains("root.setAttribute(\"data-dream-shell\", refreshShellMode())"));
        assert!(payload
            .engine()
            .contains("mediaQuery.addEventListener(\"change\", mediaHandler)"));
        assert!(payload
            .engine()
            .contains("classify(body?.getAttribute(\"data-theme\"))"));
        assert!(payload
            .engine()
            .contains("candidates.find((value) => value === \"dark\" || value === \"light\")"));
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
