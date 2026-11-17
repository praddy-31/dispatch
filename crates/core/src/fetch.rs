//! Downloading. Behind a trait so tests feed saved files and never touch the network.

use std::time::Duration;

pub trait Fetch: Sync {
    fn get(&self, url: &str) -> Result<Vec<u8>, String>;

    /// The same, asking for a particular format (doi.org answers BibTeX or RIS this way).
    fn get_as(&self, url: &str, accept: &str) -> Result<Vec<u8>, String> {
        let _ = accept;
        self.get(url)
    }

    /// For slow services (arXiv's search API often takes 20–60 s): a longer time limit.
    fn get_patient(&self, url: &str) -> Result<Vec<u8>, String> {
        self.get(url)
    }

    /// With one extra header (Semantic Scholar's `x-api-key`).
    fn get_with(&self, url: &str, header: (&str, &str)) -> Result<Vec<u8>, String> {
        let _ = header;
        self.get(url)
    }
}

pub struct Http {
    client: reqwest::blocking::Client,
}

pub const USER_AGENT: &str = concat!(
    "Dispatch/",
    env!("CARGO_PKG_VERSION"),
    " (personal feed reader; each feed at most hourly, arXiv daily)"
);

impl Http {
    pub fn new() -> Result<Http, String> {
        let client = reqwest::blocking::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(25))
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Http { client })
    }
}

impl Fetch for Http {
    fn get(&self, url: &str) -> Result<Vec<u8>, String> {
        self.send(url, self.client.get(url))
    }

    fn get_as(&self, url: &str, accept: &str) -> Result<Vec<u8>, String> {
        self.send(
            url,
            self.client.get(url).header(reqwest::header::ACCEPT, accept),
        )
    }

    fn get_patient(&self, url: &str) -> Result<Vec<u8>, String> {
        self.send(url, self.client.get(url).timeout(Duration::from_secs(90)))
    }

    fn get_with(&self, url: &str, header: (&str, &str)) -> Result<Vec<u8>, String> {
        self.send(url, self.client.get(url).header(header.0, header.1))
    }
}

impl Http {
    /// Sends and counts the request in the network log.
    fn send(&self, url: &str, req: reqwest::blocking::RequestBuilder) -> Result<Vec<u8>, String> {
        let result = Self::answer(req);
        crate::netlog::record(url, result.is_ok());
        result
    }

    fn answer(req: reqwest::blocking::RequestBuilder) -> Result<Vec<u8>, String> {
        let resp = req.send().map_err(|e| {
            if e.is_timeout() {
                "timed out".to_owned()
            } else if e.is_connect() {
                "couldn't connect (offline?)".to_owned()
            } else {
                e.to_string()
            }
        })?;
        let status = resp.status();
        if !status.is_success() {
            return Err(format!("HTTP {}", status.as_u16()));
        }
        resp.bytes().map(|b| b.to_vec()).map_err(|e| e.to_string())
    }
}
