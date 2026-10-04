use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
};

use autharie_domain::{
    CoreError,
    dataplane::{
        herald_identity::MintedHeraldIdentity, ports::HeraldIdentityProvisioner,
        value_objects::DataPlaneId,
    },
};
use uuid::Uuid;

use super::{FerrisKeyHeraldIdentities, RealmAdmin};

struct Script {
    clients: (u16, String),
    delete: (u16, String),
}

fn serve(script: Script) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a free port");
    let address = listener.local_addr().expect("an address");
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let mut buffer = [0u8; 4096];
            let read = stream.read(&mut buffer).unwrap_or(0);
            let request = String::from_utf8_lossy(&buffer[..read]).to_string();
            let line = request.lines().next().unwrap_or_default().to_string();
            let (status, body) = if line.contains("/protocol/openid-connect/token") {
                (200, r#"{"access_token":"admin-token"}"#.to_string())
            } else if line.starts_with("DELETE") {
                script.delete.clone()
            } else {
                script.clients.clone()
            };
            let response = format!(
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    format!("http://{address}")
}

fn identities(base_url: String) -> FerrisKeyHeraldIdentities {
    FerrisKeyHeraldIdentities::new(RealmAdmin {
        base_url,
        realm: "autharie".to_string(),
        admin_realm: "master".to_string(),
        admin_client_id: "admin-cli".to_string(),
        admin_username: "admin".to_string(),
        admin_password: "secret".to_string(),
    })
}

fn plane() -> DataPlaneId {
    DataPlaneId(Uuid::new_v4())
}

fn listing_the_client(plane: DataPlaneId) -> String {
    format!(
        r#"{{"data":[{{"id":"uuid-1","client_id":"{}"}}]}}"#,
        MintedHeraldIdentity::client_id_for(plane)
    )
}

#[tokio::test]
async fn revoking_an_identity_that_does_not_exist_succeeds() {
    let url = serve(Script {
        clients: (200, r#"{"data":[]}"#.to_string()),
        delete: (500, String::new()),
    });

    let outcome = identities(url).revoke(plane()).await;

    assert!(outcome.is_ok());
}

#[tokio::test]
async fn a_client_removed_between_the_lookup_and_the_delete_still_succeeds() {
    let plane = plane();
    let url = serve(Script {
        clients: (200, listing_the_client(plane)),
        delete: (404, String::new()),
    });

    assert!(identities(url).revoke(plane).await.is_ok());
}

#[tokio::test]
async fn an_existing_identity_is_removed() {
    let plane = plane();
    let url = serve(Script {
        clients: (200, listing_the_client(plane)),
        delete: (204, String::new()),
    });

    assert!(identities(url).revoke(plane).await.is_ok());
}

#[tokio::test]
async fn a_refused_delete_surfaces_as_an_error() {
    let plane = plane();
    let url = serve(Script {
        clients: (200, listing_the_client(plane)),
        delete: (500, String::new()),
    });

    let outcome = identities(url).revoke(plane).await;

    assert!(matches!(outcome, Err(CoreError::InternalError(_))));
}

#[tokio::test]
async fn a_failing_lookup_surfaces_as_an_error() {
    let url = serve(Script {
        clients: (500, String::new()),
        delete: (204, String::new()),
    });

    let outcome = identities(url).revoke(plane()).await;

    assert!(matches!(outcome, Err(CoreError::InternalError(_))));
}
