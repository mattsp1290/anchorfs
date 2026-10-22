use crate::error::ClientError;
use url::Url;

#[derive(Debug, Clone)]
pub struct Endpoint(Url);
impl Endpoint {
    pub fn parse(value: &str, allow_plaintext: bool) -> Result<Self, ClientError> {
        let url = Url::parse(value)
            .map_err(|_| ClientError::InvalidInput("invalid endpoint URL".into()))?;
        if url.scheme() == "http" && !allow_plaintext {
            return Err(ClientError::InvalidInput(
                "plaintext endpoint requires --allow-plaintext".into(),
            ));
        }
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path() != "/"
        {
            return Err(ClientError::InvalidInput(
                "endpoint must be an http(s) origin without credentials, path, query or fragment"
                    .into(),
            ));
        }
        Ok(Self(url))
    }
    pub(crate) fn is_https(&self) -> bool {
        self.0.scheme() == "https"
    }
    pub(crate) fn route(&self, path: &str) -> Url {
        self.0.join(path).expect("internal absolute API route")
    }
}
