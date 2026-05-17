//! Solana transaction building for the anon0mesh execute_payment co-sign flow.
//!
//! Client side of the protocol:
//!   1. Fetch nonce blockhash via `BeaconManager::request_account_info`
//!   2. Call `partial_sign_execute_payment` → base64 partial tx
//!   3. Call `BeaconManager::request_cosign_transaction(dest, partial_tx_b64)`
//!   4. Beacon co-signs slot 1 (broadcaster) and submits to Solana
//!
//! Wire format: legacy Solana transaction (non-versioned).
//!
//! Reference: wallet.py in anon0mesh_cli — offline_sign_nonce_transfer and
//! partial_sign_execute_payment functions.

use base64::Engine as _;
use ed25519_dalek::{Signer, SigningKey};
use sha2::{Digest, Sha256};

// ── Anchor discriminator ──────────────────────────────────────────────────────

/// Compute Anchor instruction discriminator: sha256("global:<method>")[..8].
pub fn anchor_discriminator(method: &str) -> [u8; 8] {
    let preimage = format!("global:{method}");
    let hash = Sha256::digest(preimage.as_bytes());
    hash[..8].try_into().expect("sha256 is always 32 bytes")
}

// ── Well-known Solana program IDs ─────────────────────────────────────────────

pub mod pubkeys {
    /// System Program: 11111111111111111111111111111111
    pub const SYSTEM_PROGRAM: [u8; 32] = [0u8; 32];

    /// SPL Token Program: TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA
    pub const TOKEN_PROGRAM: [u8; 32] = [
        6, 221, 246, 225, 215, 101, 161, 147, 217, 203, 225, 70, 206, 235, 121, 172,
        28, 180, 133, 237, 95, 91, 55, 145, 58, 140, 245, 133, 126, 255, 0, 169,
    ];

    /// Associated Token Program: ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJe8bXh
    pub const ASSOCIATED_TOKEN_PROGRAM: [u8; 32] = [
        140, 151, 37, 143, 78, 36, 137, 241, 187, 61, 16, 41, 20, 142, 13, 131,
        11, 90, 19, 153, 218, 255, 16, 132, 4, 142, 123, 216, 219, 233, 248, 89,
    ];

    /// SysvarRent: SysvarRent111111111111111111111111111111111
    pub const SYSVAR_RENT: [u8; 32] = [
        6, 167, 213, 23, 24, 199, 116, 201, 40, 86, 99, 152, 105, 29, 94, 182,
        139, 94, 184, 163, 155, 75, 109, 92, 115, 85, 91, 33, 0, 0, 0, 0,
    ];

    /// SysvarRecentBlockhashes: SysvarRecentB1ockHashes11111111111111111111111
    pub const SYSVAR_RECENT_BLOCKHASHES: [u8; 32] = [
        6, 167, 213, 23, 19, 172, 202, 82, 33, 140, 201, 76, 61, 74, 241, 127,
        88, 218, 238, 8, 155, 161, 253, 68, 227, 219, 217, 138, 0, 0, 0, 0,
    ];
}

// ── execute_payment instruction ───────────────────────────────────────────────

/// Parameters for the ble_revshare `execute_payment` Anchor instruction.
pub struct ExecutePaymentParams {
    /// Compensation offset (u64 LE).
    pub comp_offset: u64,
    /// Transfer amount in lamports (u64 LE).
    pub amount: u64,
    /// Arcium Rescue-encrypted amount — 32 bytes.
    pub encrypted_amount: [u8; 32],
    /// Arcium encryption nonce — u128, serialized as 16 bytes LE.
    pub nonce: u128,
    /// Arcium MXE public key — 32 bytes.
    pub encryption_pub_key: [u8; 32],
}

/// Build the 104-byte execute_payment instruction data.
///
/// Layout (matches wallet.py):
///   [discriminator 8B][comp_offset 8B LE][amount 8B LE]
///   [encrypted_amount 32B][nonce 16B LE][pub_key 32B]
pub fn build_execute_payment_ix_data(params: &ExecutePaymentParams) -> [u8; 104] {
    let mut data = [0u8; 104];
    data[0..8].copy_from_slice(&anchor_discriminator("execute_payment"));
    data[8..16].copy_from_slice(&params.comp_offset.to_le_bytes());
    data[16..24].copy_from_slice(&params.amount.to_le_bytes());
    data[24..56].copy_from_slice(&params.encrypted_amount);
    data[56..72].copy_from_slice(&params.nonce.to_le_bytes());
    data[72..104].copy_from_slice(&params.encryption_pub_key);
    data
}

// ── Accounts ──────────────────────────────────────────────────────────────────

/// All accounts required for execute_payment with a durable nonce.
///
/// Account ordering in the compiled message matches the ble_revshare IDL:
///   Signers (writable):      payer [0], broadcaster [1]
///   Writable non-signers:    nonce_account [2], payer_ata [3], recipient [4],
///                            recipient_ata [5], broadcaster_ata [6]
///   Readonly non-signers:    mint [7], token_program [8], system_program [9],
///                            assoc_token_program [10], sysvar_rent [11],
///                            sysvar_recent_blockhashes [12], program_id [13]
pub struct ExecutePaymentAccounts {
    /// Client keypair pubkey — signs slot 0.
    pub payer: [u8; 32],
    /// Beacon/broadcaster pubkey — signs slot 1 via cosignTransaction.
    pub broadcaster: [u8; 32],
    /// Durable nonce account pubkey.
    pub nonce_account: [u8; 32],
    /// Payer's associated token account.
    pub payer_ata: [u8; 32],
    /// Recipient pubkey.
    pub recipient: [u8; 32],
    /// Recipient's associated token account.
    pub recipient_ata: [u8; 32],
    /// Broadcaster's associated token account (for fee).
    pub broadcaster_ata: [u8; 32],
    /// SPL token mint.
    pub mint: [u8; 32],
    /// Anchor program ID (ble_revshare contract).
    pub program_id: [u8; 32],
}

// ── Solana wire format ────────────────────────────────────────────────────────

fn encode_compact_u16(n: u16) -> Vec<u8> {
    let mut out = Vec::with_capacity(3);
    let mut v = n;
    loop {
        let low = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 { out.push(low); break; }
        out.push(low | 0x80);
    }
    out
}

struct MessageHeader {
    num_required_signatures: u8,
    num_readonly_signed_accounts: u8,
    num_readonly_unsigned_accounts: u8,
}

struct CompiledInstruction {
    program_id_index: u8,
    account_indices: Vec<u8>,
    data: Vec<u8>,
}

struct SolanaMessage {
    header: MessageHeader,
    account_keys: Vec<[u8; 32]>,
    recent_blockhash: [u8; 32],
    instructions: Vec<CompiledInstruction>,
}

impl SolanaMessage {
    fn serialize(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.push(self.header.num_required_signatures);
        out.push(self.header.num_readonly_signed_accounts);
        out.push(self.header.num_readonly_unsigned_accounts);
        out.extend_from_slice(&encode_compact_u16(self.account_keys.len() as u16));
        for key in &self.account_keys {
            out.extend_from_slice(key);
        }
        out.extend_from_slice(&self.recent_blockhash);
        out.extend_from_slice(&encode_compact_u16(self.instructions.len() as u16));
        for ix in &self.instructions {
            out.push(ix.program_id_index);
            out.extend_from_slice(&encode_compact_u16(ix.account_indices.len() as u16));
            out.extend_from_slice(&ix.account_indices);
            out.extend_from_slice(&encode_compact_u16(ix.data.len() as u16));
            out.extend_from_slice(&ix.data);
        }
        out
    }
}

struct SolanaTransaction {
    message: SolanaMessage,
    /// One slot per required signer. Zeros = unsigned slot (beacon fills slot 1).
    signatures: Vec<[u8; 64]>,
}

impl SolanaTransaction {
    fn new_unsigned(message: SolanaMessage) -> Self {
        let n = message.header.num_required_signatures as usize;
        Self { signatures: vec![[0u8; 64]; n], message }
    }

    fn sign_at(&mut self, keypair: &SigningKey, signer_index: usize) {
        let msg_bytes = self.message.serialize();
        let sig: ed25519_dalek::Signature = keypair.sign(&msg_bytes);
        if let Some(slot) = self.signatures.get_mut(signer_index) {
            slot.copy_from_slice(&sig.to_bytes());
        }
    }

    fn serialize(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&encode_compact_u16(self.signatures.len() as u16));
        for sig in &self.signatures {
            out.extend_from_slice(sig);
        }
        out.extend_from_slice(&self.message.serialize());
        out
    }

    fn to_base64(&self) -> String {
        base64::engine::general_purpose::STANDARD.encode(self.serialize())
    }
}

// ── Transaction builder ───────────────────────────────────────────────────────

/// Build and partially sign a durable-nonce execute_payment transaction.
///
/// - Signer 0 (payer/client): signed with `payer_keypair`.
/// - Signer 1 (broadcaster/beacon): slot left as zeros for beacon to fill.
///
/// `nonce_blockhash`: 32-byte nonce value from the nonce account (fetched via
/// `BeaconManager::request_account_info` and parsed from the response).
///
/// Returns base64-encoded partial tx for `BeaconManager::request_cosign_transaction`.
pub fn partial_sign_execute_payment(
    payer_keypair: &SigningKey,
    nonce_blockhash: [u8; 32],
    accounts: &ExecutePaymentAccounts,
    params: &ExecutePaymentParams,
) -> String {
    use pubkeys::*;

    // Account table — order matches IDL (see ExecutePaymentAccounts docs).
    let account_keys: Vec<[u8; 32]> = vec![
        // Signers (writable) — must come first
        accounts.payer,
        accounts.broadcaster,
        // Writable non-signers
        accounts.nonce_account,
        accounts.payer_ata,
        accounts.recipient,
        accounts.recipient_ata,
        accounts.broadcaster_ata,
        // Readonly non-signers
        accounts.mint,
        TOKEN_PROGRAM,
        SYSTEM_PROGRAM,
        ASSOCIATED_TOKEN_PROGRAM,
        SYSVAR_RENT,
        SYSVAR_RECENT_BLOCKHASHES,
        accounts.program_id,
    ];

    let idx = |key: &[u8; 32]| -> u8 {
        account_keys.iter().position(|k| k == key).expect("key in table") as u8
    };

    // Instruction 0: SystemProgram::AdvanceNonceAccount (required first for durable nonces).
    // data: u32 LE instruction variant 4
    let advance_ix = CompiledInstruction {
        program_id_index: idx(&SYSTEM_PROGRAM),
        account_indices: vec![
            idx(&accounts.nonce_account),        // nonce account (writable)
            idx(&SYSVAR_RECENT_BLOCKHASHES),      // sysvar (readonly)
            idx(&accounts.payer),                 // nonce authority (signer)
        ],
        data: vec![4, 0, 0, 0],
    };

    // Instruction 1: execute_payment on the ble_revshare Anchor program.
    let ep_ix = CompiledInstruction {
        program_id_index: idx(&accounts.program_id),
        account_indices: vec![
            idx(&accounts.payer),
            idx(&accounts.broadcaster),
            idx(&accounts.payer_ata),
            idx(&accounts.recipient),
            idx(&accounts.recipient_ata),
            idx(&accounts.broadcaster_ata),
            idx(&accounts.mint),
            idx(&TOKEN_PROGRAM),
            idx(&SYSTEM_PROGRAM),
            idx(&ASSOCIATED_TOKEN_PROGRAM),
            idx(&SYSVAR_RENT),
        ],
        data: build_execute_payment_ix_data(params).to_vec(),
    };

    let message = SolanaMessage {
        header: MessageHeader {
            num_required_signatures: 2,       // payer + broadcaster
            num_readonly_signed_accounts: 0,
            num_readonly_unsigned_accounts: 7, // mint, token_prog, sys_prog, atp, rent, recent_bh, program_id
        },
        account_keys,
        recent_blockhash: nonce_blockhash,
        instructions: vec![advance_ix, ep_ix],
    };

    let mut tx = SolanaTransaction::new_unsigned(message);
    // Sign as payer (slot 0). Broadcaster slot (1) stays zeros.
    tx.sign_at(payer_keypair, 0);
    tx.to_base64()
}

// ── Plain tx (no durable nonce) ───────────────────────────────────────────────

/// Accounts for the plain (non-durable-nonce) execute_payment flow.
/// No `nonce_account` — `broadcaster` is filled by the beacon from its own keypair.
pub struct PlainExecutePaymentAccounts {
    pub payer: [u8; 32],
    pub payer_ata: [u8; 32],
    pub recipient: [u8; 32],
    pub recipient_ata: [u8; 32],
    pub broadcaster_ata: [u8; 32],
    pub mint: [u8; 32],
    pub program_id: [u8; 32],
}

/// Build an unsigned execute_payment transaction using a plain recent blockhash.
///
/// Single instruction only (no durable nonce advance). Both signature slots zeroed:
///   slot 0 (payer): client fills via `sign_tx_at_slot`.
///   slot 1 (broadcaster): beacon fills at cosign time.
pub fn build_unsigned_execute_payment(
    payer: &[u8; 32],
    broadcaster: &[u8; 32],
    recent_blockhash: [u8; 32],
    accounts: &PlainExecutePaymentAccounts,
    params: &ExecutePaymentParams,
) -> Vec<u8> {
    use pubkeys::*;

    // 12 accounts (vs 14 with durable nonce):
    //   Signers (writable):      payer [0], broadcaster [1]
    //   Writable non-signers:    payer_ata [2], recipient [3], recipient_ata [4], broadcaster_ata [5]
    //   Readonly non-signers:    mint [6], token_program [7], system_program [8],
    //                            assoc_token_program [9], sysvar_rent [10], program_id [11]
    let account_keys: Vec<[u8; 32]> = vec![
        *payer, *broadcaster,
        accounts.payer_ata, accounts.recipient, accounts.recipient_ata, accounts.broadcaster_ata,
        accounts.mint, TOKEN_PROGRAM, SYSTEM_PROGRAM, ASSOCIATED_TOKEN_PROGRAM,
        SYSVAR_RENT, accounts.program_id,
    ];

    let idx = |key: &[u8; 32]| -> u8 {
        account_keys.iter().position(|k| k == key).expect("key in table") as u8
    };

    let ep_ix = CompiledInstruction {
        program_id_index: idx(&accounts.program_id),
        account_indices: vec![
            idx(payer), idx(broadcaster),
            idx(&accounts.payer_ata), idx(&accounts.recipient),
            idx(&accounts.recipient_ata), idx(&accounts.broadcaster_ata),
            idx(&accounts.mint), idx(&TOKEN_PROGRAM),
            idx(&SYSTEM_PROGRAM), idx(&ASSOCIATED_TOKEN_PROGRAM), idx(&SYSVAR_RENT),
        ],
        data: build_execute_payment_ix_data(params).to_vec(),
    };

    let message = SolanaMessage {
        header: MessageHeader {
            num_required_signatures: 2,
            num_readonly_signed_accounts: 0,
            num_readonly_unsigned_accounts: 6, // mint, token_prog, sys_prog, atp, rent, program_id
        },
        account_keys,
        recent_blockhash,
        instructions: vec![ep_ix],
    };

    SolanaTransaction::new_unsigned(message).serialize()
}

// ── Tx-level utilities (operate on full serialized tx bytes) ──────────────────

fn parse_tx_compact_u16(bytes: &[u8]) -> Option<(u16, usize)> {
    let mut val: u16 = 0;
    let mut shift = 0u16;
    for (i, &b) in bytes.iter().enumerate().take(3) {
        val |= ((b & 0x7f) as u16) << shift;
        shift += 7;
        if b & 0x80 == 0 { return Some((val, i + 1)); }
    }
    None
}

/// Sign a signature slot in a serialized Solana tx. Returns the modified tx bytes.
/// Slot 0 = payer, slot 1 = broadcaster/cosigner.
pub fn sign_tx_at_slot(tx_bytes: &[u8], keypair: &SigningKey, slot: usize) -> Vec<u8> {
    let Some((n, cu16_len)) = parse_tx_compact_u16(tx_bytes) else {
        return tx_bytes.to_vec();
    };
    let msg_start = cu16_len + (n as usize) * 64;
    if msg_start > tx_bytes.len() { return tx_bytes.to_vec(); }
    let sig: ed25519_dalek::Signature = keypair.sign(&tx_bytes[msg_start..]);
    let mut out = tx_bytes.to_vec();
    let off = cu16_len + slot * 64;
    if off + 64 <= out.len() {
        out[off..off + 64].copy_from_slice(&sig.to_bytes());
    }
    out
}

/// Verify that a signature slot contains a valid ed25519 signature over the tx message bytes.
/// Beacon uses this to validate payer slot 0 before cosigning slot 1.
pub fn verify_slot_signature(tx_bytes: &[u8], pubkey: &[u8; 32], slot: usize) -> bool {
    use ed25519_dalek::{VerifyingKey, Signature, Verifier};
    let Some((n, cu16_len)) = parse_tx_compact_u16(tx_bytes) else { return false; };
    if slot >= n as usize { return false; }
    let sig_off = cu16_len + slot * 64;
    let msg_start = cu16_len + (n as usize) * 64;
    if sig_off + 64 > tx_bytes.len() || msg_start > tx_bytes.len() { return false; }
    let Ok(vk) = VerifyingKey::from_bytes(pubkey) else { return false };
    let Ok(sig) = Signature::from_slice(&tx_bytes[sig_off..sig_off + 64]) else { return false };
    vk.verify(&tx_bytes[msg_start..], &sig).is_ok()
}

// ── Nonce account parsing ─────────────────────────────────────────────────────

/// Extract the nonce value (blockhash) from a `getAccountInfo` response.
///
/// `account_data_b64`: the `data[0]` field from the account info response.
/// Nonce accounts store: [version:4][state:4][authority:32][blockhash:32][fee_calculator:8]
/// Nonce value is at offset 40..72.
pub fn extract_nonce_blockhash(account_data_b64: &str) -> Option<[u8; 32]> {
    let bytes = base64::engine::general_purpose::STANDARD.decode(account_data_b64).ok()?;
    if bytes.len() < 72 { return None; }
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&bytes[40..72]);
    Some(hash)
}

// ── Test helpers ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discriminator_is_8_bytes() {
        let d = anchor_discriminator("execute_payment");
        assert_eq!(d.len(), 8);
    }

    #[test]
    fn execute_payment_ix_data_length() {
        let params = ExecutePaymentParams {
            comp_offset: 0,
            amount: 1_000_000,
            encrypted_amount: [0u8; 32],
            nonce: 42,
            encryption_pub_key: [0u8; 32],
        };
        let data = build_execute_payment_ix_data(&params);
        assert_eq!(data.len(), 104);
    }

    #[test]
    fn compact_u16_single_byte_for_small_values() {
        assert_eq!(encode_compact_u16(0), vec![0]);
        assert_eq!(encode_compact_u16(127), vec![127]);
    }

    #[test]
    fn compact_u16_two_bytes_for_128() {
        assert_eq!(encode_compact_u16(128), vec![0x80, 0x01]);
    }

    fn test_plain_accounts() -> (PlainExecutePaymentAccounts, ExecutePaymentParams) {
        let accounts = PlainExecutePaymentAccounts {
            payer:          [1u8; 32],
            payer_ata:      [3u8; 32],
            recipient:      [4u8; 32],
            recipient_ata:  [5u8; 32],
            broadcaster_ata:[6u8; 32],
            mint:           [7u8; 32],
            program_id:     [8u8; 32],
        };
        let params = ExecutePaymentParams {
            comp_offset: 0, amount: 1_000_000,
            encrypted_amount: [0u8; 32], nonce: 42,
            encryption_pub_key: [0u8; 32],
        };
        (accounts, params)
    }

    #[test]
    fn plain_tx_has_12_accounts_and_2_zero_sig_slots() {
        let (accounts, params) = test_plain_accounts();
        let payer = [1u8; 32];
        let broadcaster = [2u8; 32];
        let tx = build_unsigned_execute_payment(&payer, &broadcaster, [0u8; 32], &accounts, &params);
        // [0x02][64B zero sig0][64B zero sig1][message...]
        assert_eq!(tx[0], 0x02);                      // compact_u16(2) — two sigs
        assert_eq!(&tx[1..129], &[0u8; 128]);          // both sig slots zeros
        assert_eq!(tx[129], 2);                        // num_required_signatures = 2
        assert_eq!(tx[130], 0);                        // num_readonly_signed = 0
        assert_eq!(tx[131], 6);                        // num_readonly_unsigned = 6
        assert_eq!(tx[132], 12);                       // compact_u16(12) account count
    }

    #[test]
    fn sign_tx_at_slot_fills_payer_and_verify_passes() {
        let seed = [42u8; 32];
        let keypair = SigningKey::from_bytes(&seed);
        let payer = keypair.verifying_key().to_bytes();
        let (mut accounts, params) = test_plain_accounts();
        accounts.payer = payer;
        let broadcaster = [2u8; 32];
        let unsigned = build_unsigned_execute_payment(&payer, &broadcaster, [0u8; 32], &accounts, &params);
        let signed = sign_tx_at_slot(&unsigned, &keypair, 0);
        assert_ne!(&signed[1..65], &[0u8; 64]);        // slot 0 filled
        assert_eq!(&signed[65..129], &[0u8; 64]);      // slot 1 still zero
        assert!(verify_slot_signature(&signed, &payer, 0));
        assert!(!verify_slot_signature(&signed, &broadcaster, 1)); // zeros = bad sig
    }

    #[test]
    fn verify_slot_signature_rejects_wrong_pubkey() {
        let seed = [42u8; 32];
        let keypair = SigningKey::from_bytes(&seed);
        let payer = keypair.verifying_key().to_bytes();
        let (mut accounts, params) = test_plain_accounts();
        accounts.payer = payer;
        let broadcaster = [2u8; 32];
        let unsigned = build_unsigned_execute_payment(&payer, &broadcaster, [0u8; 32], &accounts, &params);
        let signed = sign_tx_at_slot(&unsigned, &keypair, 0);
        let wrong_key = [99u8; 32];
        assert!(!verify_slot_signature(&signed, &wrong_key, 0));
    }
}
