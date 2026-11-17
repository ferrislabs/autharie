use autharie_domain::{
    CoreError,
    organisation::ports::{PlatformRealm, PlatformUserId},
};
use reqwest::Client;
use serde::Deserialize;

use super::herald_identity::RealmAdmin;

pub enum PlatformRealmAdapter {
    FerrisKey(FerrisKeyPlatformRealm),
    Disabled,
}

impl PlatformRealmAdapter {
    pub fn from_admin(realm: Option<RealmAdmin>) -> Self {
        match realm {
            Some(realm) => Self::FerrisKey(FerrisKeyPlatformRealm::new(realm)),
            None => Self::Disabled,
        }
    }
}

impl PlatformRealm for PlatformRealmAdapter {
    async fn find_user_by_email(&self, email: &str) -> Result<Option<PlatformUserId>, CoreError> {
        match self {
            Self::FerrisKey(realm) => realm.find_user_by_email(email).await,
            Self::Disabled => Ok(Some(PlatformUserId(String::new()))),
        }
    }

    async fn create_user(&self, email: &str) -> Result<PlatformUserId, CoreError> {
        match self {
            Self::FerrisKey(realm) => realm.create_user(email).await,
            Self::Disabled => Ok(PlatformUserId(String::new())),
        }
    }

    async fn request_password_setup(&self, email: &str) -> Result<(), CoreError> {
        match self {
            Self::FerrisKey(realm) => realm.request_password_setup(email).await,
            Self::Disabled => Ok(()),
        }
    }
}

pub struct FerrisKeyPlatformRealm {
    http: Client,
    realm: RealmAdmin,
}

#[derive(Deserialize)]
struct Listed {
    data: Vec<RealmUser>,
}

#[derive(Deserialize)]
struct Created {
    data: RealmUser,
}

#[derive(Deserialize)]
struct RealmUser {
    id: String,
    email: Option<String>,
}

impl FerrisKeyPlatformRealm {
    pub fn new(realm: RealmAdmin) -> Self {
        Self {
            http: Client::new(),
            realm,
        }
    }

    fn unavailable(what: &str, detail: impl std::fmt::Display) -> CoreError {
        CoreError::IdentityProviderUnavailable {
            reason: format!("could not {what}: {detail}"),
        }
    }

    fn realm_url(&self, path: &str) -> String {
        format!("{}/realms/{}/{path}", self.realm.base_url, self.realm.realm)
    }

    async fn token(&self) -> Result<String, CoreError> {
        self.realm
            .token(&self.http)
            .await
            .map_err(|reason| CoreError::IdentityProviderUnavailable { reason })
    }
}

impl PlatformRealm for FerrisKeyPlatformRealm {
    async fn find_user_by_email(&self, email: &str) -> Result<Option<PlatformUserId>, CoreError> {
        let token = self.token().await?;

        let answer = self
            .http
            .get(self.realm_url("users"))
            .query(&[("email", email), ("limit", "100")])
            .bearer_auth(token)
            .send()
            .await
            .map_err(|e| Self::unavailable("look an account up", e))?;

        if !answer.status().is_success() {
            return Err(Self::unavailable("look an account up", answer.status()));
        }

        let listed: Listed = answer
            .json()
            .await
            .map_err(|e| Self::unavailable("read the accounts found", e))?;

        Ok(listed
            .data
            .into_iter()
            .find(|user| {
                user.email
                    .as_deref()
                    .is_some_and(|found| found.eq_ignore_ascii_case(email))
            })
            .map(|user| PlatformUserId(user.id)))
    }

    async fn create_user(&self, email: &str) -> Result<PlatformUserId, CoreError> {
        let token = self.token().await?;

        let answer = self
            .http
            .post(self.realm_url("users"))
            .bearer_auth(token)
            .json(&serde_json::json!({
                "username": email,
                "email": email,
                "email_verified": false,
            }))
            .send()
            .await
            .map_err(|e| Self::unavailable("create an account", e))?;

        if !answer.status().is_success() {
            return Err(Self::unavailable("create an account", answer.status()));
        }

        let created: Created = answer
            .json()
            .await
            .map_err(|e| Self::unavailable("read the account just created", e))?;

        Ok(PlatformUserId(created.data.id))
    }

    async fn request_password_setup(&self, email: &str) -> Result<(), CoreError> {
        let answer = self
            .http
            .post(self.realm_url("login-actions/forgot-password"))
            .json(&serde_json::json!({ "email": email }))
            .send()
            .await
            .map_err(|e| Self::unavailable("request the set-password email", e))?;

        if !answer.status().is_success() {
            return Err(Self::unavailable(
                "request the set-password email",
                answer.status(),
            ));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests;
