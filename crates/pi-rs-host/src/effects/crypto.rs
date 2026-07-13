use ring::rand::{SecureRandom as _, SystemRandom};
use sha2::{Digest as _, Sha256};

use super::{CryptoRequest, EffectError, EffectResult};

pub async fn execute(request: CryptoRequest) -> Result<EffectResult, EffectError> {
    match request {
        CryptoRequest::Sha256(bytes) => {
            if bytes.len() > 1024 * 1024 {
                return Err(EffectError::Invalid(
                    "SHA-256 input must not exceed 1048576 bytes".to_owned(),
                ));
            }
            Ok(EffectResult::Crypto(Sha256::digest(bytes).to_vec()))
        }
        CryptoRequest::RandomBytes(length) => {
            if length == 0 || length > 1024 * 1024 {
                return Err(EffectError::Invalid(
                    "random byte length must be in 1..=1048576".to_owned(),
                ));
            }
            let mut bytes = vec![0_u8; length];
            SystemRandom::new()
                .fill(&mut bytes)
                .map_err(|_| EffectError::Io("operating-system random source failed".to_owned()))?;
            Ok(EffectResult::Crypto(bytes))
        }
        CryptoRequest::RandomUuid => {
            let mut bytes = [0_u8; 16];
            SystemRandom::new()
                .fill(&mut bytes)
                .map_err(|_| EffectError::Io("operating-system random source failed".to_owned()))?;
            bytes[6] = (bytes[6] & 0x0f) | 0x40;
            bytes[8] = (bytes[8] & 0x3f) | 0x80;
            Ok(EffectResult::Uuid(format!(
                "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
                bytes[0],
                bytes[1],
                bytes[2],
                bytes[3],
                bytes[4],
                bytes[5],
                bytes[6],
                bytes[7],
                bytes[8],
                bytes[9],
                bytes[10],
                bytes[11],
                bytes[12],
                bytes[13],
                bytes[14],
                bytes[15]
            )))
        }
    }
}
