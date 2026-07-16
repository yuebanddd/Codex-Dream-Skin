use crate::error::{AppError, AppResult};
use crate::models::{CatalogSkin, GithubRepository, SkinManifest, SourceManifest, SourceRecord};
use chrono::Utc;
use reqwest::Client;
use url::Url;

const SOURCE_MANIFEST: &str = "dreamskin-source.json";

#[derive(Debug, Clone)]
struct GithubSource {
    repository_url: String,
    owner: String,
    repository: String,
    requested_ref: Option<String>,
}

pub async fn fetch_source(client: &Client, repository_url: &str) -> AppResult<SourceRecord> {
    let source = parse_github_source(repository_url)?;
    let ref_name = match source.requested_ref.clone() {
        Some(value) => value,
        None => fetch_default_branch(client, &source).await?,
    };
    validate_ref(&ref_name)?;

    let manifest_url = raw_url(&source, &ref_name, SOURCE_MANIFEST)?;
    let manifest: SourceManifest = fetch_json(client, &manifest_url).await?;
    validate_source_manifest(&manifest)?;

    let mut skins = Vec::with_capacity(manifest.skins.len());
    for entry in &manifest.skins {
        validate_repo_path(&entry.manifest)?;
        let skin_url = raw_url(&source, &ref_name, &entry.manifest)?;
        let skin: SkinManifest = fetch_json(client, &skin_url).await?;
        validate_skin_manifest(&entry.id, &skin)?;

        let preview_url = skin
            .preview
            .as_deref()
            .map(|path| raw_url(&source, &ref_name, path))
            .transpose()?;
        let background_url = raw_url(&source, &ref_name, &skin.background)?;
        let css_url = skin
            .css
            .as_deref()
            .map(|path| raw_url(&source, &ref_name, path))
            .transpose()?;

        skins.push(CatalogSkin {
            source_id: manifest.id.clone(),
            source_name: manifest.name.clone(),
            manifest_path: entry.manifest.clone(),
            preview_url,
            background_url,
            css_url,
            manifest: skin,
        });
    }

    Ok(SourceRecord {
        id: manifest.id,
        repository_url: source.repository_url,
        owner: source.owner,
        repository: source.repository,
        ref_name,
        name: manifest.name,
        author: manifest.author,
        description: manifest.description,
        refreshed_at: Utc::now().to_rfc3339(),
        skins,
    })
}

fn parse_github_source(value: &str) -> AppResult<GithubSource> {
    let url = Url::parse(value).map_err(|error| AppError::InvalidSource(error.to_string()))?;
    if url.scheme() != "https" || url.host_str() != Some("github.com") {
        return Err(AppError::InvalidSource(
            "第一版只支持 https://github.com/owner/repository".into(),
        ));
    }
    let segments: Vec<_> = url
        .path_segments()
        .map(|parts| parts.filter(|part| !part.is_empty()).collect())
        .unwrap_or_default();
    if segments.len() != 2 {
        return Err(AppError::InvalidSource(
            "地址必须指向 GitHub 仓库根目录".into(),
        ));
    }
    let owner = segments[0].to_string();
    let repository = segments[1].trim_end_matches(".git").to_string();
    validate_component(&owner, "owner")?;
    validate_component(&repository, "repository")?;
    let requested_ref = url
        .query_pairs()
        .find(|(key, _)| key == "ref")
        .map(|(_, value)| value.into_owned());

    Ok(GithubSource {
        repository_url: format!("https://github.com/{owner}/{repository}"),
        owner,
        repository,
        requested_ref,
    })
}

async fn fetch_default_branch(client: &Client, source: &GithubSource) -> AppResult<String> {
    let url = format!(
        "https://api.github.com/repos/{}/{}",
        source.owner, source.repository
    );
    let repository: GithubRepository = fetch_json(client, &url).await?;
    Ok(repository.default_branch)
}

async fn fetch_json<T: serde::de::DeserializeOwned>(client: &Client, url: &str) -> AppResult<T> {
    Ok(client
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .json::<T>()
        .await?)
}

fn raw_url(source: &GithubSource, ref_name: &str, path: &str) -> AppResult<String> {
    validate_ref(ref_name)?;
    validate_repo_path(path)?;
    Ok(format!(
        "https://raw.githubusercontent.com/{}/{}/{}/{}",
        source.owner, source.repository, ref_name, path
    ))
}

fn validate_source_manifest(manifest: &SourceManifest) -> AppResult<()> {
    if manifest.schema_version != 1 {
        return Err(AppError::InvalidManifest(format!(
            "不支持的订阅源 schemaVersion {}",
            manifest.schema_version
        )));
    }
    validate_id(&manifest.id)?;
    if manifest.name.trim().is_empty() || manifest.author.trim().is_empty() {
        return Err(AppError::InvalidManifest(
            "订阅源 name 和 author 不能为空".into(),
        ));
    }
    if manifest.skins.len() > 500 {
        return Err(AppError::InvalidManifest(
            "单个订阅源最多包含 500 套皮肤".into(),
        ));
    }
    Ok(())
}

fn validate_skin_manifest(expected_id: &str, manifest: &SkinManifest) -> AppResult<()> {
    if manifest.schema_version != 1 {
        return Err(AppError::InvalidManifest(format!(
            "皮肤 {expected_id} 使用了不支持的 schemaVersion"
        )));
    }
    validate_id(&manifest.id)?;
    if manifest.id != expected_id {
        return Err(AppError::InvalidManifest(format!(
            "皮肤索引 ID {expected_id} 与清单 ID {} 不一致",
            manifest.id
        )));
    }
    validate_repo_path(&manifest.background)?;
    if let Some(path) = &manifest.preview {
        validate_repo_path(path)?;
    }
    if let Some(path) = &manifest.css {
        validate_repo_path(path)?;
    }
    if manifest.name.trim().is_empty()
        || manifest.author.trim().is_empty()
        || manifest.version.trim().is_empty()
    {
        return Err(AppError::InvalidManifest(format!(
            "皮肤 {expected_id} 的 name、author 和 version 不能为空"
        )));
    }
    Ok(())
}

fn validate_id(value: &str) -> AppResult<()> {
    if value.is_empty()
        || value.len() > 80
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(AppError::InvalidManifest(format!("无效 ID：{value}")));
    }
    Ok(())
}

fn validate_component(value: &str, label: &str) -> AppResult<()> {
    if value.is_empty()
        || value.len() > 100
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(AppError::InvalidSource(format!("无效的 {label}")));
    }
    Ok(())
}

fn validate_ref(value: &str) -> AppResult<()> {
    if value.is_empty()
        || value.len() > 200
        || value.contains("..")
        || value.starts_with('/')
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'/'))
    {
        return Err(AppError::InvalidSource("无效的 Git ref".into()));
    }
    Ok(())
}

fn validate_repo_path(value: &str) -> AppResult<()> {
    if value.is_empty()
        || value.len() > 500
        || value.starts_with('/')
        || value
            .split('/')
            .any(|segment| segment.is_empty() || segment == "..")
        || value.contains('\\')
    {
        return Err(AppError::InvalidManifest(format!(
            "资源路径必须位于仓库内：{value}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_repository_and_ref() {
        let source =
            parse_github_source("https://github.com/yuebanddd/Codex-Dream-Skin?ref=release")
                .unwrap();
        assert_eq!(source.owner, "yuebanddd");
        assert_eq!(source.repository, "Codex-Dream-Skin");
        assert_eq!(source.requested_ref.as_deref(), Some("release"));
    }

    #[test]
    fn rejects_nested_github_pages() {
        assert!(parse_github_source("https://github.com/a/b/tree/main").is_err());
    }

    #[test]
    fn rejects_parent_resource_paths() {
        assert!(validate_repo_path("../secret.png").is_err());
    }
}
