use anyhow::anyhow;
use lettre::message::header::ContentType;
use lettre::message::{Mailbox, Message, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{SmtpTransport, Transport};
use std::time::Duration;

pub struct Template {
    pub name: &'static str,
    pub subject: &'static str,
    pub body_text: &'static str,
    pub body_html: &'static str,
}

pub fn reserved() -> &'static [Template] {
    &[
        Template {
            name: "magic_link",
            subject: "Sign in",
            body_text: "Sign in as {{email}}.\n\n{{link}}\n",
            body_html: "<p>Sign in as {{email}}.</p><p><a href=\"{{link}}\">{{link}}</a></p>",
        },
        Template {
            name: "recovery",
            subject: "Reset your password",
            body_text: "Reset the password for {{email}}.\n\n{{link}}\n",
            body_html:
                "<p>Reset the password for {{email}}.</p><p><a href=\"{{link}}\">{{link}}</a></p>",
        },
        Template {
            name: "invite",
            subject: "You are invited",
            body_text: "Accept the invite for {{email}}.\n\n{{link}}\n",
            body_html:
                "<p>Accept the invite for {{email}}.</p><p><a href=\"{{link}}\">{{link}}</a></p>",
        },
        Template {
            name: "confirm_email",
            subject: "Confirm your email",
            body_text: "Confirm {{email}}.\n\nCode: {{code}}\n\n{{link}}\n",
            body_html:
                "<p>Confirm {{email}}.</p><p>Code: {{code}}</p><p><a href=\"{{link}}\">{{link}}</a></p>",
        },
        Template {
            name: "otp",
            subject: "Your sign-in code",
            body_text: "Sign in as {{email}}.\n\nCode: {{code}}\n\nThe code expires in 10 minutes.\n",
            body_html:
                "<p>Sign in as {{email}}.</p><p>Code: {{code}}</p><p>The code expires in 10 minutes.</p>",
        },
    ]
}

pub fn reserved_name(name: &str) -> bool {
    reserved().iter().any(|template| template.name == name)
}

pub fn render(body: &str, email: &str, token: &str, link: &str) -> String {
    fill(body, email, token, link, "")
}

pub fn fill(body: &str, email: &str, token: &str, link: &str, code: &str) -> String {
    body.replace("{{email}}", email)
        .replace("{{token}}", token)
        .replace("{{link}}", link)
        .replace("{{code}}", code)
}

pub fn auth_link(base: &str, token: &str) -> String {
    let base = base.trim();
    if base.is_empty() {
        return token.to_string();
    }
    let sep = if base.contains('?') { '&' } else { '?' };
    format!("{base}{sep}token={token}")
}

#[derive(Clone)]
pub struct Mail {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub from: String,
    pub tls: String,
}

pub fn send(mail: &Mail, to: &str, subject: &str, text: &str, html: &str) -> anyhow::Result<()> {
    let from: Mailbox = mail
        .from
        .parse()
        .map_err(|err: lettre::address::AddressError| anyhow!(err.to_string()))?;
    let to: Mailbox = to
        .parse()
        .map_err(|err: lettre::address::AddressError| anyhow!(err.to_string()))?;
    let message = Message::builder()
        .from(from)
        .to(to)
        .subject(subject)
        .multipart(
            MultiPart::alternative()
                .singlepart(
                    SinglePart::builder()
                        .header(ContentType::TEXT_PLAIN)
                        .body(text.to_string()),
                )
                .singlepart(
                    SinglePart::builder()
                        .header(ContentType::TEXT_HTML)
                        .body(html.to_string()),
                ),
        )?;
    let mut builder = match mail.tls.as_str() {
        "tls" => SmtpTransport::relay(&mail.host)?.port(mail.port),
        "none" => SmtpTransport::builder_dangerous(&mail.host).port(mail.port),
        _ => SmtpTransport::starttls_relay(&mail.host)?.port(mail.port),
    };
    builder = builder.timeout(Some(Duration::from_secs(10)));
    if !mail.username.is_empty() {
        builder = builder.credentials(Credentials::new(
            mail.username.clone(),
            mail.password.clone(),
        ));
    }
    builder.build().send(&message)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{auth_link, fill, render};

    #[test]
    fn fills_the_link_and_the_token() {
        let body = render(
            "Hi {{email}} {{link}} {{token}}",
            "a@b.co",
            "tok",
            "https://app/?token=tok",
        );
        assert_eq!(body, "Hi a@b.co https://app/?token=tok tok");
        assert_eq!(
            fill(
                "Code {{code}} {{link}}",
                "a@b.co",
                "tok",
                "https://app/?token=tok",
                "123456"
            ),
            "Code 123456 https://app/?token=tok"
        );
        assert_eq!(
            auth_link("https://app/cb", "tok"),
            "https://app/cb?token=tok"
        );
        assert_eq!(
            auth_link("https://app/cb?next=1", "tok"),
            "https://app/cb?next=1&token=tok"
        );
        assert_eq!(auth_link("", "tok"), "tok");
    }
}
