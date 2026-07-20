use crate::error::{AppError, AppResult};
use futures_util::{SinkExt, StreamExt};
use reqwest::Client;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::time::{timeout, timeout_at, Instant};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use url::Url;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const COMMAND_TIMEOUT: Duration = Duration::from_secs(5);
const DIRECT_PROBE_TIMEOUT: Duration = Duration::from_millis(750);
const BROWSER_PROBE_TIMEOUT: Duration = Duration::from_secs(3);
const EVALUATE_TIMEOUT: Duration = Duration::from_secs(30);
const DIAGNOSTIC_PROBE_TIMEOUT: Duration = Duration::from_secs(12);
const DIAGNOSTIC_TARGET_LIMIT: usize = 16;
const DIAGNOSTIC_PROBE_LIMIT: usize = 4;

const PROBE_EXPRESSION: &str = r#"(() => {
  const clip = (value, limit = 80) => typeof value === 'string' ? value.slice(0, limit) : null;
  const describe = (node) => ({
    tag: node.tagName?.toLowerCase() ?? null,
    id: clip(node.id),
    role: clip(node.getAttribute?.('role')),
    testId: clip(node.getAttribute?.('data-testid')),
    classes: Array.from(node.classList ?? []).slice(0, 8).map((value) => clip(value, 64)),
  });
  const markers = {
    shell: Boolean(document.querySelector('main.main-surface')),
    sidebar: Boolean(document.querySelector('aside.app-shell-left-panel')),
    composer: Boolean(document.querySelector('.composer-surface-chrome')),
    main: Boolean(document.querySelector('[role="main"]')),
  };
  const security = {
    appProtocol: location.protocol === 'app:',
    documentReady: Boolean(document.documentElement && document.head && document.body) &&
      (document.readyState === 'interactive' || document.readyState === 'complete'),
  };
  const landmarks = Array.from(document.querySelectorAll(
    'main,aside,nav,header,[role="main"],[role="navigation"],[data-testid]'
  )).slice(0, 24).map(describe);
  return {
    // Rust has already verified the signed Codex process, listener owner, browser
    // identity, app:// target URL and loopback WebSocket path. Keep only the
    // renderer-local, navigation-safe checks here; private CSS class names are
    // diagnostic hints and must not become a compatibility gate.
    codex: security.appProtocol && security.documentReady,
    security,
    document: {
      protocol: location.protocol,
      pathname: clip(location.pathname, 160),
      readyState: document.readyState,
      hasDocumentElement: Boolean(document.documentElement),
      hasHead: Boolean(document.head),
      hasBody: Boolean(document.body),
      elementCount: document.getElementsByTagName('*').length,
      root: document.documentElement ? describe(document.documentElement) : null,
      body: document.body ? describe(document.body) : null,
    },
    markers,
    landmarks,
  };
})()"#;

pub const REMOVE_EXPRESSION: &str = r#"(() => {
  const state = window.__LUMADROBE_RUNTIME__;
  if (state?.cleanup) return state.cleanup();
  document.documentElement?.classList.remove('lumadrobe-theme', 'codex-dream-skin');
  for (const property of ['--lumadrobe-art', '--dream-art', '--dream-skin-art']) {
    document.documentElement?.style.removeProperty(property);
  }
  document.documentElement?.removeAttribute('data-dream-shell');
  document.querySelectorAll('.lumadrobe-surface, .dream-home, .dream-skin-home, .dream-home-shell, .dream-skin-home-shell')
    .forEach((node) => node.classList.remove('lumadrobe-surface', 'dream-home', 'dream-skin-home', 'dream-home-shell', 'dream-skin-home-shell'));
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

type PageSocket = WebSocketStream<MaybeTlsStream<TcpStream>>;

struct CdpSession {
    endpoint: String,
    stream: PageSocket,
    next_id: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CdpTransport {
    DirectPage,
    BrowserSession,
}

impl CdpTransport {
    fn as_str(self) -> &'static str {
        match self {
            Self::DirectPage => "directPage",
            Self::BrowserSession => "browserSession",
        }
    }
}

struct CdpEvaluation {
    values: Vec<Value>,
    transport: CdpTransport,
}

#[derive(Debug, Clone, Copy)]
pub struct CdpApplyOutcome {
    pub applied_targets: usize,
    pub direct_targets: usize,
    pub browser_session_targets: usize,
}

impl CdpSession {
    async fn connect_page(target: &CdpTarget, port: u16) -> AppResult<Self> {
        let url = validated_page_url(target, port)?;
        Self::connect_url(url, format!("page/{}", target.id)).await
    }

    async fn connect_browser(
        client: &Client,
        port: u16,
        expected_browser_id: &str,
    ) -> AppResult<Self> {
        let version: VersionResponse = fetch_json(client, port, "/json/version").await?;
        let url = validated_debugger_url(&version.web_socket_debugger_url, port, "browser")?;
        let browser_id = url
            .path()
            .strip_prefix("/devtools/browser/")
            .filter(|value| valid_identifier(value))
            .ok_or_else(|| AppError::Runtime("CDP 浏览器身份路径无效".into()))?;
        if browser_id != expected_browser_id {
            return Err(AppError::Runtime(format!(
                "CDP 浏览器身份已改变：{expected_browser_id} -> {browser_id}"
            )));
        }
        let mut session = Self::connect_url(url, format!("browser/{browser_id}")).await?;
        session
            .command(None, "Browser.getVersion", json!({}), COMMAND_TIMEOUT)
            .await?;
        Ok(session)
    }

    async fn connect_url(url: Url, endpoint: String) -> AppResult<Self> {
        let (stream, _) = timeout(
            CONNECT_TIMEOUT,
            tokio_tungstenite::connect_async(url.as_str()),
        )
        .await
        .map_err(|_| AppError::Runtime("CDP WebSocket 连接超时".into()))?
        .map_err(|error| AppError::Runtime(format!("CDP WebSocket 连接失败：{error}")))?;
        Ok(Self {
            endpoint,
            stream,
            next_id: 1,
        })
    }

    async fn evaluate(
        &mut self,
        session_id: Option<&str>,
        expression: &str,
        wait: Duration,
    ) -> AppResult<Value> {
        let result = self
            .command(
                session_id,
                "Runtime.evaluate",
                json!({
                    "expression": expression,
                    "awaitPromise": true,
                    "returnByValue": true,
                    "userGesture": false,
                }),
                wait,
            )
            .await?;
        if let Some(exception) = result.get("exceptionDetails") {
            return Err(AppError::Runtime(format!("渲染器执行失败：{exception}")));
        }
        Ok(result
            .pointer("/result/value")
            .cloned()
            .unwrap_or(Value::Null))
    }

    async fn command(
        &mut self,
        session_id: Option<&str>,
        method: &str,
        params: Value,
        wait: Duration,
    ) -> AppResult<Value> {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        let request = command_request(id, session_id, method, params).to_string();
        let request_bytes = request.len();
        let deadline = Instant::now() + wait;
        timeout_at(deadline, self.stream.send(Message::Text(request.into())))
            .await
            .map_err(|_| {
                AppError::Runtime(format!(
                    "CDP 命令发送超时：{method}，目标 {}，载荷 {request_bytes} bytes",
                    self.endpoint
                ))
            })?
            .map_err(|error| AppError::Runtime(format!("发送 CDP 命令失败：{error}")))?;

        let mut received_frames = 0_u64;
        let mut event_frames = 0_u64;
        let mut last_event_method = None;
        let response = loop {
            let next = timeout_at(deadline, self.stream.next())
                .await
                .map_err(|_| {
                    let last_event = last_event_method.as_deref().unwrap_or("none");
                    AppError::Runtime(format!(
                        "CDP 命令等待超时：{method}，目标 {}，载荷 {request_bytes} bytes，已接收 {received_frames} 帧（事件 {event_frames}），最近事件 {last_event}",
                        self.endpoint,
                    ))
                })?;
            let message = next
                .ok_or_else(|| AppError::Runtime("CDP WebSocket 已关闭".into()))?
                .map_err(|error| AppError::Runtime(format!("CDP WebSocket 错误：{error}")))?;
            received_frames = received_frames.saturating_add(1);
            let text = match message {
                Message::Text(text) => text,
                Message::Close(_) => {
                    return Err(AppError::Runtime("CDP WebSocket 提前关闭".into()))
                }
                _ => continue,
            };
            let parsed: Value = serde_json::from_str(text.as_str())?;
            let matching_session = match session_id {
                Some(expected) => {
                    parsed.get("sessionId").and_then(Value::as_str) == Some(expected)
                }
                None => true,
            };
            if parsed.get("id").and_then(Value::as_u64) == Some(id) && matching_session {
                break parsed;
            }
            if let Some(event) = parsed.get("method").and_then(Value::as_str) {
                event_frames = event_frames.saturating_add(1);
                last_event_method = Some(event.chars().take(96).collect::<String>());
            }
        };
        if let Some(error) = response.get("error") {
            return Err(AppError::Runtime(format!(
                "CDP 命令被拒绝：{method}：{error}"
            )));
        }
        response
            .get("result")
            .cloned()
            .ok_or_else(|| AppError::Runtime(format!("CDP 响应缺少 result：{method}")))
    }

    async fn close(mut self) {
        let _ = self.stream.close(None).await;
    }
}

fn command_request(id: u64, session_id: Option<&str>, method: &str, params: Value) -> Value {
    let mut request = json!({ "id": id, "method": method, "params": params });
    if let Some(session_id) = session_id {
        request["sessionId"] = Value::String(session_id.into());
    }
    request
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
    let mut targets: Vec<CdpTarget> = targets
        .into_iter()
        .filter(|target| valid_page_target(target, port))
        .collect();
    // The upstream injector probes every app:// page. Prefer the primary page,
    // but retain initialRoute windows as fallback candidates because current
    // Codex builds can expose the responsive renderer under either target.
    targets.sort_by_key(target_priority);
    Ok(targets)
}

/// Collects a bounded, content-free renderer fingerprint for persistent failure logs.
/// It intentionally excludes text, form values, storage, query strings and page titles.
pub async fn diagnostic_snapshot(
    client: &Client,
    port: u16,
    expected_browser_id: Option<&str>,
) -> Value {
    collect_diagnostic_snapshot(client, port, expected_browser_id).await
}

async fn collect_diagnostic_snapshot(
    client: &Client,
    port: u16,
    expected_browser_id: Option<&str>,
) -> Value {
    let identity = match browser_identity(client, port).await {
        Ok(identity) => identity,
        Err(error) => {
            return json!({
                "port": port,
                "stage": "browserIdentity",
                "error": error.to_string(),
            });
        }
    };
    let identity_matches = expected_browser_id
        .map(|expected| expected == identity.id)
        .unwrap_or(true);
    if !identity_matches {
        return json!({
            "port": port,
            "stage": "browserIdentity",
            "browserId": identity.id,
            "identityMatches": false,
        });
    }

    let targets: Vec<CdpTarget> = match fetch_json(client, port, "/json/list").await {
        Ok(targets) => targets,
        Err(error) => {
            return json!({
                "port": port,
                "stage": "targetList",
                "browserId": identity.id,
                "identityMatches": true,
                "error": error.to_string(),
            });
        }
    };
    let total_targets = targets.len();
    let page_targets = targets
        .iter()
        .filter(|target| target.target_type == "page")
        .count();
    let eligible_targets = targets
        .iter()
        .filter(|target| valid_page_target(target, port))
        .count();
    let mut reports = Vec::new();
    let mut probed_targets = 0;
    for target in targets.into_iter().take(DIAGNOSTIC_TARGET_LIMIT) {
        let eligible = valid_page_target(&target, port);
        let mut report = json!({
            "targetId": target.id,
            "targetType": target.target_type,
            "location": safe_target_location(&target.url),
            "eligible": eligible,
        });
        if eligible && probed_targets < DIAGNOSTIC_PROBE_LIMIT {
            probed_targets += 1;
            match timeout(
                DIAGNOSTIC_PROBE_TIMEOUT,
                evaluate_many(
                    client,
                    &target,
                    port,
                    identity.id.as_str(),
                    &[PROBE_EXPRESSION],
                ),
            )
            .await
            {
                Ok(Ok(evaluation)) => {
                    report["probe"] = evaluation
                        .values
                        .into_iter()
                        .next()
                        .unwrap_or(Value::Null);
                    report["probeTransport"] =
                        Value::String(evaluation.transport.as_str().into());
                }
                Ok(Err(error)) => report["probeError"] = Value::String(error.to_string()),
                Err(_) => {
                    report["probeTimeoutMs"] =
                        Value::from(DIAGNOSTIC_PROBE_TIMEOUT.as_millis() as u64)
                }
            }
        } else if eligible {
            report["probeSkipped"] = Value::String("diagnosticProbeLimit".into());
        }
        reports.push(report);
    }
    json!({
        "port": port,
        "stage": "rendererProbe",
        "browserId": identity.id,
        "identityMatches": true,
        "totalTargets": total_targets,
        "pageTargets": page_targets,
        "eligibleTargets": eligible_targets,
        "reportedTargets": reports.len(),
        "probedTargets": probed_targets,
        "targetLimit": DIAGNOSTIC_TARGET_LIMIT,
        "probeLimit": DIAGNOSTIC_PROBE_LIMIT,
        "probeTimeoutMs": DIAGNOSTIC_PROBE_TIMEOUT.as_millis(),
        "bounded": true,
        "targets": reports,
    })
}

pub async fn apply_to_verified_targets(
    client: &Client,
    port: u16,
    browser_id: &str,
    payload: &str,
) -> AppResult<CdpApplyOutcome> {
    let targets = verified_targets(client, port, browser_id).await?;
    let mut applied = 0;
    let mut direct_targets = 0;
    let mut browser_session_targets = 0;
    let mut last_error = None;
    let guarded_payload = guarded_expression(payload);
    for target in targets {
        match evaluate_many(
            client,
            &target,
            port,
            browser_id,
            &[&guarded_payload],
        )
        .await
        {
            Ok(evaluation)
                if probe_is_codex(evaluation.values.first())
                    && theme_install_is_confirmed(evaluation.values.first()) =>
            {
                applied += 1;
                match evaluation.transport {
                    CdpTransport::DirectPage => direct_targets += 1,
                    CdpTransport::BrowserSession => browser_session_targets += 1,
                }
            }
            Ok(evaluation) if probe_is_codex(evaluation.values.first()) => {
                last_error = Some("主题样式未完成挂载".to_string())
            }
            Ok(_) => last_error = Some("页面尚未达到安全可注入状态".to_string()),
            Err(error) => last_error = Some(error.to_string()),
        }
    }
    if applied == 0 {
        return Err(AppError::Runtime(format!(
            "没有找到经过验证的 Codex 渲染页：{}",
            last_error.unwrap_or_else(|| "目标列表为空".into())
        )));
    }
    Ok(CdpApplyOutcome {
        applied_targets: applied,
        direct_targets,
        browser_session_targets,
    })
}

pub async fn ensure_theme_on_verified_targets(
    client: &Client,
    port: u16,
    browser_id: &str,
    theme_key: &str,
    payload: &str,
) -> AppResult<usize> {
    let targets = verified_targets(client, port, browser_id).await?;
    let health = guarded_expression(&theme_health_expression(theme_key)?);
    let mut guarded_payload = None;
    let mut healthy = 0;
    let mut last_error = None;
    for target in targets {
        match evaluate_many(client, &target, port, browser_id, &[&health]).await {
            Ok(evaluation)
                if probe_is_codex(evaluation.values.first())
                    && action_result_is_true(evaluation.values.first()) =>
            {
                healthy += 1;
                continue;
            }
            Ok(evaluation) if probe_is_codex(evaluation.values.first()) => {}
            Ok(_) => {
                last_error = Some("页面尚未达到安全可注入状态".to_string());
                continue;
            }
            Err(error) => {
                last_error = Some(error.to_string());
                continue;
            }
        }

        let guarded_payload = guarded_payload.get_or_insert_with(|| guarded_expression(payload));
        match evaluate_many(
            client,
            &target,
            port,
            browser_id,
            &[guarded_payload.as_str()],
        )
        .await
        {
            Ok(evaluation)
                if probe_is_codex(evaluation.values.first())
                    && theme_install_is_confirmed(evaluation.values.first()) =>
            {
                healthy += 1
            }
            Ok(evaluation) if probe_is_codex(evaluation.values.first()) => {
                last_error = Some("重注入后主题样式未完成挂载".to_string())
            }
            Ok(_) => last_error = Some("页面在重注入前尚未达到安全可注入状态".to_string()),
            Err(error) => last_error = Some(error.to_string()),
        }
    }
    if healthy == 0 {
        return Err(AppError::Runtime(format!(
            "没有找到健康的 Codex 主题渲染页：{}",
            last_error.unwrap_or_else(|| "目标列表为空".into())
        )));
    }
    Ok(healthy)
}

pub async fn count_codex_targets(client: &Client, port: u16, browser_id: &str) -> AppResult<usize> {
    let targets = verified_targets(client, port, browser_id).await?;
    let guarded_probe = guarded_expression("true");
    let mut verified = 0;
    for target in targets {
        let evaluation =
            evaluate_many(client, &target, port, browser_id, &[&guarded_probe]).await?;
        if probe_is_codex(evaluation.values.first()) {
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
        let evaluation =
            evaluate_many(client, &target, port, browser_id, &[&guarded_remove]).await?;
        if probe_is_codex(evaluation.values.first()) {
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
    client: &Client,
    target: &CdpTarget,
    port: u16,
    browser_id: &str,
    expressions: &[&str],
) -> AppResult<CdpEvaluation> {
    let direct_error = match evaluate_many_direct(target, port, expressions).await {
        Ok(values) => {
            return Ok(CdpEvaluation {
                values,
                transport: CdpTransport::DirectPage,
            })
        }
        Err(error) => error,
    };
    match evaluate_many_attached(client, target, port, browser_id, expressions).await {
        Ok(values) => Ok(CdpEvaluation {
            values,
            transport: CdpTransport::BrowserSession,
        }),
        Err(browser_error) => Err(AppError::Runtime(format!(
            "CDP 页面直连与浏览器会话均失败；页面直连：{direct_error}；浏览器会话：{browser_error}"
        ))),
    }
}

async fn evaluate_many_direct(
    target: &CdpTarget,
    port: u16,
    expressions: &[&str],
) -> AppResult<Vec<Value>> {
    let mut session = CdpSession::connect_page(target, port).await?;
    let probe = session
        .evaluate(None, PROBE_EXPRESSION, DIRECT_PROBE_TIMEOUT)
        .await?;
    if !probe_is_codex(Some(&probe)) {
        return Err(AppError::Runtime(
            "页面直连探针未确认可注入的 Codex 文档".into(),
        ));
    }
    let mut values = Vec::with_capacity(expressions.len());
    for expression in expressions {
        values.push(session.evaluate(None, expression, EVALUATE_TIMEOUT).await?);
    }
    session.close().await;
    Ok(values)
}

async fn evaluate_many_attached(
    client: &Client,
    target: &CdpTarget,
    port: u16,
    browser_id: &str,
    expressions: &[&str],
) -> AppResult<Vec<Value>> {
    let mut browser = CdpSession::connect_browser(client, port, browser_id).await?;
    let attached = browser
        .command(
            None,
            "Target.attachToTarget",
            json!({ "targetId": target.id.as_str(), "flatten": true }),
            COMMAND_TIMEOUT,
        )
        .await?;
    let session_id = attached
        .get("sessionId")
        .and_then(Value::as_str)
        .filter(|value| valid_identifier(value))
        .ok_or_else(|| AppError::Runtime("CDP 浏览器附加会话身份无效".into()))?
        .to_string();
    let result = async {
        let probe = browser
            .evaluate(
                Some(session_id.as_str()),
                PROBE_EXPRESSION,
                BROWSER_PROBE_TIMEOUT,
            )
            .await?;
        if !probe_is_codex(Some(&probe)) {
            return Err(AppError::Runtime(
                "浏览器附加探针未确认可注入的 Codex 文档".into(),
            ));
        }
        let mut values = Vec::with_capacity(expressions.len());
        for expression in expressions {
            values.push(
                browser
                    .evaluate(
                        Some(session_id.as_str()),
                        expression,
                        EVALUATE_TIMEOUT,
                    )
                    .await?,
            );
        }
        Ok(values)
    }
    .await;
    let _ = browser
        .command(
            None,
            "Target.detachFromTarget",
            json!({ "sessionId": session_id }),
            COMMAND_TIMEOUT,
        )
        .await;
    browser.close().await;
    result
}

fn probe_is_codex(value: Option<&Value>) -> bool {
    value
        .and_then(|value| value.get("codex"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn action_result_is_true(value: Option<&Value>) -> bool {
    value
        .and_then(|value| value.get("result"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn theme_install_is_confirmed(value: Option<&Value>) -> bool {
    let result = value.and_then(|value| value.get("result"));
    [
        "installed",
        "styleAttached",
        "rootTagged",
        "artAttached",
        "chromeAttached",
    ]
    .into_iter()
    .all(|field| {
        result
            .and_then(|value| value.get(field))
            .and_then(Value::as_bool)
            == Some(true)
    })
}

fn theme_health_expression(theme_key: &str) -> AppResult<String> {
    let key = serde_json::to_string(theme_key)?;
    Ok(format!(
        r#"(() => {{
  const state = window.__LUMADROBE_RUNTIME__;
  if (state?.themeKey !== {key} || typeof state?.ensure !== "function" ||
      typeof state?.status !== "function") return false;
  try {{
    state.ensure();
    const status = state.status();
    return Boolean(status?.installed && status?.styleAttached &&
      status?.rootTagged && status?.artAttached && status?.chromeAttached);
  }} catch {{ return false; }}
}})()"#
    ))
}

fn guarded_expression(action: &str) -> String {
    format!(
        "(() => {{ const checked = ({PROBE_EXPRESSION}); if (!checked.codex) return checked; return {{ codex: true, result: ({action}) }}; }})()"
    )
}

fn valid_page_target(target: &CdpTarget, port: u16) -> bool {
    let Ok(document_url) = Url::parse(&target.url) else {
        return false;
    };
    target.target_type == "page"
        && target.url.starts_with("app://")
        && document_url.scheme() == "app"
        && valid_identifier(&target.id)
        && validated_page_url(target, port).is_ok()
}

fn target_priority(target: &CdpTarget) -> u8 {
    Url::parse(&target.url)
        .ok()
        .map(|url| {
            u8::from(
                url.query_pairs()
                    .any(|(key, _)| key.eq_ignore_ascii_case("initialRoute")),
            )
        })
        .unwrap_or(1)
}

fn safe_target_location(raw: &str) -> Value {
    match Url::parse(raw) {
        Ok(url) => json!({
            "scheme": url.scheme(),
            "host": url.host_str().map(|value| value.chars().take(80).collect::<String>()),
            "path": url.path().chars().take(160).collect::<String>(),
            "initialRoute": url.query_pairs().any(|(key, _)| key.eq_ignore_ascii_case("initialRoute")),
        }),
        Err(_) => json!({ "scheme": "invalid" }),
    }
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
        item.url = "app:settings".into();
        assert!(!valid_page_target(&item, 9341));
        item.url = "app:index.html".into();
        assert!(!valid_page_target(&item, 9341));
        item.url = "app://codex/home".into();
        item.id = "../browser".into();
        assert!(!valid_page_target(&item, 9341));
    }

    #[test]
    fn keeps_initial_route_targets_as_low_priority_fallbacks() {
        let mut item = target("ws://127.0.0.1:9341/devtools/page/page-1");
        item.url = "app://-/index.html?initialRoute=settings".into();
        assert!(valid_page_target(&item, 9341));
        assert_eq!(target_priority(&item), 1);

        item.url = "app://-/index.html?INITIALROUTE=settings".into();
        assert!(valid_page_target(&item, 9341));
        assert_eq!(target_priority(&item), 1);

        item.url = "app://-/index.html?route=settings".into();
        assert!(valid_page_target(&item, 9341));
        assert_eq!(target_priority(&item), 0);
    }

    #[test]
    fn flattened_commands_carry_the_attached_session_id() {
        let request = command_request(
            7,
            Some("session-1"),
            "Runtime.evaluate",
            json!({ "expression": "true" }),
        );
        assert_eq!(request["id"], 7);
        assert_eq!(request["sessionId"], "session-1");
        assert_eq!(request["method"], "Runtime.evaluate");
        assert_eq!(request["params"]["expression"], "true");

        let browser_request = command_request(8, None, "Browser.getVersion", json!({}));
        assert!(browser_request.get("sessionId").is_none());
    }

    #[test]
    fn guarded_actions_run_only_after_the_codex_probe() {
        let guarded = guarded_expression("window.__testAction = true");
        let probe = guarded.find("if (!checked.codex)").unwrap();
        let action = guarded.find("window.__testAction").unwrap();
        assert!(probe < action);
    }

    #[test]
    fn watcher_health_probe_is_small_and_theme_specific() {
        let expression = theme_health_expression("source:night@1.0.0").unwrap();
        assert!(expression.contains("source:night@1.0.0"));
        assert!(expression.contains("state.ensure()"));
        assert!(!expression.contains("data:image"));
    }

    #[test]
    fn diagnostic_probe_is_bounded_and_content_free() {
        assert!(PROBE_EXPRESSION.contains("slice(0, 24)"));
        assert!(PROBE_EXPRESSION.contains("elementCount"));
        assert!(PROBE_EXPRESSION.contains("security.appProtocol && security.documentReady"));
        assert_eq!(DIAGNOSTIC_PROBE_TIMEOUT, Duration::from_secs(12));
        assert_eq!(DIAGNOSTIC_TARGET_LIMIT, 16);
        assert_eq!(DIAGNOSTIC_PROBE_LIMIT, 4);
        for forbidden in [
            "innerText",
            "textContent",
            "innerHTML",
            "outerHTML",
            "localStorage",
            "sessionStorage",
            "document.title",
            ".value",
        ] {
            assert!(
                !PROBE_EXPRESSION.contains(forbidden),
                "diagnostic probe must not capture {forbidden}"
            );
        }
    }

    #[test]
    fn theme_install_requires_observable_renderer_state() {
        let value = json!({
            "result": {
                "installed": true,
                "styleAttached": true,
                "rootTagged": true,
                "artAttached": true,
                "chromeAttached": true,
            }
        });
        assert!(theme_install_is_confirmed(Some(&value)));
        let missing_style = json!({
            "result": {
                "installed": true,
                "styleAttached": false,
                "rootTagged": true,
                "artAttached": true,
                "chromeAttached": true,
            }
        });
        assert!(!theme_install_is_confirmed(Some(&missing_style)));
        let missing_chrome = json!({
            "result": {
                "installed": true,
                "styleAttached": true,
                "rootTagged": true,
                "artAttached": true,
                "chromeAttached": false,
            }
        });
        assert!(!theme_install_is_confirmed(Some(&missing_chrome)));
    }

    #[test]
    fn safe_target_location_drops_query_and_fragment() {
        let location = safe_target_location("app://codex/home?secret=value#private");
        assert_eq!(location["scheme"], "app");
        assert_eq!(location["host"], "codex");
        assert_eq!(location["path"], "/home");
        assert_eq!(location["initialRoute"], false);
        assert!(location.get("query").is_none());
        assert!(location.get("fragment").is_none());
    }
}
