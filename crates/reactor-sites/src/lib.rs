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

pub fn decode_txt_rdata(data: &str) -> String {
    let data = data.trim();
    if !data.contains('"') {
        return data.to_string();
    }
    let bytes = data.as_bytes();
    let mut index = 0;
    let mut out = Vec::new();
    while index < bytes.len() {
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if index >= bytes.len() {
            break;
        }
        if bytes[index] != b'"' {
            while index < bytes.len() && !bytes[index].is_ascii_whitespace() {
                out.push(bytes[index]);
                index += 1;
            }
            continue;
        }
        index += 1;
        while index < bytes.len() {
            match bytes[index] {
                b'"' => {
                    index += 1;
                    break;
                }
                b'\\' if index + 1 < bytes.len() => {
                    let next = bytes[index + 1];
                    if (b'0'..=b'7').contains(&next)
                        && index + 3 < bytes.len()
                        && (b'0'..=b'7').contains(&bytes[index + 2])
                        && (b'0'..=b'7').contains(&bytes[index + 3])
                    {
                        let value = (next - b'0') * 64
                            + (bytes[index + 2] - b'0') * 8
                            + (bytes[index + 3] - b'0');
                        out.push(value);
                        index += 4;
                    } else {
                        out.push(next);
                        index += 2;
                    }
                }
                other => {
                    out.push(other);
                    index += 1;
                }
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn txt_from_doh(body: &str) -> Option<Vec<String>> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let Some(answers) = value.get("Answer").and_then(|item| item.as_array()) else {
        return Some(Vec::new());
    };
    Some(
        answers
            .iter()
            .filter_map(|answer| {
                if answer.get("type").and_then(|item| item.as_u64())? != 16 {
                    return None;
                }
                let data = answer.get("data").and_then(|item| item.as_str())?;
                let text = decode_txt_rdata(data);
                if text.is_empty() {
                    None
                } else {
                    Some(text)
                }
            })
            .collect(),
    )
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

    #[test]
    fn txt_rdata_drops_presentation_quotes_and_joins_chunks() {
        assert_eq!(
            decode_txt_rdata("reactor-site-verification=abc"),
            "reactor-site-verification=abc"
        );
        assert_eq!(
            decode_txt_rdata("\"reactor-site-verification=abc\""),
            "reactor-site-verification=abc"
        );
        assert_eq!(
            decode_txt_rdata("\"reactor-site-\" \"verification=abc\""),
            "reactor-site-verification=abc"
        );
        assert_eq!(decode_txt_rdata("\"say \\\"hi\\\"\""), "say \"hi\"");
    }

    #[test]
    fn doh_json_keeps_txt_answers_only() {
        let body = r#"{"Answer":[
            {"type":5,"data":"canonical.example."},
            {"type":16,"data":"\"reactor-site-verification=abc\""}
        ]}"#;
        assert_eq!(
            txt_from_doh(body),
            Some(vec!["reactor-site-verification=abc".into()])
        );
        assert_eq!(txt_from_doh("{}"), Some(vec![]));
        assert_eq!(txt_from_doh("not-json"), None);
        assert!(txt_matches(txt_from_doh(body).unwrap().as_slice(), "abc"));
    }
}
