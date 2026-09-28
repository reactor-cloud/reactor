mod jwt;
mod password;
mod seal;

pub use jwt::{random_token, token_hash, JwtIssuer};
pub use password::{hash_password, verify_password};
pub use reactor_identity::{Identity, IdentityError, ProjectRef, Role};
pub use seal::{seal, unseal};

use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("unauthorized")]
    Unauthorized,
    #[error("forbidden")]
    Forbidden,
    #[error(transparent)]
    Identity(#[from] IdentityError),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

#[async_trait]
pub trait IdentityProvider: Send + Sync {
    async fn resolve(&self, bearer: &str) -> Result<Identity, AuthError>;
}

pub struct InternalIdentity {
    pub pool: PgPool,
    pub issuer: JwtIssuer,
}

#[async_trait]
impl IdentityProvider for InternalIdentity {
    async fn resolve(&self, bearer: &str) -> Result<Identity, AuthError> {
        let claims = self
            .issuer
            .decode(bearer)
            .map_err(|_| AuthError::Unauthorized)?;
        let pref = ProjectRef::parse(&claims.pref).map_err(|_| AuthError::Unauthorized)?;
        let role = Role::parse(&claims.role).ok_or(AuthError::Unauthorized)?;
        let row = sqlx::query_as::<_, (Uuid,)>("SELECT id FROM reactor.projects WHERE ref = $1")
            .bind(pref.as_str())
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| AuthError::Other(e.into()))?
            .ok_or(AuthError::Unauthorized)?;

        if matches!(role, Role::Anon | Role::Service) {
            let hash = token_hash(bearer);
            let found = sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM reactor.api_keys WHERE project_id = $1 AND token_hash = $2 AND role = $3",
            )
            .bind(row.0)
            .bind(&hash)
            .bind(role.as_str())
            .fetch_one(&self.pool)
            .await
            .map_err(|e| AuthError::Other(e.into()))?;
            if found == 0 {
                return Err(AuthError::Unauthorized);
            }
        }

        let user_id = if role == Role::Authenticated {
            Some(Uuid::parse_str(&claims.sub).map_err(|_| AuthError::Unauthorized)?)
        } else {
            None
        };

        Ok(Identity {
            project_id: row.0,
            project_ref: pref,
            user_id,
            role,
            claims: serde_json::json!({
                "sub": claims.sub,
                "ref": claims.pref,
                "role": claims.role,
            }),
        })
    }
}

pub fn constant_time_eq(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    subtle::ConstantTimeEq::ct_eq(a, b).into()
}
