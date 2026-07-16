use crate::error::{AppError, AppResult};
use crate::models::{CatalogSkin, InstalledSkin};
use chrono::Utc;
use reqwest::header::CONTENT_LENGTH;
use reqwest::Client;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const IMAGE_LIMIT: usize = 32 * 1024 * 1024;
const PREVIEW_LIMIT: usize = 8 * 1024 * 1024;
const CSS_LIMIT: usize = 1024 * 1024;

pub async fn install_skin(
    client: &Client,
    data_dir: &Path,
    skin: &CatalogSkin,
) -> AppResult<InstalledSkin> {
    validate_component(&skin.source_id, "订阅源 ID")?;
    validate_component(&skin.manifest.id, "主题 ID")?;
    validate_component(&skin.manifest.version, "主题版本")?;

    let root = data_dir
        .join("themes")
        .join(&skin.source_id)
        .join(&skin.manifest.id);
    let target = root.join(&skin.manifest.version);
    let temporary = root.join(format!(".installing-{}", std::process::id()));
    if temporary.exists() {
        std::fs::remove_dir_all(&temporary)?;
    }
    std::fs::create_dir_all(&temporary)?;

    let result = download_theme_assets(client, skin, &temporary).await;
    let assets = match result {
        Ok(assets) => assets,
        Err(error) => {
            let _ = std::fs::remove_dir_all(&temporary);
            return Err(error);
        }
    };

    std::fs::write(
        temporary.join("skin.json"),
        serde_json::to_vec_pretty(&skin.manifest)?,
    )?;
    if target.exists() {
        std::fs::remove_dir_all(&target)?;
    }
    std::fs::rename(&temporary, &target)?;

    Ok(InstalledSkin {
        source_id: skin.source_id.clone(),
        source_name: skin.source_name.clone(),
        skin_id: skin.manifest.id.clone(),
        name: skin.manifest.name.clone(),
        version: skin.manifest.version.clone(),
        author: skin.manifest.author.clone(),
        installed_at: Utc::now().to_rfc3339(),
        install_dir: target.to_string_lossy().into_owned(),
        background_path: target
            .join(&assets.background_name)
            .to_string_lossy()
            .into_owned(),
        preview_path: assets
            .preview_name
            .map(|name| target.join(name).to_string_lossy().into_owned()),
        css_path: assets
            .css_name
            .map(|name| target.join(name).to_string_lossy().into_owned()),
        asset_hashes: assets.hashes,
        manifest: skin.manifest.clone(),
    })
}

pub fn remove_installed_skin(
    data_dir: &Path,
    source_id: &str,
    skin_id: &str,
    version: &str,
) -> AppResult<()> {
    let target = installed_skin_path(data_dir, source_id, skin_id, version)?;
    if target.exists() {
        std::fs::remove_dir_all(target)?;
    }
    Ok(())
}

pub fn installed_versions_share_directory(
    data_dir: &Path,
    source_id: &str,
    skin_id: &str,
    previous_version: &str,
    current_version: &str,
) -> AppResult<bool> {
    let previous = installed_skin_path(data_dir, source_id, skin_id, previous_version)?;
    let current = installed_skin_path(data_dir, source_id, skin_id, current_version)?;
    match (previous.canonicalize(), current.canonicalize()) {
        (Ok(previous), Ok(current)) => Ok(previous == current),
        _ => Ok(paths_equal_for_platform(&previous, &current)),
    }
}

fn installed_skin_path(
    data_dir: &Path,
    source_id: &str,
    skin_id: &str,
    version: &str,
) -> AppResult<PathBuf> {
    validate_component(source_id, "订阅源 ID")?;
    validate_component(skin_id, "主题 ID")?;
    validate_component(version, "主题版本")?;
    Ok(data_dir
        .join("themes")
        .join(source_id)
        .join(skin_id)
        .join(version))
}

fn paths_equal_for_platform(left: &Path, right: &Path) -> bool {
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    {
        left.to_string_lossy()
            .eq_ignore_ascii_case(&right.to_string_lossy())
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        left == right
    }
}

struct InstalledAssets {
    background_name: String,
    preview_name: Option<String>,
    css_name: Option<String>,
    hashes: BTreeMap<String, String>,
}

async fn download_theme_assets(
    client: &Client,
    skin: &CatalogSkin,
    destination: &Path,
) -> AppResult<InstalledAssets> {
    let mut hashes = BTreeMap::new();
    let background = download(client, &skin.background_url, IMAGE_LIMIT).await?;
    let background_extension = image_extension(&background, "背景图")?;
    let background_name = format!("background.{background_extension}");
    write_asset(destination, &background_name, &background, &mut hashes)?;

    let preview_name = if let Some(url) = &skin.preview_url {
        let preview = download(client, url, PREVIEW_LIMIT).await?;
        let preview_extension = image_extension(&preview, "预览图")?;
        let name = format!("preview.{preview_extension}");
        write_asset(destination, &name, &preview, &mut hashes)?;
        Some(name)
    } else {
        None
    };

    let css_name = if let Some(url) = &skin.css_url {
        let css = download(client, url, CSS_LIMIT).await?;
        let css = std::str::from_utf8(&css)
            .map_err(|_| AppError::UnsafeAsset("CSS 必须使用 UTF-8 编码".into()))?;
        validate_css(css)?;
        write_asset(destination, "theme.css", css.as_bytes(), &mut hashes)?;
        Some("theme.css".into())
    } else {
        None
    };
    Ok(InstalledAssets {
        background_name,
        preview_name,
        css_name,
        hashes,
    })
}

async fn download(client: &Client, url: &str, limit: usize) -> AppResult<Vec<u8>> {
    let mut response = client.get(url).send().await?.error_for_status()?;
    let advertised_length = response
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok());
    if advertised_length.is_some_and(|length| length > limit) {
        return Err(AppError::UnsafeAsset(format!(
            "资源超过 {} MiB",
            limit / 1024 / 1024
        )));
    }
    let mut bytes = Vec::with_capacity(advertised_length.unwrap_or_default().min(limit));
    while let Some(chunk) = response.chunk().await? {
        extend_with_limit(&mut bytes, &chunk, limit)?;
    }
    Ok(bytes)
}

fn extend_with_limit(bytes: &mut Vec<u8>, chunk: &[u8], limit: usize) -> AppResult<()> {
    if bytes.len().saturating_add(chunk.len()) > limit {
        return Err(AppError::UnsafeAsset(format!(
            "资源超过 {} MiB",
            limit / 1024 / 1024
        )));
    }
    bytes.extend_from_slice(chunk);
    Ok(())
}

fn image_extension(bytes: &[u8], label: &str) -> AppResult<&'static str> {
    let png = bytes.starts_with(b"\x89PNG\r\n\x1a\n");
    let jpeg = bytes.starts_with(b"\xff\xd8\xff");
    let webp = bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP";
    if png {
        Ok("png")
    } else if jpeg {
        Ok("jpg")
    } else if webp {
        Ok("webp")
    } else {
        Err(AppError::UnsafeAsset(format!(
            "{label} 仅支持 PNG、JPEG 或 WebP"
        )))
    }
}

fn validate_css(css: &str) -> AppResult<()> {
    let normalized = normalize_css_escapes(&strip_css_comments(css));
    let blocked = [
        "@import",
        "http:",
        "https:",
        "file:",
        "javascript:",
        "-moz-binding",
        "//",
    ];
    if let Some(token) = blocked.iter().find(|token| normalized.contains(**token)) {
        return Err(AppError::UnsafeAsset(format!(
            "CSS 包含被禁止的内容：{token}"
        )));
    }
    Ok(())
}

fn strip_css_comments(css: &str) -> String {
    let mut output = String::with_capacity(css.len());
    let mut chars = css.chars().peekable();
    let mut quote = None;
    while let Some(character) = chars.next() {
        if let Some(active_quote) = quote {
            output.push(character);
            if character == '\\' {
                if let Some(escaped) = chars.next() {
                    output.push(escaped);
                }
            } else if character == active_quote {
                quote = None;
            }
            continue;
        }
        if matches!(character, '\'' | '"') {
            quote = Some(character);
            output.push(character);
            continue;
        }
        if character == '/' && chars.peek() == Some(&'*') {
            chars.next();
            while let Some(comment_character) = chars.next() {
                if comment_character == '*' && chars.peek() == Some(&'/') {
                    chars.next();
                    break;
                }
            }
            output.push(' ');
            continue;
        }
        output.push(character);
    }
    output
}

fn normalize_css_escapes(css: &str) -> String {
    let mut output = String::with_capacity(css.len());
    let mut chars = css.chars().peekable();
    while let Some(character) = chars.next() {
        if character != '\\' {
            output.push(character.to_ascii_lowercase());
            continue;
        }

        let Some(next) = chars.peek().copied() else {
            output.push('\\');
            break;
        };
        if matches!(next, '\n' | '\r' | '\u{000c}') {
            chars.next();
            if next == '\r' && chars.peek() == Some(&'\n') {
                chars.next();
            }
            continue;
        }
        if next.is_ascii_hexdigit() {
            let mut value = 0_u32;
            for _ in 0..6 {
                let Some(digit) = chars.peek().and_then(|value| value.to_digit(16)) else {
                    break;
                };
                value = value * 16 + digit;
                chars.next();
            }
            if chars
                .peek()
                .is_some_and(|value| value.is_ascii_whitespace())
            {
                let whitespace = chars.next();
                if whitespace == Some('\r') && chars.peek() == Some(&'\n') {
                    chars.next();
                }
            }
            output.push(
                char::from_u32(value)
                    .filter(|value| *value != '\0')
                    .unwrap_or('\u{fffd}')
                    .to_ascii_lowercase(),
            );
            continue;
        }
        output.push(chars.next().unwrap_or(next).to_ascii_lowercase());
    }
    output
}

fn write_asset(
    destination: &Path,
    name: &str,
    bytes: &[u8],
    hashes: &mut BTreeMap<String, String>,
) -> AppResult<PathBuf> {
    let path = destination.join(name);
    std::fs::write(&path, bytes)?;
    let digest = Sha256::digest(bytes);
    hashes.insert(name.into(), format!("{digest:x}"));
    Ok(path)
}

fn validate_component(value: &str, label: &str) -> AppResult<()> {
    if value.is_empty()
        || value.starts_with('.')
        || value.ends_with('.')
        || is_windows_reserved_component(value)
        || value.len() > 80
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(AppError::InvalidManifest(format!("{label} 无效")));
    }
    Ok(())
}

fn is_windows_reserved_component(value: &str) -> bool {
    let stem = value.split('.').next().unwrap_or(value);
    if ["con", "prn", "aux", "nul", "conin$", "conout$"]
        .iter()
        .any(|reserved| stem.eq_ignore_ascii_case(reserved))
    {
        return true;
    }
    let bytes = stem.as_bytes();
    bytes.len() == 4
        && (bytes[..3].eq_ignore_ascii_case(b"com") || bytes[..3].eq_ignore_ascii_case(b"lpt"))
        && matches!(bytes[3], b'0'..=b'9')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_local_css_and_rejects_remote_imports() {
        assert!(validate_css("body { color: var(--accent); }").is_ok());
        assert!(validate_css("@import url(https://example.com/theme.css);").is_err());
        assert!(validate_css("a { background: url(https:tracker.example/pixel); }").is_err());
        assert!(validate_css("a { background: url(//tracker.example/pixel); }").is_err());
        assert!(validate_css("a { background: url(  \t\n //tracker.example/pixel); }").is_err());
        assert!(validate_css("a { background: url(  \"//tracker.example/pixel\"); }").is_err());
        assert!(
            validate_css(r#"a { background-image: image-set("//tracker.example/pixel" 1x); }"#)
                .is_err()
        );
        assert!(
            validate_css("a { background: url(/* hidden */ //tracker.example/pixel); }").is_err()
        );
        assert!(validate_css(r"@\69mport url(h\74tps://example.com/theme.css);").is_err());
        assert!(
            validate_css(r"a { background: url(\68\74\74\70\73\3a//example.com/pixel); }").is_err()
        );
        assert!(validate_css(r".group\/home { color: var(--accent); }").is_ok());
        assert!(validate_css("a { background: url(./local-image.png); }").is_ok());
        assert!(validate_css("a { background: url(file:/Users/test/secret.png); }").is_err());
        assert!(validate_css(r#"a{content:"/*";background:url(//tracker.example/p)}"#).is_err());
    }

    #[test]
    fn recognizes_supported_image_signatures() {
        assert_eq!(
            image_extension(b"\x89PNG\r\n\x1a\nrest", "test").unwrap(),
            "png"
        );
        assert!(image_extension(b"<svg onload='alert(1)'>", "test").is_err());
    }

    #[test]
    fn enforces_download_limits_per_chunk() {
        let mut bytes = vec![1, 2, 3];
        assert!(extend_with_limit(&mut bytes, &[4, 5], 5).is_ok());
        assert!(extend_with_limit(&mut bytes, &[6], 5).is_err());
        assert_eq!(bytes, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn rejects_unsafe_storage_components() {
        assert!(validate_component("pink-dream", "主题 ID").is_ok());
        assert!(validate_component(".", "主题 ID").is_err());
        assert!(validate_component("..", "主题 ID").is_err());
        assert!(validate_component(".night", "主题 ID").is_err());
        assert!(validate_component("night.", "主题 ID").is_err());
        assert!(validate_component("CON", "主题 ID").is_err());
        assert!(validate_component("nul.json", "主题 ID").is_err());
        assert!(validate_component("LPT9", "主题 ID").is_err());
        assert!(validate_component("../escape", "主题 ID").is_err());
    }
}
