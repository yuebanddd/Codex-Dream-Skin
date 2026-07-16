use crate::error::{AppError, AppResult};
use futures_util::{SinkExt, StreamExt};
use reqwest::Client;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;
use url::Url;

const PROBE_EXPRESSION: &str = r#"(() => {
  const markers = {
    shell: Boolean(document.querySelector('main.main-surface')),
    sidebar: Boolean(document.querySelector('aside.app-shell-left-panel')),
    composer: Boolean(document.querySelector('.composer-surface-chrome')),
    main: Boolean(document.querySelector('[role="main"]')),
  };
  return {
    codex: location.protocol === 'app:' && markers.shell && markers.sidebar &&
      (markers.composer || markers.main),
    markers,
  };
})()"#;

pub const REMOVE_EXPRESSION: &str = r#"(() => {
  const state = window.__LUMADROBE_RUNTIME__;
  if (state?.cleanup) return state.cleanup();
  document.documentElement?.classList.remove('lumadrobe-theme');
  document.documentElement?.style.removeProperty('--lumadrobe-art');
  document.getElementById('lumadrobe-theme-style')?.remove();
  delete window.__LUMADROBE_RUNTIME__;
  return true;
})()"#;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CdpTarget {
    pub id: String,
    #[serde(rename = "type")]
    pub target_type: String,
    pub url: String,
    pub web_socket_debugger_url: String,
}

#[derive(Debug, Clone)]
pub struct BrowserIdentity {
    pub id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VersionResponse {
    web_socket_debugger_url: String,
}

pub fn build_client() -> AppResult<Client> {
    Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(3))
        .build()
        .map_err(AppError::Network)
}

pub async fn browser_identity(client: &Client, port: u16) -> AppResult<BrowserIdentity> {
    let version: VersionResponse = fetch_json(client, port, "/json/version").await?;
    let url = validated_debugger_url(&version.web_socket_debugger_url, port, "browser")?;
    let id = url
        .path()
        .strip_prefix("/devtools/browser/")
        .filter(|value| valid_identifier(value))
        .ok_or_else(|| AppError::Runtime("CDP 浏览器身份路径无效".into()))?;
    Ok(BrowserIdentity { id: id.into() })
}

pub async fn verified_targets(
    client: &Client,
    port: u16,
    expected_browser_id: &str,
) -> AppResult<Vec<CdpTarget>> {
    let identity = browser_identity(client, port).await?;
    if identity.id != expected_browser_id {
        return Err(AppError::Runtime(format!(
            "CDP 浏览器身份已改变：{} -> {}",
            expected_browser_id, identity.id
        )));
    }
    let targets: Vec<CdpTarget> = fetch_json(client, port, "/json/list").await?;
    Ok(targets
        .into_iter()
        .filter(|target| valid_page_target(target, port))
        .collect())
}

pub async fn apply_to_verified_targets(
    client: &Client,
    port: u16,
    browser_id: &str,
    payload: &str,
) -> AppResult<usize> {
    let targets = verified_targets(client, port, browser_id).await?;
    let mut applied = 0;
    let mut last_error = None;
    let guarded_payload = guarded_expression(payload);
    for target in targets {
        match evaluate_many(&target, port, &[&guarded_payload]).await {
            Ok(values) if probe_is_codex(values.first()) => applied += 1,
            Ok(_) => last_error = Some("页面未通过 Codex DOM 标记校验".to_string()),
            Err(error) => last_error = Some(error.to_string()),
        }
    }
    if applied == 0 {
        return Err(AppError::Runtime(format!(
            "没有找到经过验证的 Codex 渲染页：{}",
            last_error.unwrap_or_else(|| "目标列表为空".into())
        )));
    }
    Ok(applied)
}

pub async fn count_codex_targets(client: &Client, port: u16, browser_id: &str) -> AppResult<usize> {
    let targets = verified_targets(client, port, browser_id).await?;
    let guarded_probe = guarded_expression("true");
    let mut verified = 0;
    for target in targets {
        let values = evaluate_many(&target, port, &[&guarded_probe]).await?;
        if probe_is_codex(values.first()) {
            verified += 1;
        }
    }
    Ok(verified)
}

pub async fn remove_from_verified_targets(
    client: &Client,
    port: u16,
    browser_id: &str,
) -> AppResult<usize> {
    let targets = verified_targets(client, port, browser_id).await?;
    let mut removed = 0;
    let guarded_remove = guarded_expression(REMOVE_EXPRESSION);
    for target in targets {
        let values = evaluate_many(&target, port, &[&guarded_remove]).await?;
        if probe_is_codex(values.first()) {
            removed += 1;
        }
    }
    Ok(removed)
}

async fn fetch_json<T: DeserializeOwned>(
    client: &Client,
    port: u16,
    resource: &str,
) -> AppResult<T> {
    let response = client
        .get(format!("http://127.0.0.1:{port}{resource}"))
        .send()
        .await?
        .error_for_status()?;
    response.json().await.map_err(AppError::Network)
}

async fn evaluate_many(
    target: &CdpTarget,
    port: u16,
    expressions: &[&str],
) -> AppResult<Vec<Value>> {
    let url = validated_page_url(target, port)?;
    let (mut stream, _) = timeout(
        Duration::from_secs(5),
        tokio_tungstenite::connect_async(url.as_str()),
    )
    .await
    .map_err(|_| AppError::Runtime("CDP WebSocket 连接超时".into()))?
    .map_err(|error| AppError::Runtime(format!("CDP WebSocket 连接失败：{error}")))?;

    let mut values = Vec::with_capacity(expressions.len());
    for (index, expression) in expressions.iter().enumerate() {
        let id = (index + 1) as u64;
        let request = json!({
            "id": id,
            "method": "Runtime.evaluate",
            "params": {
                "expression": expression,
                "awaitPromise": true,
                "returnByValue": true,
                "userGesture": false,
            }
        });
        stream
            .send(Message::Text(request.to_string().into()))
            .await
            .map_err(|error| AppError::Runtime(format!("发送 CDP 命令失败：{error}")))?;
        let response = loop {
            let next = timeout(Duration::from_secs(10), stream.next())
                .await
                .map_err(|_| AppError::Runtime("CDP 命令等待超时".into()))?;
            let message = next
                .ok_or_else(|| AppError::Runtime("CDP WebSocket 已关闭".into()))?
                .map_err(|error| AppError::Runtime(format!("CDP WebSocket 错误：{error}")))?;
            let text = match message {
                Message::Text(text) => text,
                Message::Close(_) => {
                    return Err(AppError::Runtime("CDP WebSocket 提前关闭".into()))
                }
                _ => continue,
            };
            let parsed: Value = serde_json::from_str(text.as_str())?;
            if parsed.get("id").and_then(Value::as_u64) == Some(id) {
                break parsed;
            }
        };
        if let Some(error) = response.get("error") {
            return Err(AppError::Runtime(format!("CDP 命令被拒绝：{error}")));
        }
        let result = response
            .get("result")
            .ok_or_else(|| AppError::Runtime("CDP 响应缺少 result".into()))?;
        if let Some(exception) = result.get("exceptionDetails") {
            return Err(AppError::Runtime(format!("渲染器执行失败：{exception}")));
        }
        values.push(
            result
                .pointer("/result/value")
                .cloned()
                .unwrap_or(Value::Null),
        );
    }
    let _ = stream.close(None).await;
    Ok(values)
}

fn probe_is_codex(value: Option<&Value>) -> bool {
    value
        .and_then(|value| value.get("codex"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn guarded_expression(action: &str) -> String {
    format!(
        "(() => {{ const checked = ({PROBE_EXPRESSION}); if (!checked.codex) return checked; return {{ codex: true, result: ({action}) }}; }})()"
    )
}

fn valid_page_target(target: &CdpTarget, port: u16) -> bool {
    target.target_type == "page"
        && target.url.starts_with("app://")
        && valid_identifier(&target.id)
        && validated_page_url(target, port).is_ok()
}

fn validated_page_url(target: &CdpTarget, port: u16) -> AppResult<Url> {
    let url = validated_debugger_url(&target.web_socket_debugger_url, port, "page")?;
    if url.path() != format!("/devtools/page/{}", target.id) {
        return Err(AppError::Runtime(
            "CDP 页面身份与 WebSocket 路径不一致".into(),
        ));
    }
    Ok(url)
}

fn validated_debugger_url(value: &str, port: u16, kind: &str) -> AppResult<Url> {
    let url = Url::parse(value)
        .map_err(|error| AppError::Runtime(format!("CDP WebSocket URL 无效：{error}")))?;
    let loopback = matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "::1"));
    let prefix = format!("/devtools/{kind}/");
    let id = url.path().strip_prefix(&prefix).unwrap_or_default();
    if url.scheme() != "ws"
        || !loopback
        || url.port() != Some(port)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !valid_identifier(id)
    {
        return Err(AppError::Runtime(
            "拒绝连接不符合回环端点规则的 CDP WebSocket".into(),
        ));
    }
    Ok(url)
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 200
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(url: &str) -> CdpTarget {
        CdpTarget {
            id: "page-1".into(),
            target_type: "page".into(),
            url: "app://codex/home".into(),
            web_socket_debugger_url: url.into(),
        }
    }

    #[test]
    fn accepts_only_exact_loopback_page_urls() {
        assert!(valid_page_target(
            &target("ws://127.0.0.1:9341/devtools/page/page-1"),
            9341
        ));
        assert!(!valid_page_target(
            &target("ws://example.com:9341/devtools/page/page-1"),
            9341
        ));
        assert!(!valid_page_target(
            &target("ws://127.0.0.1:9342/devtools/page/page-1"),
            9341
        ));
        assert!(!valid_page_target(
            &target("ws://127.0.0.1:9341/devtools/page/other"),
            9341
        ));
        assert!(!valid_page_target(
            &target("ws://127.0.0.1:9341/devtools/page/page-1?token=x"),
            9341
        ));
    }

    #[test]
    fn rejects_non_app_targets_and_unsafe_ids() {
        let mut item = target("ws://127.0.0.1:9341/devtools/page/page-1");
        item.url = "https://example.com".into();
        assert!(!valid_page_target(&item, 9341));
        item.url = "app://codex/home".into();
        item.id = "../browser".into();
        assert!(!valid_page_target(&item, 9341));
    }

    #[test]
    fn guarded_actions_run_only_after_the_codex_probe() {
        let guarded = guarded_expression("window.__testAction = true");
        let probe = guarded.find("if (!checked.codex)").unwrap();
        let action = guarded.find("window.__testAction").unwrap();
        assert!(probe < action);
    }
}
