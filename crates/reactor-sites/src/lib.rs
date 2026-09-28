pub fn content_type_for(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("txt") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

pub fn safe_site_path(path: &str) -> Option<String> {
    let path = path.trim_start_matches('/');
    if path.is_empty() {
        return Some("index.html".into());
    }
    if path.contains("..") || path.contains('\\') || path.starts_with('/') {
        return None;
    }
    Some(path.to_string())
}

pub fn verification_name(host: &str) -> String {
    format!("_reactor-verify.{host}")
}

pub fn expected_txt(token: &str) -> String {
    format!("reactor-site-verification={token}")
}

pub fn txt_matches(records: &[String], token: &str) -> bool {
    let expect = expected_txt(token);
    records.iter().any(|r| r.trim() == expect)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_and_txt() {
        assert_eq!(safe_site_path("/").as_deref(), Some("index.html"));
        assert!(safe_site_path("/../secret").is_none());
        assert!(txt_matches(
            &["reactor-site-verification=abc".into()],
            "abc"
        ));
        assert!(!txt_matches(&["nope".into()], "abc"));
    }
}
