use autharie_crds::v1alpha::identity_instance::SealingKeySpec;
use autharie_domain::iam::{
    IamActionPayload, IamOperation as ControlPlaneOperation, IamOutcome as ControlPlaneOutcome,
    IamVerdict,
};
use autharie_operator_core::domain::agents::sealing::SealingKey;
use autharie_operator_core::domain::iam::IamOperation as OperatorOperation;
use chrono::{Duration, Utc};
use genesis_core::domain::entities::iam_outcome::IamOutcomeReport;
use genesis_core::domain::entities::iam_payload::IamRequestPayloadV1;
use herald_core::domain::entities::outcome::InboundOutcome;
use serde_json::{Value, json};
use uuid::Uuid;

const FIXTURE: &str = include_str!("../../autharie-operator-core/tests/fixtures/iam-envelope.json");

fn fixture() -> Value {
    serde_json::from_str(FIXTURE).expect("the committed fixture is JSON")
}

fn public_key_of(fixture: &Value) -> Value {
    let private = &fixture["private_jwk"];
    json!({ "kty": "RSA", "n": private["n"], "e": private["e"], "alg": "RSA-OAEP-256" })
}

fn the_action_the_control_plane_records(operation: &ControlPlaneOperation) -> (Uuid, Value) {
    let request_id = Uuid::new_v4();
    let payload = IamActionPayload::new(
        Uuid::new_v4(),
        "acme",
        request_id,
        "retail-fr",
        operation,
        public_key_of(&fixture()),
        Utc::now() + Duration::minutes(5),
    );

    (
        request_id,
        serde_json::to_value(payload).expect("serialises"),
    )
}

#[test]
fn a_listing_asked_by_the_control_plane_is_read_by_genesis_and_then_by_the_operator() {
    let operation = ControlPlaneOperation::applications_list(Some(2), Some(40), Some("dupont"))
        .expect("a valid operation");
    let (request_id, action) = the_action_the_control_plane_records(&operation);

    let payload = IamRequestPayloadV1::from_value(&action).expect("genesis reads the action");

    assert_eq!(
        payload.entry.request_id,
        request_id.hyphenated().to_string()
    );
    assert_eq!(payload.entry.realm, "retail-fr");
    let read = OperatorOperation::try_from(payload.entry.operation.as_str())
        .expect("the operator reads what genesis wrote");
    let OperatorOperation::ApplicationsList {
        page,
        limit,
        search,
    } = read
    else {
        panic!("a listing stays a listing");
    };
    assert_eq!((page.get(), limit.get()), (2, 40));
    assert_eq!(search.expect("the search term travels").as_str(), "dupont");
}

#[test]
fn a_single_application_asked_by_the_control_plane_reaches_the_operator() {
    let id = Uuid::new_v4();
    let (_, action) =
        the_action_the_control_plane_records(&ControlPlaneOperation::ApplicationsGet { id });

    let payload = IamRequestPayloadV1::from_value(&action).expect("genesis reads the action");

    assert_eq!(
        OperatorOperation::try_from(payload.entry.operation.as_str())
            .expect("the operator reads it"),
        OperatorOperation::ApplicationsGet { id }
    );
}

#[test]
fn the_sealing_key_genesis_passes_on_is_one_the_operator_accepts() {
    let operation = ControlPlaneOperation::applications_list(None, None, None).expect("valid");
    let (_, action) = the_action_the_control_plane_records(&operation);
    let payload = IamRequestPayloadV1::from_value(&action).expect("genesis reads the action");

    let spec: SealingKeySpec =
        serde_json::from_str(&payload.entry.sealing_key).expect("the entry holds a JWK");

    SealingKey::try_from(&spec).expect("the operator accepts it");
}

#[test]
fn the_envelope_the_operator_made_comes_back_through_herald_and_is_accepted_by_the_control_plane() {
    let fixture = fixture();
    let request_id: Uuid = fixture["request_id"]
        .as_str()
        .expect("a request id")
        .parse()
        .expect("a UUID");
    let deployment_id = Uuid::new_v4();

    let published = serde_json::to_vec(&IamOutcomeReport::done(
        deployment_id,
        request_id.hyphenated().to_string(),
        fixture["envelope"].clone(),
    ))
    .expect("serialises");
    let InboundOutcome::Relayed(relayed) =
        InboundOutcome::parse(&published).expect("herald reads what genesis published")
    else {
        panic!("herald must carry an iam outcome, not drop it as unreadable");
    };
    assert_eq!(relayed.kind(), "iam");
    assert_eq!(relayed.deployment_id(), deployment_id);

    let body = relayed.body();
    let outcome = ControlPlaneOutcome::parse(
        body["request_id"].as_str().and_then(|id| id.parse().ok()),
        body["phase"].as_str(),
        body["message"].as_str().map(str::to_string),
        body.get("envelope").cloned(),
    )
    .expect("the control plane accepts the report herald relayed");

    assert_eq!(outcome.request_id, request_id);
    let IamVerdict::Done(envelope) = outcome.verdict else {
        panic!("a finished request stays finished");
    };
    assert_eq!(
        envelope.into_value().expect("an envelope is a value"),
        fixture["envelope"],
        "the control plane keeps the envelope exactly as the operator sealed it"
    );
}

#[test]
fn a_failure_keeps_its_reason_all_the_way_to_the_control_plane() {
    let request_id = Uuid::new_v4();

    let published = serde_json::to_vec(&IamOutcomeReport::failed(
        Uuid::new_v4(),
        request_id.hyphenated().to_string(),
        Some("result too large".to_string()),
    ))
    .expect("serialises");
    let InboundOutcome::Relayed(relayed) = InboundOutcome::parse(&published).expect("parses")
    else {
        panic!("herald must carry an iam outcome");
    };
    let body = relayed.body();

    let outcome = ControlPlaneOutcome::parse(
        body["request_id"].as_str().and_then(|id| id.parse().ok()),
        body["phase"].as_str(),
        body["message"].as_str().map(str::to_string),
        body.get("envelope").cloned(),
    )
    .expect("accepted");

    assert_eq!(
        outcome.verdict,
        IamVerdict::Failed {
            message: "result too large".to_string()
        }
    );
}
