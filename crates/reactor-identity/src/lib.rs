use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Anon,
    Authenticated,
    Service,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Anon => "anon",
            Role::Authenticated => "authenticated",
            Role::Service => "service",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "anon" => Some(Role::Anon),
            "authenticated" => Some(Role::Authenticated),
            "service" => Some(Role::Service),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProjectRef(String);

impl ProjectRef {
    pub fn parse(value: &str) -> Result<Self, IdentityError> {
        if value.len() == 20
            && value
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        {
            Ok(Self(value.to_string()))
        } else {
            Err(IdentityError::InvalidRef)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn schema(&self) -> String {
        format!("proj_{}", self.0)
    }
}

impl std::fmt::Display for ProjectRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Identity {
    pub project_id: Uuid,
    pub project_ref: ProjectRef,
    pub user_id: Option<Uuid>,
    pub role: Role,
    pub claims: serde_json::Value,
}

#[derive(Debug, thiserror::Error)]
pub enum IdentityError {
    #[error("invalid project ref")]
    InvalidRef,
    #[error("unauthorized")]
    Unauthorized,
    #[error("host and token project mismatch")]
    Mismatch,
}

pub fn project_ref_from_host(host: &str, base_domain: &str) -> Option<ProjectRef> {
    let host = host.split(':').next()?.trim();
    let suffix = format!(".{base_domain}");
    let label = host.strip_suffix(&suffix)?;
    if label.is_empty() || label.contains('.') {
        return None;
    }
    ProjectRef::parse(label).ok()
}

pub fn is_api_host(host: &str) -> bool {
    let host = host.split(':').next().unwrap_or(host);
    matches!(host, "localhost" | "127.0.0.1" | "reactor" | "0.0.0.0")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ref_and_host() {
        let r = ProjectRef::parse("abcdefghij0123456789").unwrap();
        assert_eq!(r.schema(), "proj_abcdefghij0123456789");
        assert!(ProjectRef::parse("ABC").is_err());
        let host = format!("{r}.apps.localhost:18000");
        assert_eq!(project_ref_from_host(&host, "apps.localhost").unwrap(), r);
        assert!(project_ref_from_host("localhost:18000", "apps.localhost").is_none());
    }
}
