use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{Arc, Mutex},
    thread,
};

use autharie_domain::{
    CoreError,
    organisation::ports::{PlatformRealm, PlatformUserId},
};

use super::{FerrisKeyPlatformRealm, PlatformRealmAdapter};
use crate::infrastructure::herald_identity::RealmAdmin;

struct Script {
    users: (u16, String),
    create: (u16, String),
    forgot: (u16, String),
}

fn serve(script: Script) -> (String, Arc<Mutex<Vec<String>>>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let recorded = seen.clone();
    let listener = TcpListener::bind("127.0.0.1:0").expect("a free port");
    let address = listener.local_addr().expect("an address");
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let mut buffer = [0u8; 8192];
            let read = stream.read(&mut buffer).unwrap_or(0);
            let request = String::from_utf8_lossy(&buffer[..read]).to_string();
            let line = request.lines().next().unwrap_or_default().to_string();
            recorded.lock().unwrap().push(request.clone());
            let (status, body) = if line.contains("/protocol/openid-connect/token") {
                (200, r#"{"access_token":"admin-token"}"#.to_string())
            } else if line.contains("forgot-password") {
                script.forgot.clone()
            } else if line.starts_with("POST") {
                script.create.clone()
            } else {
                script.users.clone()
            };
            let response = format!(
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    (format!("http://{address}"), seen)
}

fn realm(base_url: String) -> FerrisKeyPlatformRealm {
    FerrisKeyPlatformRealm::new(RealmAdmin {
        base_url,
        realm: "autharie".to_string(),
        admin_realm: "master".to_string(),
        admin_client_id: "admin-cli".to_string(),
        admin_username: "admin".to_string(),
        admin_password: "secret".to_string(),
    })
}

fn quiet() -> Script {
    Script {
        users: (200, r#"{"data":[]}"#.to_string()),
        create: (
            201,
            r#"{"data":{"id":"u-1","email":"new@acme.test"}}"#.to_string(),
        ),
        forgot: (200, "{}".to_string()),
    }
}

#[tokio::test]
async fn an_address_nobody_holds_is_not_found() {
    let (url, seen) = serve(quiet());

    let found = realm(url).find_user_by_email("new@acme.test").await;

    assert_eq!(found.expect("answered"), None);
    let requests = seen.lock().unwrap();
    assert!(
        requests[1].starts_with("GET /realms/autharie/users?email=new%40acme.test"),
        "{}",
        requests[1]
    );
    assert!(requests[1].contains("authorization: Bearer admin-token"));
}

#[tokio::test]
async fn only_an_exact_address_counts_as_found() {
    let (url, _) = serve(Script {
        users: (
            200,
            r#"{"data":[{"id":"u-9","email":"anew@acme.test"},{"id":"u-2","email":"New@Acme.test"}]}"#
                .to_string(),
        ),
        ..quiet()
    });

    let found = realm(url).find_user_by_email("new@acme.test").await;

    assert_eq!(
        found.expect("answered"),
        Some(PlatformUserId("u-2".to_string()))
    );
}

#[tokio::test]
async fn a_user_is_created_with_the_address_as_username_and_unverified() {
    let (url, seen) = serve(quiet());

    let created = realm(url).create_user("new@acme.test").await;

    assert_eq!(created.expect("created"), PlatformUserId("u-1".to_string()));
    let requests = seen.lock().unwrap();
    assert!(
        requests[1].starts_with("POST /realms/autharie/users "),
        "{}",
        requests[1]
    );
    assert!(requests[1].contains(r#""username":"new@acme.test""#));
    assert!(requests[1].contains(r#""email_verified":false"#));
    assert!(!requests[1].contains("password"));
}

#[tokio::test]
async fn the_set_password_request_goes_to_the_public_forgot_password_path() {
    let (url, seen) = serve(quiet());

    realm(url)
        .request_password_setup("new@acme.test")
        .await
        .expect("requested");

    let requests = seen.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0].starts_with("POST /realms/autharie/login-actions/forgot-password "),
        "{}",
        requests[0]
    );
    assert!(requests[0].contains(r#""email":"new@acme.test""#));
}

#[tokio::test]
async fn refusals_surface_as_identity_provider_unavailable() {
    let (url, _) = serve(Script {
        users: (500, String::new()),
        create: (403, String::new()),
        forgot: (403, String::new()),
    });
    let realm = realm(url);

    for outcome in [
        realm.find_user_by_email("a@acme.test").await.map(|_| ()),
        realm.create_user("a@acme.test").await.map(|_| ()),
        realm.request_password_setup("a@acme.test").await,
    ] {
        assert!(matches!(
            outcome,
            Err(CoreError::IdentityProviderUnavailable { .. })
        ));
    }
}

#[tokio::test]
async fn without_an_administrator_the_adapter_skips_account_creation() {
    let adapter = PlatformRealmAdapter::from_admin(None);

    assert!(
        adapter
            .find_user_by_email("a@acme.test")
            .await
            .expect("no call")
            .is_some()
    );
}
