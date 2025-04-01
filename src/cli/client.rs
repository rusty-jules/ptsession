use std::ops::Deref;

#[derive(Clone)]
pub struct HttpClient {
    client: reqwest::Client,
    oci: oci_client::Client,
}

impl HttpClient {
    pub fn new(oci_client: &oci_client::Client) -> Self {
        let client = reqwest::Client::builder()
            //.connection_verbose(true) // Enables detailed connection info for debugging
            .pool_idle_timeout(Some(std::time::Duration::from_secs(300))) // Keep connections alive
            .tcp_keepalive(Some(std::time::Duration::from_secs(60)))
            .pool_max_idle_per_host(32) // Allow plenty of idle connections
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        Self {
            client,
            oci: oci_client.clone(),
        }
    }

    pub async fn post(&self, url: &str) -> reqwest::RequestBuilder {
        self.client.post(url)
    }

    pub async fn patch(&self, url: &str) -> reqwest::RequestBuilder {
        self.client.patch(url)
    }

    pub async fn put(&self, url: reqwest::Url) -> reqwest::RequestBuilder {
        self.client.put(url)
    }
}

impl Deref for HttpClient {
    type Target = oci_client::Client;

    fn deref(&self) -> &Self::Target {
        &self.oci
    }
}
