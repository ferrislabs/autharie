use autharie_domain::dataplane::credential::{CredentialError, SecretString};
use reqwest::{
    Client, RequestBuilder, Response,
    header::{HeaderName, HeaderValue},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::{
    config::ScalewayConfig,
    error::{ApiError, ScalewayError},
};

const AUTH_HEADER: HeaderName = HeaderName::from_static("x-auth-token");

pub(crate) struct Http {
    client: Client,
    base_url: String,
}

impl Http {
    pub(crate) fn new(config: &ScalewayConfig) -> Result<Self, ScalewayError> {
        let client = Client::builder()
            .timeout(config.request_timeout)
            .connect_timeout(config.connect_timeout)
            .build()
            .map_err(|error| ScalewayError::Client(error.to_string()))?;

        Ok(Self {
            client,
            base_url: config.base_url.trim_end_matches('/').to_string(),
        })
    }
}

#[derive(Deserialize)]
struct RawCredential {
    access_key: String,
    secret_key: String,
    project_id: String,
    #[serde(default)]
    organization_id: Option<String>,
}

pub(crate) struct Session<'a> {
    http: &'a Http,
    token: HeaderValue,
    pub(crate) access_key: String,
    pub(crate) project_id: String,
    pub(crate) organization_id: Option<String>,
}

impl<'a> Session<'a> {
    pub(crate) fn open(http: &'a Http, secret: &SecretString) -> Result<Self, CredentialError> {
        let raw: RawCredential =
            serde_json::from_str(secret.expose()).map_err(|_| CredentialError::Invalid)?;
        let secret_key = SecretString::new(raw.secret_key);
        let mut token =
            HeaderValue::from_str(secret_key.expose()).map_err(|_| CredentialError::Invalid)?;
        token.set_sensitive(true);

        if raw.access_key.is_empty() || raw.project_id.is_empty() {
            return Err(CredentialError::Invalid);
        }

        Ok(Self {
            http,
            token,
            access_key: raw.access_key,
            project_id: raw.project_id,
            organization_id: raw.organization_id.filter(|id| !id.is_empty()),
        })
    }

    pub(crate) async fn get<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<T, ApiError> {
        let request = self.request(self.http.client.get(self.url(path)).query(query));
        Self::decode(self.execute(request).await?).await
    }

    pub(crate) async fn get_text(
        &self,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<String, ApiError> {
        let request = self.request(self.http.client.get(self.url(path)).query(query));
        self.execute(request)
            .await?
            .text()
            .await
            .map_err(|error| ApiError::Decode(error.without_url().to_string()))
    }

    pub(crate) async fn post<T: DeserializeOwned, B: Serialize>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, ApiError> {
        let request = self.request(self.http.client.post(self.url(path)).json(body));
        Self::decode(self.execute(request).await?).await
    }

    pub(crate) async fn patch<T: DeserializeOwned, B: Serialize>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, ApiError> {
        let request = self.request(self.http.client.patch(self.url(path)).json(body));
        Self::decode(self.execute(request).await?).await
    }

    pub(crate) async fn delete(&self, path: &str, query: &[(&str, &str)]) -> Result<(), ApiError> {
        let request = self.request(self.http.client.delete(self.url(path)).query(query));
        self.execute(request).await.map(|_| ())
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.http.base_url, path)
    }

    fn request(&self, builder: RequestBuilder) -> RequestBuilder {
        builder.header(AUTH_HEADER, self.token.clone())
    }

    async fn execute(&self, request: RequestBuilder) -> Result<Response, ApiError> {
        let response = request
            .send()
            .await
            .map_err(|error| ApiError::Transport(error.without_url().to_string()))?;

        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }

        let body = response.text().await.unwrap_or_default();
        let parsed: ErrorBody = serde_json::from_str(&body).unwrap_or_default();
        Err(ApiError::Status {
            status: status.as_u16(),
            kind: parsed.kind,
            message: if parsed.message.is_empty() {
                status.to_string()
            } else {
                parsed.message
            },
        })
    }

    async fn decode<T: DeserializeOwned>(response: Response) -> Result<T, ApiError> {
        response
            .json()
            .await
            .map_err(|error| ApiError::Decode(error.without_url().to_string()))
    }
}

#[derive(Default, Deserialize)]
struct ErrorBody {
    #[serde(default, rename = "type")]
    kind: String,
    #[serde(default)]
    message: String,
}
