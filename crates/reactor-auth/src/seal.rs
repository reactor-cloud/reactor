use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;

pub fn seal(key: &[u8; 32], plain: &[u8]) -> anyhow::Result<String> {
    let cipher = Aes256Gcm::new_from_slice(key)?;
    let mut nonce = [0u8; 12];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut nonce);
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce), plain)
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    let mut packed = nonce.to_vec();
    packed.extend(ciphertext);
    Ok(URL_SAFE_NO_PAD.encode(packed))
}

pub fn unseal(key: &[u8; 32], packed: &str) -> anyhow::Result<Vec<u8>> {
    let bytes = URL_SAFE_NO_PAD
        .decode(packed)
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    if bytes.len() < 12 {
        anyhow::bail!("sealed value is too short");
    }
    let (nonce, ciphertext) = bytes.split_at(12);
    let cipher = Aes256Gcm::new_from_slice(key)?;
    cipher
        .decrypt(Nonce::from_slice(nonce), ciphertext)
        .map_err(|err| anyhow::anyhow!(err.to_string()))
}

#[cfg(test)]
mod tests {
    use super::{seal, unseal};

    #[test]
    fn round_trip() {
        let key = [7u8; 32];
        let sealed = seal(&key, b"smtp-secret").unwrap();
        assert_eq!(unseal(&key, &sealed).unwrap(), b"smtp-secret");
    }
}
