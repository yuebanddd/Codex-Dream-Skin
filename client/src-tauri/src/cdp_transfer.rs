use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use sha2::{Digest, Sha256};

pub(crate) const INLINE_EXPRESSION_LIMIT_BYTES: usize = 192 * 1024;
const ENCODED_CHUNK_BYTES: usize = 96 * 1024;
const TRANSFER_KEY: &str = "__LUMADROBE_CDP_TRANSFER__";

#[derive(Debug)]
pub(crate) struct TransferPlan {
    token: String,
    sha256: String,
    original_bytes: usize,
    encoded_bytes: usize,
    chunks: Vec<String>,
}

impl TransferPlan {
    pub(crate) fn new(expression: &str) -> Self {
        let sha256 = format!("{:x}", Sha256::digest(expression.as_bytes()));
        let encoded = STANDARD.encode(expression.as_bytes());
        let chunks = encoded
            .as_bytes()
            .chunks(ENCODED_CHUNK_BYTES)
            .map(|chunk| String::from_utf8(chunk.to_vec()).expect("base64 is ASCII"))
            .collect::<Vec<_>>();
        Self {
            token: format!("{}-{}", &sha256[..20], expression.len()),
            sha256,
            original_bytes: expression.len(),
            encoded_bytes: encoded.len(),
            chunks,
        }
    }

    pub(crate) fn sha256(&self) -> &str {
        &self.sha256
    }

    pub(crate) fn original_bytes(&self) -> usize {
        self.original_bytes
    }

    pub(crate) fn chunk_count(&self) -> usize {
        self.chunks.len()
    }

    pub(crate) fn initialize_expression(&self) -> String {
        let key = json_string(TRANSFER_KEY);
        let token = json_string(&self.token);
        let sha256 = json_string(&self.sha256);
        let original_bytes = self.original_bytes;
        let encoded_bytes = self.encoded_bytes;
        let chunk_count = self.chunks.len();
        format!(
            r#"(() => {{
  const key = {key};
  const token = {token};
  globalThis[key] = {{
    token,
    sha256: {sha256},
    originalBytes: {original_bytes},
    encodedBytes: {encoded_bytes},
    chunkCount: {chunk_count},
    chunks: [],
  }};
  return {{ accepted: true, token, receivedChunks: 0 }};
}})()"#
        )
    }

    pub(crate) fn append_expression(&self, index: usize) -> String {
        let key = json_string(TRANSFER_KEY);
        let token = json_string(&self.token);
        let chunk = json_string(&self.chunks[index]);
        format!(
            r#"(() => {{
  const transfer = globalThis[{key}];
  if (!transfer || transfer.token !== {token}) throw new Error("LumaDrobe transfer token mismatch");
  if (transfer.chunks.length !== {index}) throw new Error("LumaDrobe transfer chunk order mismatch");
  transfer.chunks.push({chunk});
  return {{ accepted: true, token: transfer.token, receivedChunks: transfer.chunks.length }};
}})()"#
        )
    }

    pub(crate) fn commit_expression(&self) -> String {
        let key = json_string(TRANSFER_KEY);
        let token = json_string(&self.token);
        format!(
            r#"(async () => {{
  const key = {key};
  const token = {token};
  const transfer = globalThis[key];
  try {{
    if (!transfer || transfer.token !== token) throw new Error("LumaDrobe transfer token mismatch");
    if (transfer.chunks.length !== transfer.chunkCount) throw new Error("LumaDrobe transfer is incomplete");
    const encoded = transfer.chunks.join("");
    if (encoded.length !== transfer.encodedBytes) throw new Error("LumaDrobe transfer encoded length mismatch");
    const binary = atob(encoded);
    const bytes = Uint8Array.from(binary, (value) => value.charCodeAt(0));
    if (bytes.byteLength !== transfer.originalBytes) throw new Error("LumaDrobe transfer byte length mismatch");
    if (!globalThis.crypto?.subtle) throw new Error("LumaDrobe transfer SHA-256 is unavailable");
    const digestBytes = new Uint8Array(await globalThis.crypto.subtle.digest("SHA-256", bytes));
    const digest = Array.from(digestBytes, (value) => value.toString(16).padStart(2, "0")).join("");
    if (digest !== transfer.sha256) throw new Error("LumaDrobe transfer SHA-256 mismatch");
    const source = new TextDecoder("utf-8", {{ fatal: true }}).decode(bytes);
    return (0, eval)(source);
  }} finally {{
    if (globalThis[key]?.token === token) delete globalThis[key];
  }}
}})()"#
        )
    }

    pub(crate) fn cleanup_expression(&self) -> String {
        let key = json_string(TRANSFER_KEY);
        let token = json_string(&self.token);
        format!(
            r#"(() => {{
  const key = {key};
  if (globalThis[key]?.token === {token}) delete globalThis[key];
  return true;
}})()"#
        )
    }
}

pub(crate) fn requires_chunking(expression: &str) -> bool {
    expression.len() > INLINE_EXPRESSION_LIMIT_BYTES
}

fn json_string(value: &str) -> String {
    serde_json::to_string(value).expect("string serialization cannot fail")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_expressions_remain_inline() {
        assert!(!requires_chunking("true"));
        assert!(!requires_chunking(
            &"a".repeat(INLINE_EXPRESSION_LIMIT_BYTES)
        ));
        assert!(requires_chunking(
            &"a".repeat(INLINE_EXPRESSION_LIMIT_BYTES + 1)
        ));
    }

    #[test]
    fn chunk_plan_round_trips_utf8_and_bounds_each_message() {
        let expression = format!(
            "(() => {{ const message = {:?}; return message.length; }})()",
            "皮肤分片验证\\\"".repeat(40_000)
        );
        let plan = TransferPlan::new(&expression);
        assert!(plan.chunk_count() > 1);
        assert!(plan
            .chunks
            .iter()
            .all(|chunk| chunk.len() <= ENCODED_CHUNK_BYTES));
        let encoded = plan.chunks.concat();
        assert_eq!(STANDARD.decode(encoded).unwrap(), expression.as_bytes());
        assert_eq!(plan.original_bytes(), expression.len());
        assert_eq!(plan.sha256(), format!("{:x}", Sha256::digest(expression)));
    }

    #[test]
    fn generated_protocol_checks_order_digest_and_cleanup() {
        let plan = TransferPlan::new(&"payload".repeat(40_000));
        let initialize = plan.initialize_expression();
        let append = plan.append_expression(0);
        let commit = plan.commit_expression();
        let cleanup = plan.cleanup_expression();

        assert!(initialize.contains(TRANSFER_KEY));
        assert!(initialize.contains(plan.sha256()));
        assert!(append.contains("chunk order mismatch"));
        assert!(commit.contains("crypto.subtle.digest(\"SHA-256\", bytes)"));
        assert!(commit.contains("TextDecoder(\"utf-8\", { fatal: true })"));
        assert!(commit.contains("delete globalThis[key]"));
        assert!(cleanup.contains("delete globalThis[key]"));
    }

    #[test]
    fn append_commands_stay_below_the_inline_limit() {
        let plan = TransferPlan::new(&"\\\"\\\\\n".repeat(100_000));
        for index in 0..plan.chunk_count() {
            assert!(plan.append_expression(index).len() < INLINE_EXPRESSION_LIMIT_BYTES);
        }
    }
}
