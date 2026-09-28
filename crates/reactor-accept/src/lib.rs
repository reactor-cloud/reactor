pub fn enabled() -> bool {
    std::env::var("REACTOR_ACCEPT").ok().as_deref() == Some("1")
}
