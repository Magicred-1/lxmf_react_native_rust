//! Beacon-side JSON-RPC 2.0 server for the anon0mesh co-sign protocol.
//!
//! When this node runs as a beacon (`is_beacon = true`), inbound link data that
//! looks like a JSON-RPC *request* (has a "method" field) is routed here instead
//! of being treated as a client-side response.
//!
//! Two categories of method are handled:
//!   1. `cosignTransaction` — sign slot 1 of the partial tx and submit to Solana
//!   2. Everything else   — forward to the configured Solana JSON-RPC URL (proxy)

use base64::Engine as _;
use ed25519_dalek::Signer;

use crate::beacon::{compress_payload, decompress_payload, RpcRequest};

// ── Public config ─────────────────────────────────────────────────────────────

/// Runtime configuration for a beacon node.
/// Stored in `LxmfNode` behind `Arc<Mutex>` so FFI can update it after init.
#[derive(Default)]
pub struct BeaconConfig {
    /// ed25519 signing keypair for co-signing Solana transactions (slot 1 / broadcaster).
    pub keypair: Option<ed25519_dalek::SigningKey>,
    /// Solana JSON-RPC endpoint URL (e.g. "https://api.mainnet-beta.solana.com").
    pub solana_rpc_url: Option<String>,
}

// ── Request detection ─────────────────────────────────────────────────────────

/// Returns true if `data` (possibly compressed) looks like a JSON-RPC *request*.
///
/// Both requests and responses are JSON/zlib, but only requests carry a "method" key.
/// This distinguishes beacon-side (incoming request) from client-side (incoming response).
pub fn is_rpc_request(data: &[u8]) -> bool {
    let raw = match decompress_payload(data) {
        Ok(r) => r,
        Err(_) => return false,
    };
    let v: serde_json::Value = match serde_json::from_slice(&raw) {
        Ok(v) => v,
        Err(_) => return false,
    };
    v.get("method").is_some()
}

// ── Top-level dispatcher ──────────────────────────────────────────────────────

/// Parse and handle an incoming JSON-RPC request. Returns compressed response bytes.
///
/// On any error (parse failure, signing failure, HTTP error) returns a compressed
/// JSON-RPC error response so the client always gets a well-formed reply.
pub async fn handle_rpc_request(
    data: &[u8],
    keypair: &ed25519_dalek::SigningKey,
    solana_rpc_url: &str,
) -> Vec<u8> {
    let raw = match decompress_payload(data) {
        Ok(r) => r,
        Err(e) => return error_response(0, -32700, &format!("decompress failed: {e}")),
    };
    let req: RpcRequest = match serde_json::from_slice(&raw) {
        Ok(r) => r,
        Err(e) => return error_response(0, -32700, &format!("parse failed: {e}")),
    };

    let id = req.id;
    match req.method.as_str() {
        "cosignTransaction" => cosign_and_submit(id, &req.params, keypair, solana_rpc_url).await,
        method => proxy_solana(id, method, req.params, solana_rpc_url).await,
    }
}

// ── cosignTransaction ─────────────────────────────────────────────────────────

/// Co-sign a partial Solana transaction and submit it via `sendRawTransaction`.
///
/// Protocol (matches `partial_sign_execute_payment` in solana_tx.rs):
///   Wire format: [compact_u16: num_sigs] [sig_0: 64B] [sig_1: 64B (zeros)] [message...]
///   Beacon signs `message` bytes and fills sig_1.
///
/// Returns compressed JSON-RPC response: `{"result": "tx_sig_b58"}` or an error.
async fn cosign_and_submit(
    request_id: u32,
    params: &serde_json::Value,
    keypair: &ed25519_dalek::SigningKey,
    solana_rpc_url: &str,
) -> Vec<u8> {
    // params is a JSON array; element 0 is the base64 partial tx
    let partial_tx_b64 = match params.get(0).and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return error_response(request_id, -32602, "cosignTransaction: missing params[0]"),
    };

    let mut tx_bytes = match base64::engine::general_purpose::STANDARD.decode(partial_tx_b64) {
        Ok(b) => b,
        Err(e) => return error_response(request_id, -32602, &format!("base64 decode: {e}")),
    };

    // Parse compact_u16 signature count
    let (num_sigs, consumed) = match read_compact_u16(&tx_bytes) {
        Some(v) => v,
        None => return error_response(request_id, -32602, "tx: compact_u16 parse failed"),
    };
    if num_sigs < 2 {
        return error_response(request_id, -32602, "tx: expected at least 2 signers");
    }

    let sig1_start = consumed + 64;      // slot 1 starts after sig_0
    let sig1_end   = sig1_start + 64;
    let msg_start  = consumed + (num_sigs as usize) * 64;

    if tx_bytes.len() < msg_start {
        return error_response(request_id, -32602, "tx: too short for declared sig count");
    }

    // Sign the Solana message bytes (everything after the signature array)
    let message_bytes = tx_bytes[msg_start..].to_vec();

    // TODO(security): verify message contains a valid execute_payment discriminator
    // before signing, to prevent the beacon being used to co-sign arbitrary transactions.

    let sig: ed25519_dalek::Signature = keypair.sign(&message_bytes);
    tx_bytes[sig1_start..sig1_end].copy_from_slice(&sig.to_bytes());

    let full_tx_b64 = base64::engine::general_purpose::STANDARD.encode(&tx_bytes);

    // Submit to Solana
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "sendRawTransaction",
        "params": [full_tx_b64, {"encoding": "base64", "skipPreflight": true}]
    });

    let client = match build_http_client() {
        Ok(c) => c,
        Err(e) => return error_response(request_id, -32000, &format!("http client: {e}")),
    };

    let resp_json = match post_json(&client, solana_rpc_url, &body).await {
        Ok(v) => v,
        Err(e) => return error_response(request_id, -32000, &format!("sendRawTransaction: {e}")),
    };

    // Relay Solana's result/error back to the client, preserving our request_id
    rewrap_response(request_id, resp_json)
}

// ── Solana RPC proxy ──────────────────────────────────────────────────────────

/// Forward any other Solana JSON-RPC method straight to the configured RPC URL.
/// Preserves the client's `request_id` in the response.
async fn proxy_solana(
    request_id: u32,
    method: &str,
    params: serde_json::Value,
    solana_rpc_url: &str,
) -> Vec<u8> {
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": request_id,
        "method": method,
        "params": params,
    });

    let client = match build_http_client() {
        Ok(c) => c,
        Err(e) => return error_response(request_id, -32000, &format!("http client: {e}")),
    };

    let resp_json = match post_json(&client, solana_rpc_url, &body).await {
        Ok(v) => v,
        Err(e) => return error_response(request_id, -32000, &format!("proxy {method}: {e}")),
    };

    rewrap_response(request_id, resp_json)
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Parse a Solana compact_u16 from the start of `data`.
/// Returns `(value, bytes_consumed)` or `None` on malformed input.
fn read_compact_u16(data: &[u8]) -> Option<(u16, usize)> {
    let mut result = 0u16;
    let mut shift = 0u16;
    for (i, &b) in data.iter().enumerate().take(3) {
        result |= ((b & 0x7f) as u16) << shift;
        shift += 7;
        if b & 0x80 == 0 {
            return Some((result, i + 1));
        }
    }
    None
}

fn build_http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())
}

async fn post_json(
    client: &reqwest::Client,
    url: &str,
    body: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    client
        .post(url)
        .json(body)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json::<serde_json::Value>()
        .await
        .map_err(|e| e.to_string())
}

/// Re-wrap a Solana HTTP response, fixing the `id` field to match the client's request_id.
/// Compresses the result using the beacon protocol's zlib encoding.
fn rewrap_response(request_id: u32, mut resp: serde_json::Value) -> Vec<u8> {
    resp["id"] = serde_json::json!(request_id);
    let bytes = serde_json::to_vec(&resp).unwrap_or_default();
    compress_payload(&bytes)
}

/// Build a compressed JSON-RPC error response.
fn error_response(id: u32, code: i32, message: &str) -> Vec<u8> {
    let resp = serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {"code": code, "message": message}
    });
    compress_payload(resp.to_string().as_bytes())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_u16_single_byte() {
        assert_eq!(read_compact_u16(&[0x02]), Some((2, 1)));
        assert_eq!(read_compact_u16(&[0x7f]), Some((127, 1)));
    }

    #[test]
    fn compact_u16_two_bytes() {
        assert_eq!(read_compact_u16(&[0x80, 0x01]), Some((128, 2)));
    }

    #[test]
    fn is_rpc_request_detects_method_field() {
        let req = r#"{"jsonrpc":"2.0","id":1,"method":"getLatestBlockhash","params":[]}"#;
        assert!(is_rpc_request(req.as_bytes()));
    }

    #[test]
    fn is_rpc_request_rejects_response() {
        let resp = r#"{"jsonrpc":"2.0","id":1,"result":"5NG..."}"#;
        assert!(!is_rpc_request(resp.as_bytes()));
    }

    #[test]
    fn is_rpc_request_handles_compressed() {
        use crate::beacon::compress_payload;
        let req = r#"{"jsonrpc":"2.0","id":1,"method":"getLatestBlockhash","params":[]}"#;
        let compressed = compress_payload(req.as_bytes());
        assert!(is_rpc_request(&compressed));
    }

    #[test]
    fn is_rpc_request_rejects_compressed_response() {
        use crate::beacon::compress_payload;
        let resp = r#"{"jsonrpc":"2.0","id":1,"result":"5NG..."}"#;
        let compressed = compress_payload(resp.as_bytes());
        assert!(!is_rpc_request(&compressed));
    }

    #[test]
    fn error_response_is_valid_json() {
        let bytes = error_response(7, -32000, "test error");
        // decompress and check
        let raw = decompress_payload(&bytes).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&raw).unwrap();
        assert_eq!(v["id"], 7);
        assert_eq!(v["error"]["code"], -32000);
    }
}
