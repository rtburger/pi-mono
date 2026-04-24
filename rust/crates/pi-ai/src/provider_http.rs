use reqwest::Client;
use std::sync::OnceLock;

pub(crate) fn shared_http_client() -> &'static Client {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    CLIENT.get_or_init(Client::new)
}

#[cfg(test)]
mod tests {
    use super::shared_http_client;

    #[test]
    fn shared_http_client_returns_same_instance() {
        assert!(std::ptr::eq(shared_http_client(), shared_http_client()));
    }
}
