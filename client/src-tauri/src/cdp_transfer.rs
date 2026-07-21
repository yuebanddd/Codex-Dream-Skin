use crate::renderer_payload::RendererPayload;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const DATA_CHUNK_BYTES: usize = 72 * 1024;
const TRANSFER_KEY_PREFIX: &str = "__LUMADROBE_THEME_TRANSFER__";
const ENGINE_KEY: &str = "__LUMADROBE_ENGINE__";
const ENGINE_VERSION: u32 = 1;
const TRANSFER_TTL_MS: u64 = 120_000;
static TRANSFER_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Debug)]
pub(crate) struct ThemeTransferPlan {
    storage_key: String,
    token: String,
    theme_sha256: String,
    theme_bytes: usize,
    theme_chunks: Vec<String>,
    css_sha256: String,
    css_bytes: usize,
    css_chunks: Vec<String>,
    art_mime: String,
    art_sha256: String,
    art_bytes: usize,
    art_chunks: Vec<String>,
}

impl ThemeTransferPlan {
    pub(crate) fn new(payload: &RendererPayload) -> Self {
        let theme_sha256 = format!("{:x}", Sha256::digest(payload.theme_json().as_bytes()));
        let css_sha256 = format!("{:x}", Sha256::digest(payload.css().as_bytes()));
        let art_sha256 = format!("{:x}", Sha256::digest(payload.art()));
        let attempt_id = next_attempt_id();
        let theme_chunks = payload
            .theme_json()
            .as_bytes()
            .chunks(DATA_CHUNK_BYTES)
            .map(|chunk| STANDARD.encode(chunk))
            .collect::<Vec<_>>();
        let css_chunks = payload
            .css()
            .as_bytes()
            .chunks(DATA_CHUNK_BYTES)
            .map(|chunk| STANDARD.encode(chunk))
            .collect::<Vec<_>>();
        let art_chunks = payload
            .art()
            .chunks(DATA_CHUNK_BYTES)
            .map(|chunk| STANDARD.encode(chunk))
            .collect::<Vec<_>>();
        Self {
            storage_key: format!("{TRANSFER_KEY_PREFIX}:{attempt_id}"),
            token: format!(
                "{}-{}-{attempt_id}",
                &art_sha256[..20],
                payload.data_bytes()
            ),
            theme_sha256,
            theme_bytes: payload.theme_json().len(),
            theme_chunks,
            css_sha256,
            css_bytes: payload.css().len(),
            css_chunks,
            art_mime: payload.art_mime().to_string(),
            art_sha256,
            art_bytes: payload.art().len(),
            art_chunks,
        }
    }

    pub(crate) fn token(&self) -> &str {
        &self.token
    }

    pub(crate) fn css_sha256(&self) -> &str {
        &self.css_sha256
    }

    pub(crate) fn theme_sha256(&self) -> &str {
        &self.theme_sha256
    }

    pub(crate) fn theme_bytes(&self) -> usize {
        self.theme_bytes
    }

    pub(crate) fn css_bytes(&self) -> usize {
        self.css_bytes
    }

    pub(crate) fn art_sha256(&self) -> &str {
        &self.art_sha256
    }

    pub(crate) fn art_bytes(&self) -> usize {
        self.art_bytes
    }

    pub(crate) fn css_chunk_count(&self) -> usize {
        self.css_chunks.len()
    }

    pub(crate) fn theme_chunk_count(&self) -> usize {
        self.theme_chunks.len()
    }

    pub(crate) fn art_chunk_count(&self) -> usize {
        self.art_chunks.len()
    }

    pub(crate) fn css_received_bytes_after(&self, index: usize) -> usize {
        self.css_bytes
            .min((index + 1).saturating_mul(DATA_CHUNK_BYTES))
    }

    pub(crate) fn theme_received_bytes_after(&self, index: usize) -> usize {
        self.theme_bytes
            .min((index + 1).saturating_mul(DATA_CHUNK_BYTES))
    }

    pub(crate) fn art_received_bytes_after(&self, index: usize) -> usize {
        self.art_bytes
            .min((index + 1).saturating_mul(DATA_CHUNK_BYTES))
    }

    pub(crate) fn initialize_expression(&self) -> String {
        let key = json_string(&self.storage_key);
        let token = json_string(&self.token);
        let theme_sha256 = json_string(&self.theme_sha256);
        let css_sha256 = json_string(&self.css_sha256);
        let art_mime = json_string(&self.art_mime);
        let art_sha256 = json_string(&self.art_sha256);
        let theme_bytes = self.theme_bytes;
        let css_bytes = self.css_bytes;
        let art_bytes = self.art_bytes;
        let theme_chunk_count = self.theme_chunks.len();
        let css_chunk_count = self.css_chunks.len();
        let art_chunk_count = self.art_chunks.len();
        let ttl_ms = TRANSFER_TTL_MS;
        format!(
            r#"(() => {{
  const key = {key};
  const token = {token};
  const previous = globalThis[key];
  if (previous?.cleanupTimer !== undefined) globalThis.clearTimeout(previous.cleanupTimer);
  const cleanupTimer = globalThis.setTimeout(() => {{
    if (globalThis[key]?.token === token) delete globalThis[key];
  }}, {ttl_ms});
  globalThis[key] = {{
    token,
    phase: "initialized",
    theme: null,
    themeBytes: {theme_bytes},
    themeSha256: {theme_sha256},
    themeChunkCount: {theme_chunk_count},
    themeReceivedBytes: 0,
    themeParts: [],
    cssText: null,
    cssBytes: {css_bytes},
    cssSha256: {css_sha256},
    cssChunkCount: {css_chunk_count},
    cssReceivedBytes: 0,
    cssParts: [],
    artMime: {art_mime},
    artBytes: {art_bytes},
    artSha256: {art_sha256},
    artChunkCount: {art_chunk_count},
    artReceivedBytes: 0,
    artParts: [],
    result: null,
    error: null,
    failedPhase: null,
    timings: {{}},
    cleanupTimer,
  }};
  return {{ accepted: true, token, phase: "initialized", receivedChunks: 0, receivedBytes: 0 }};
}})()"#
        )
    }

    pub(crate) fn theme_chunk_expression(&self, index: usize) -> String {
        let key = json_string(&self.storage_key);
        let token = json_string(&self.token);
        let chunk = json_string(&self.theme_chunks[index]);
        let ttl_ms = TRANSFER_TTL_MS;
        format!(
            r#"(() => {{
  const key = {key};
  const token = {token};
  const transfer = globalThis[key];
  if (!transfer || transfer.token !== token) throw new Error("LumaDrobe transfer token mismatch");
  if (transfer.phase !== "initialized" && transfer.phase !== "themeReceiving") {{
    throw new Error("LumaDrobe metadata transfer phase mismatch");
  }}
  if (transfer.themeParts.length !== {index}) throw new Error("LumaDrobe metadata chunk order mismatch");
  const binary = atob({chunk});
  const bytes = new Uint8Array(binary.length);
  for (let offset = 0; offset < binary.length; offset += 1) {{
    bytes[offset] = binary.charCodeAt(offset);
  }}
  transfer.themeParts.push(bytes);
  transfer.themeReceivedBytes += bytes.byteLength;
  if (transfer.themeReceivedBytes > transfer.themeBytes) throw new Error("LumaDrobe metadata payload overflow");
  transfer.phase = "themeReceiving";
  if (transfer.cleanupTimer !== undefined) globalThis.clearTimeout(transfer.cleanupTimer);
  transfer.cleanupTimer = globalThis.setTimeout(() => {{
    if (globalThis[key]?.token === token) delete globalThis[key];
  }}, {ttl_ms});
  return {{
    accepted: true,
    token,
    phase: transfer.phase,
    receivedChunks: transfer.themeParts.length,
    receivedBytes: transfer.themeReceivedBytes,
  }};
}})()"#
        )
    }

    pub(crate) fn css_chunk_expression(&self, index: usize) -> String {
        let key = json_string(&self.storage_key);
        let token = json_string(&self.token);
        let chunk = json_string(&self.css_chunks[index]);
        let ttl_ms = TRANSFER_TTL_MS;
        format!(
            r#"(() => {{
  const key = {key};
  const token = {token};
  const transfer = globalThis[key];
  if (!transfer || transfer.token !== token) throw new Error("LumaDrobe transfer token mismatch");
  if (transfer.phase !== "themeReceiving" && transfer.phase !== "cssReceiving") {{
    throw new Error("LumaDrobe CSS transfer phase mismatch");
  }}
  if (transfer.themeParts.length !== transfer.themeChunkCount ||
      transfer.themeReceivedBytes !== transfer.themeBytes) {{
    throw new Error("LumaDrobe metadata transfer is incomplete");
  }}
  if (transfer.cssParts.length !== {index}) throw new Error("LumaDrobe CSS chunk order mismatch");
  const binary = atob({chunk});
  const bytes = new Uint8Array(binary.length);
  for (let offset = 0; offset < binary.length; offset += 1) {{
    bytes[offset] = binary.charCodeAt(offset);
  }}
  transfer.cssParts.push(bytes);
  transfer.cssReceivedBytes += bytes.byteLength;
  if (transfer.cssReceivedBytes > transfer.cssBytes) throw new Error("LumaDrobe CSS payload overflow");
  transfer.phase = "cssReceiving";
  if (transfer.cleanupTimer !== undefined) globalThis.clearTimeout(transfer.cleanupTimer);
  transfer.cleanupTimer = globalThis.setTimeout(() => {{
    if (globalThis[key]?.token === token) delete globalThis[key];
  }}, {ttl_ms});
  return {{
    accepted: true,
    token,
    phase: transfer.phase,
    receivedChunks: transfer.cssParts.length,
    receivedBytes: transfer.cssReceivedBytes,
  }};
}})()"#
        )
    }

    pub(crate) fn art_chunk_expression(&self, index: usize) -> String {
        let key = json_string(&self.storage_key);
        let token = json_string(&self.token);
        let chunk = json_string(&self.art_chunks[index]);
        let ttl_ms = TRANSFER_TTL_MS;
        format!(
            r#"(() => {{
  const key = {key};
  const token = {token};
  const transfer = globalThis[key];
  if (!transfer || transfer.token !== token) throw new Error("LumaDrobe transfer token mismatch");
  if (transfer.phase !== "cssReceiving" && transfer.phase !== "artReceiving") {{
    throw new Error("LumaDrobe art transfer phase mismatch");
  }}
  if (transfer.cssParts.length !== transfer.cssChunkCount ||
      transfer.cssReceivedBytes !== transfer.cssBytes) {{
    throw new Error("LumaDrobe CSS transfer is incomplete");
  }}
  if (transfer.artParts.length !== {index}) throw new Error("LumaDrobe art chunk order mismatch");
  const binary = atob({chunk});
  const bytes = new Uint8Array(binary.length);
  for (let offset = 0; offset < binary.length; offset += 1) {{
    bytes[offset] = binary.charCodeAt(offset);
  }}
  transfer.artParts.push(bytes);
  transfer.artReceivedBytes += bytes.byteLength;
  if (transfer.artReceivedBytes > transfer.artBytes) throw new Error("LumaDrobe art payload overflow");
  transfer.phase = "artReceiving";
  if (transfer.cleanupTimer !== undefined) globalThis.clearTimeout(transfer.cleanupTimer);
  transfer.cleanupTimer = globalThis.setTimeout(() => {{
    if (globalThis[key]?.token === token) delete globalThis[key];
  }}, {ttl_ms});
  return {{
    accepted: true,
    token,
    phase: transfer.phase,
    receivedChunks: transfer.artParts.length,
    receivedBytes: transfer.artReceivedBytes,
  }};
}})()"#
        )
    }

    pub(crate) fn start_expression(&self) -> String {
        let key = json_string(&self.storage_key);
        let token = json_string(&self.token);
        let engine_key = json_string(ENGINE_KEY);
        let engine_version = ENGINE_VERSION;
        let ttl_ms = TRANSFER_TTL_MS;
        format!(
            r#"(() => {{
  const key = {key};
  const token = {token};
  const transfer = globalThis[key];
  if (!transfer || transfer.token !== token) throw new Error("LumaDrobe transfer token mismatch");
  if (transfer.phase !== "artReceiving" ||
      transfer.themeParts.length !== transfer.themeChunkCount ||
      transfer.themeReceivedBytes !== transfer.themeBytes ||
      transfer.cssParts.length !== transfer.cssChunkCount ||
      transfer.cssReceivedBytes !== transfer.cssBytes ||
      transfer.artParts.length !== transfer.artChunkCount ||
      transfer.artReceivedBytes !== transfer.artBytes ||
      transfer.theme !== null ||
      transfer.cssText !== null) {{
    throw new Error("LumaDrobe staged theme is incomplete");
  }}
  const engine = globalThis[{engine_key}];
  if (engine?.engineVersion !== {engine_version} || typeof engine?.install !== "function") {{
    throw new Error("LumaDrobe renderer engine is unavailable");
  }}
  if (transfer.cleanupTimer !== undefined) globalThis.clearTimeout(transfer.cleanupTimer);
  transfer.cleanupTimer = globalThis.setTimeout(() => {{
    if (globalThis[key]?.token === token) delete globalThis[key];
  }}, {ttl_ms});
  transfer.phase = "queued";
  transfer.startedAt = performance.now();
  globalThis.setTimeout(() => {{
    void (async () => {{
      const current = globalThis[key];
      if (!current || current.token !== token || current.phase !== "queued") return;
      const mark = (phase) => {{
        current.phase = phase;
        current.timings[phase] = Number((performance.now() - current.startedAt).toFixed(3));
      }};
      const digest = async (bytes) => {{
        const value = new Uint8Array(await crypto.subtle.digest("SHA-256", bytes));
        return Array.from(value, (item) => item.toString(16).padStart(2, "0")).join("");
      }};
      try {{
        if (!globalThis.crypto?.subtle) throw new Error("LumaDrobe SHA-256 is unavailable");
        mark("validatingMetadata");
        const themeBlob = new Blob(current.themeParts);
        if (themeBlob.size !== current.themeBytes) {{
          throw new Error("LumaDrobe metadata Blob size mismatch");
        }}
        const themeBuffer = await themeBlob.arrayBuffer();
        if (await digest(themeBuffer) !== current.themeSha256) {{
          throw new Error("LumaDrobe metadata SHA-256 mismatch");
        }}
        const themeText = new TextDecoder("utf-8", {{ fatal: true }}).decode(themeBuffer);
        const theme = JSON.parse(themeText);
        if (!theme || typeof theme !== "object" || Array.isArray(theme) ||
            typeof theme.key !== "string" || typeof theme.id !== "string" ||
            typeof theme.name !== "string" || typeof theme.version !== "string") {{
          throw new Error("LumaDrobe metadata shape is invalid");
        }}
        current.theme = theme;
        current.themeParts = [];
        mark("validatingCss");
        const cssBlob = new Blob(current.cssParts);
        if (cssBlob.size !== current.cssBytes) throw new Error("LumaDrobe CSS Blob size mismatch");
        const cssBuffer = await cssBlob.arrayBuffer();
        if (await digest(cssBuffer) !== current.cssSha256) {{
          throw new Error("LumaDrobe CSS SHA-256 mismatch");
        }}
        current.cssText = new TextDecoder("utf-8", {{ fatal: true }}).decode(cssBuffer);
        current.cssParts = [];
        mark("assemblingImage");
        const artBlob = new Blob(current.artParts, {{ type: current.artMime }});
        if (artBlob.size !== current.artBytes) throw new Error("LumaDrobe art Blob size mismatch");
        current.artParts = [];
        mark("validatingImage");
        const artBuffer = await artBlob.arrayBuffer();
        if (await digest(artBuffer) !== current.artSha256) {{
          throw new Error("LumaDrobe art SHA-256 mismatch");
        }}
        mark("installing");
        current.result = engine.install(current.theme, current.cssText, artBlob);
        mark("installed");
      }} catch (error) {{
        current.failedPhase = current.phase;
        current.error = String(error?.message || error || "unknown renderer failure").slice(0, 500);
        mark("failed");
      }}
    }})();
  }}, 0);
  return {{ accepted: true, token, phase: transfer.phase }};
}})()"#
        )
    }

    pub(crate) fn status_expression(&self) -> String {
        let key = json_string(&self.storage_key);
        let token = json_string(&self.token);
        format!(
            r#"(() => {{
  const transfer = globalThis[{key}];
  if (!transfer || transfer.token !== {token}) {{
    return {{ accepted: false, phase: "missing" }};
  }}
  return {{
    accepted: true,
    token: transfer.token,
    phase: transfer.phase,
    receivedChunks: transfer.artParts.length,
    receivedBytes: transfer.artReceivedBytes,
    result: transfer.phase === "installed" ? transfer.result : null,
    error: transfer.phase === "failed" ? transfer.error : null,
    failedPhase: transfer.phase === "failed" ? transfer.failedPhase : null,
    timings: transfer.timings,
  }};
}})()"#
        )
    }

    pub(crate) fn cleanup_expression(&self) -> String {
        let key = json_string(&self.storage_key);
        let token = json_string(&self.token);
        format!(
            r#"(() => {{
  const key = {key};
  const transfer = globalThis[key];
  if (transfer?.token === {token}) {{
    if (transfer.cleanupTimer !== undefined) globalThis.clearTimeout(transfer.cleanupTimer);
    transfer.artParts = [];
    transfer.cssParts = [];
    transfer.themeParts = [];
    transfer.theme = null;
    transfer.cssText = null;
    delete globalThis[key];
  }}
  return true;
}})()"#
        )
    }
}

fn next_attempt_id() -> String {
    let sequence = TRANSFER_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{:x}-{timestamp:x}-{sequence:x}", std::process::id())
}

fn json_string(value: &str) -> String {
    serde_json::to_string(value).expect("string serialization cannot fail")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::renderer_payload::RendererPayload;

    fn payload(art_bytes: usize) -> RendererPayload {
        RendererPayload::test_fixture(
            "source:night@1.0.0",
            "body { color: white; }",
            "image/png",
            vec![7_u8; art_bytes],
        )
    }

    #[test]
    fn art_is_encoded_in_bounded_independent_chunks() {
        let plan = ThemeTransferPlan::new(&payload(DATA_CHUNK_BYTES * 2 + 17));
        assert_eq!(plan.art_chunk_count(), 3);
        assert!(plan
            .art_chunks
            .iter()
            .all(|chunk| { chunk.len() <= DATA_CHUNK_BYTES.div_ceil(3) * 4 }));
        let decoded = plan
            .art_chunks
            .iter()
            .flat_map(|chunk| STANDARD.decode(chunk).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(decoded, vec![7_u8; DATA_CHUNK_BYTES * 2 + 17]);
    }

    #[test]
    fn css_is_encoded_in_bounded_utf8_safe_chunks() {
        let css = "皮肤{color:white}".repeat(12_000);
        let payload =
            RendererPayload::test_fixture("source:night@1.0.0", &css, "image/png", vec![7_u8; 16]);
        let plan = ThemeTransferPlan::new(&payload);
        assert!(plan.css_chunk_count() > 1);
        assert!(plan
            .css_chunks
            .iter()
            .all(|chunk| chunk.len() <= DATA_CHUNK_BYTES.div_ceil(3) * 4));
        let decoded = plan
            .css_chunks
            .iter()
            .flat_map(|chunk| STANDARD.decode(chunk).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(decoded, css.as_bytes());
    }

    #[test]
    fn metadata_is_encoded_in_bounded_independent_chunks() {
        let name = "夜色主题".repeat(20_000);
        let payload = RendererPayload::test_fixture_with_name(
            "source:night@1.0.0",
            &name,
            "body { color: white; }",
            "image/png",
            vec![7_u8; 16],
        );
        let plan = ThemeTransferPlan::new(&payload);
        assert!(plan.theme_chunk_count() > 1);
        assert!(plan
            .theme_chunks
            .iter()
            .all(|chunk| chunk.len() <= DATA_CHUNK_BYTES.div_ceil(3) * 4));
        let decoded = plan
            .theme_chunks
            .iter()
            .flat_map(|chunk| STANDARD.decode(chunk).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(decoded, payload.theme_json().as_bytes());
        assert!(!plan.initialize_expression().contains(&name));
    }

    #[test]
    fn generated_protocol_separates_code_data_and_installation() {
        let plan = ThemeTransferPlan::new(&payload(DATA_CHUNK_BYTES + 1));
        let initialize = plan.initialize_expression();
        let metadata = plan.theme_chunk_expression(0);
        let css = plan.css_chunk_expression(0);
        let chunk = plan.art_chunk_expression(0);
        let start = plan.start_expression();
        let status = plan.status_expression();
        let cleanup = plan.cleanup_expression();

        assert!(initialize.contains(TRANSFER_KEY_PREFIX));
        assert!(initialize.contains(plan.theme_sha256()));
        assert!(initialize.contains(plan.art_sha256()));
        assert!(!initialize.contains("source:night@1.0.0"));
        assert!(metadata.contains("metadata chunk order mismatch"));
        assert!(metadata.len() < 192 * 1024);
        assert!(css.contains("CSS chunk order mismatch"));
        assert!(css.len() < 192 * 1024);
        assert!(chunk.contains("art chunk order mismatch"));
        assert!(chunk.len() < 192 * 1024);
        assert!(start.contains("globalThis.setTimeout"));
        assert!(start.contains("validatingMetadata"));
        assert!(start.contains("JSON.parse"));
        assert!(start.contains("validatingCss"));
        assert!(start.contains("TextDecoder(\"utf-8\", { fatal: true })"));
        assert!(start.contains("validatingImage"));
        assert!(start.contains("engine.install"));
        assert!(status.contains("transfer.phase"));
        assert!(start.len() < 32 * 1024);
        assert!(status.len() < 8 * 1024);
        assert!(cleanup.contains("delete globalThis[key]"));
        assert!(!start.contains("eval("));
    }

    #[test]
    fn attempts_use_independent_storage_keys_and_tokens() {
        let payload = payload(16);
        let first = ThemeTransferPlan::new(&payload);
        let second = ThemeTransferPlan::new(&payload);
        assert_ne!(first.storage_key, second.storage_key);
        assert_ne!(first.token, second.token);
    }
}
