use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use ed25519_dalek::pkcs8::{DecodePrivateKey, EncodePrivateKey, EncodePublicKey};
use ed25519_dalek::{SigningKey, VerifyingKey};
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use pkcs8::LineEnding;
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone)]
pub struct JwtIssuer {
    encoding: EncodingKey,
    decoding: DecodingKey,
    seal_key: [u8; 32],
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TokenClaims {
    pub sub: String,
    #[serde(rename = "ref")]
    pub pref: String,
    pub role: String,
    #[serde(default)]
    pub aud: String,
    pub exp: usize,
    pub iat: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jti: Option<String>,
}

impl JwtIssuer {
    pub fn generate(dir: &Path) -> anyhow::Result<Self> {
        fs::create_dir_all(dir)?;
        let signing = SigningKey::generate(&mut OsRng);
        let pem = signing.to_pkcs8_pem(LineEnding::LF)?;
        fs::write(dir.join("private.pem"), pem.as_str())?;
        let public_pem = signing.verifying_key().to_public_key_pem(LineEnding::LF)?;
        fs::write(dir.join("public.pem"), &public_pem)?;
        let x = URL_SAFE_NO_PAD.encode(signing.verifying_key().as_bytes());
        let jwk = serde_json::json!({
            "kty": "OKP",
            "crv": "Ed25519",
            "x": x,
            "alg": "EdDSA"
        });
        fs::write(dir.join("public.jwk"), serde_json::to_string(&jwk)?)?;
        Self::from_pem(pem.as_bytes(), public_pem.as_bytes())
    }

    pub fn from_private_pem_file(path: &Path) -> anyhow::Result<Self> {
        let pem = fs::read(path)?;
        let signing = SigningKey::from_pkcs8_pem(std::str::from_utf8(&pem)?)?;
        let public_pem = signing.verifying_key().to_public_key_pem(LineEnding::LF)?;
        if let Some(dir) = path.parent() {
            let jwk_path = dir.join("public.jwk");
            if !jwk_path.exists() {
                let x = URL_SAFE_NO_PAD.encode(signing.verifying_key().as_bytes());
                let jwk = serde_json::json!({"kty":"OKP","crv":"Ed25519","x":x,"alg":"EdDSA"});
                fs::write(jwk_path, serde_json::to_string(&jwk)?)?;
            }
            let _ = fs::write(dir.join("public.pem"), &public_pem);
        }
        Self::from_pem(&pem, public_pem.as_bytes())
    }

    fn from_pem(private_pem: &[u8], public_pem: &[u8]) -> anyhow::Result<Self> {
        let digest = Sha256::digest(private_pem);
        let mut seal_key = [0u8; 32];
        seal_key.copy_from_slice(&digest);
        Ok(Self {
            encoding: EncodingKey::from_ed_pem(private_pem)?,
            decoding: DecodingKey::from_ed_pem(public_pem)?,
            seal_key,
        })
    }

    pub fn seal_key(&self) -> &[u8; 32] {
        &self.seal_key
    }

    pub fn sign(&self, pref: &str, sub: &str, role: &str, ttl_secs: u64) -> anyhow::Result<String> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as usize;
        let claims = TokenClaims {
            sub: sub.to_string(),
            pref: pref.to_string(),
            role: role.to_string(),
            aud: String::new(),
            iat: now,
            exp: now + ttl_secs as usize,
            jti: None,
        };
        let header = Header::new(Algorithm::EdDSA);
        Ok(encode(&header, &claims, &self.encoding)?)
    }

    pub fn sign_console(&self, sub: &str, ttl_secs: u64) -> anyhow::Result<String> {
        self.sign_audience(sub, "console", ttl_secs)
    }

    pub fn sign_audience(&self, sub: &str, aud: &str, ttl_secs: u64) -> anyhow::Result<String> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as usize;
        let claims = TokenClaims {
            sub: sub.to_string(),
            pref: String::new(),
            role: aud.to_string(),
            aud: aud.to_string(),
            iat: now,
            exp: now + ttl_secs as usize,
            jti: None,
        };
        let header = Header::new(Algorithm::EdDSA);
        Ok(encode(&header, &claims, &self.encoding)?)
    }

    pub fn sign_audience_unique(
        &self,
        sub: &str,
        aud: &str,
        ttl_secs: u64,
    ) -> anyhow::Result<String> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as usize;
        let claims = TokenClaims {
            sub: sub.to_string(),
            pref: String::new(),
            role: aud.to_string(),
            aud: aud.to_string(),
            iat: now,
            exp: now + ttl_secs as usize,
            jti: Some(random_token()),
        };
        let header = Header::new(Algorithm::EdDSA);
        Ok(encode(&header, &claims, &self.encoding)?)
    }

    pub fn decode(&self, token: &str) -> anyhow::Result<TokenClaims> {
        let mut validation = Validation::new(Algorithm::EdDSA);
        validation.set_required_spec_claims(&["exp"]);
        validation.validate_exp = true;
        validation.validate_aud = false;
        let data = decode::<TokenClaims>(token, &self.decoding, &validation)?;
        Ok(data.claims)
    }
}

pub fn token_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

pub fn random_token() -> String {
    let bytes: [u8; 32] = rand::random();
    URL_SAFE_NO_PAD.encode(bytes)
}

// Keep the verifying key type referenced so PEM helpers stay honest.
#[allow(dead_code)]
fn _vk(signing: &SigningKey) -> VerifyingKey {
    signing.verifying_key()
}
