use autharie_domain::billing::{
    BillingError, ProviderCustomerId, ProviderEvent, ProviderEventId, ProviderEventKind,
    ProviderInvoiceId, ProviderPaymentMethodId,
};
use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use serde_json::Value;
use sha2::Sha256;

const TOLERANCE_SECONDS: u64 = 300;

#[derive(Deserialize)]
struct Envelope {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    data: Value,
}

struct Header<'a> {
    timestamp: &'a str,
    seconds: i64,
    candidates: Vec<Vec<u8>>,
}

fn parse_header(header: &str) -> Option<Header<'_>> {
    let mut timestamp = None;
    let mut candidates = Vec::new();

    for item in header.split(',') {
        let (key, value) = item.split_once('=')?;

        match key {
            "t" if timestamp.is_none() => timestamp = Some((value, value.parse::<i64>().ok()?)),
            "t" => return None,
            "v1" => candidates.extend(hex::decode(value).ok()),
            _ => {}
        }
    }

    let (timestamp, seconds) = timestamp?;

    Some(Header {
        timestamp,
        seconds,
        candidates,
    })
}

fn verify(
    secret: &SecretString,
    payload: &[u8],
    header: &str,
    now: DateTime<Utc>,
) -> Result<(), BillingError> {
    let header = parse_header(header).ok_or(BillingError::InvalidEventSignature)?;

    let mut mac = Hmac::<Sha256>::new_from_slice(secret.expose_secret().as_bytes())
        .map_err(|_| BillingError::InvalidEventSignature)?;
    mac.update(header.timestamp.as_bytes());
    mac.update(b".");
    mac.update(payload);

    let fresh = now.timestamp().abs_diff(header.seconds) <= TOLERANCE_SECONDS;
    let genuine = header
        .candidates
        .iter()
        .any(|candidate| mac.clone().verify_slice(candidate).is_ok());

    if fresh && genuine {
        Ok(())
    } else {
        Err(BillingError::InvalidEventSignature)
    }
}

fn invoice_of(data: &Value) -> Result<ProviderInvoiceId, BillingError> {
    data.pointer("/object/id")
        .and_then(Value::as_str)
        .and_then(|raw| ProviderInvoiceId::parse(raw).ok())
        .ok_or(BillingError::InvalidEventSignature)
}

fn payment_method_ready(data: &Value) -> ProviderEventKind {
    let customer = data
        .pointer("/object/customer")
        .and_then(Value::as_str)
        .and_then(|raw| ProviderCustomerId::parse(raw).ok());
    let payment_method = data
        .pointer("/object/payment_method")
        .and_then(Value::as_str)
        .and_then(|raw| ProviderPaymentMethodId::parse(raw).ok());

    match (customer, payment_method) {
        (Some(customer), Some(payment_method)) => ProviderEventKind::PaymentMethodReady {
            customer,
            payment_method,
        },
        _ => ProviderEventKind::Ignored,
    }
}

fn kind_of(kind: &str, data: &Value) -> Result<ProviderEventKind, BillingError> {
    match kind {
        "setup_intent.succeeded" => Ok(payment_method_ready(data)),
        "invoice.paid" => Ok(ProviderEventKind::InvoicePaid {
            invoice: invoice_of(data)?,
        }),
        "invoice.payment_failed" => Ok(ProviderEventKind::PaymentFailed {
            invoice: invoice_of(data)?,
        }),
        "invoice.marked_uncollectible" => Ok(ProviderEventKind::InvoiceUncollectible {
            invoice: invoice_of(data)?,
        }),
        _ => Ok(ProviderEventKind::Ignored),
    }
}

pub(crate) fn read(
    secret: &SecretString,
    payload: &[u8],
    header: &str,
    now: DateTime<Utc>,
) -> Result<ProviderEvent, BillingError> {
    verify(secret, payload, header, now)?;

    let envelope: Envelope =
        serde_json::from_slice(payload).map_err(|_| BillingError::InvalidEventSignature)?;
    let id =
        ProviderEventId::parse(&envelope.id).map_err(|_| BillingError::InvalidEventSignature)?;
    let kind = kind_of(&envelope.kind, &envelope.data)?;

    Ok(ProviderEvent { id, kind })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_header_is_split_into_its_timestamp_and_its_v1_signatures() {
        let header = parse_header("t=1700000000,v0=aa,v1=0a0b,v1=zz,v1=ff").unwrap();

        assert_eq!(header.timestamp, "1700000000");
        assert_eq!(header.seconds, 1_700_000_000);
        assert_eq!(header.candidates, vec![vec![0x0a, 0x0b], vec![0xff]]);
    }

    #[test]
    fn a_header_with_two_timestamps_is_refused() {
        assert!(parse_header("t=1,t=2,v1=aa").is_none());
    }

    #[test]
    fn the_timestamp_is_signed_as_written() {
        let header = parse_header("t=0001,v1=aa").unwrap();

        assert_eq!(header.timestamp, "0001");
        assert_eq!(header.seconds, 1);
    }
}
