//! Monero address validation: Monero-flavoured base58 (8-byte blocks → 11 chars) + Keccak-256
//! checksum + network byte. Format-only checks live in `config::validate_wallet`; this is the
//! strict check every user-entered wallet must pass (a typo would otherwise pay an unspendable address).
use anyhow::{anyhow, bail, Result};
use serde::Serialize;
use sha3::{Digest, Keccak256};

/// Monero base58's alphabet; `config::validate_wallet` uses it too.
pub(crate) const ALPHABET: &[u8; 58] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
const FULL_BLOCK: usize = 8;
const FULL_ENCODED: usize = 11;
/// Encoded length for a trailing block of 0..=8 decoded bytes.
const ENCODED_SIZES: [usize; 9] = [0, 2, 3, 5, 6, 7, 9, 10, 11];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AddressKind {
    Primary,
    Subaddress,
}

fn decode_block(chunk: &[u8], out_len: usize) -> Option<Vec<u8>> {
    let mut num: u128 = 0;
    for &c in chunk {
        let d = ALPHABET.iter().position(|&a| a == c)? as u128;
        num = num.checked_mul(58)?.checked_add(d)?;
    }
    if num >> (8 * out_len) != 0 {
        return None; // value does not fit the block: invalid encoding
    }
    Some(num.to_be_bytes()[16 - out_len..].to_vec())
}

pub fn monero_base58_decode(s: &str) -> Option<Vec<u8>> {
    let b = s.as_bytes();
    let (full, rem) = (b.len() / FULL_ENCODED, b.len() % FULL_ENCODED);
    let rem_len = ENCODED_SIZES.iter().position(|&e| e == rem)?;
    let mut out = Vec::with_capacity(full * FULL_BLOCK + rem_len);
    for i in 0..full {
        out.extend(decode_block(&b[i * FULL_ENCODED..(i + 1) * FULL_ENCODED], FULL_BLOCK)?);
    }
    if rem > 0 {
        out.extend(decode_block(&b[full * FULL_ENCODED..], rem_len)?);
    }
    Some(out)
}

pub fn check_address(addr: &str) -> Result<AddressKind> {
    crate::config::validate_wallet(addr)?;
    let raw = monero_base58_decode(addr).ok_or_else(|| anyhow!("not valid Monero base58"))?;
    if raw.len() != 69 {
        bail!("not a standard Monero address (decoded {} bytes)", raw.len());
    }
    let (body, sum) = raw.split_at(65);
    if Keccak256::digest(body)[..4] != *sum {
        bail!("checksum mismatch: a character is wrong, please check the address");
    }
    match raw[0] {
        18 => Ok(AddressKind::Primary),
        42 => Ok(AddressKind::Subaddress),
        n => bail!("not a Monero mainnet address (network byte {n})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DEFAULT_WALLET;

    #[test]
    fn accepts_default_wallet() {
        assert_eq!(check_address(DEFAULT_WALLET).unwrap(), AddressKind::Primary);
    }

    #[test]
    fn decodes_to_69_bytes_with_mainnet_byte() {
        let raw = monero_base58_decode(DEFAULT_WALLET).unwrap();
        assert_eq!(raw.len(), 69);
        assert_eq!(raw[0], 18);
    }

    #[test]
    fn rejects_single_char_typo() {
        // flip one character in the middle to another valid base58 character
        let mut chars: Vec<char> = DEFAULT_WALLET.chars().collect();
        chars[40] = if chars[40] == 'a' { 'b' } else { 'a' };
        let typo: String = chars.into_iter().collect();
        let e = check_address(&typo).unwrap_err().to_string();
        assert!(e.contains("checksum") || e.contains("base58"), "{e}");
    }

    #[test]
    fn rejects_synthetic_format_only_address() {
        let synthetic = format!("4Q{}", "A".repeat(93));
        assert!(check_address(&synthetic).is_err());
    }

    #[test]
    fn rejects_bad_format_before_decoding() {
        assert!(check_address("4short").is_err());
        assert!(check_address("").is_err());
    }
}
