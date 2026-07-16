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
    validate_component(source_id, "订阅源 ID")?;
    validate_component(skin_id, "主题 ID")?;
    validate_component(version, "主题版本")?;
    let target = data_dir
        .join("themes")
        .join(source_id)
        .join(skin_id)
        .join(version);
    if target.exists() {
        std::fs::remove_dir_all(target)?;
    }
    Ok(())
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
    let response = client.get(url).send().await?.error_for_status()?;
    if response
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok())
        .is_some_and(|length| length > limit)
    {
        return Err(AppError::UnsafeAsset(format!(
            "资源超过 {} MiB",
            limit / 1024 / 1024
        )));
    }
    let bytes = response.bytes().await?;
    if bytes.len() > limit {
        return Err(AppError::UnsafeAsset(format!(
            "资源超过 {} MiB",
            limit / 1024 / 1024
        )));
    }
    Ok(bytes.to_vec())
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
    let normalized = css.to_ascii_lowercase();
    let blocked = [
        "@import",
        "http://",
        "https://",
        "file://",
        "javascript:",
        "url(//",
        "url( //",
        "-moz-binding",
    ];
    if let Some(token) = blocked.iter().find(|token| normalized.contains(**token)) {
        return Err(AppError::UnsafeAsset(format!(
            "CSS 包含被禁止的内容：{token}"
        )));
    }
    Ok(())
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
        || matches!(value, "." | "..")
        || value.len() > 80
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(AppError::InvalidManifest(format!("{label} 无效")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_local_css_and_rejects_remote_imports() {
        assert!(validate_css("body { color: var(--accent); }").is_ok());
        assert!(validate_css("@import url(https://example.com/theme.css);").is_err());
        assert!(validate_css("a { background: url(//tracker.example/pixel); }").is_err());
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
    fn rejects_unsafe_storage_components() {
        assert!(validate_component("pink-dream", "主题 ID").is_ok());
        assert!(validate_component(".", "主题 ID").is_err());
        assert!(validate_component("..", "主题 ID").is_err());
        assert!(validate_component("../escape", "主题 ID").is_err());
    }
}
