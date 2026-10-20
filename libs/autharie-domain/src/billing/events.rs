use crate::billing::{
    BillingError,
    account::{ProviderCustomerId, provider_reference},
    statement::ProviderInvoiceId,
};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProviderEventId(String);

impl ProviderEventId {
    pub fn parse(raw: &str) -> Result<Self, BillingError> {
        provider_reference(raw).map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProviderPaymentMethodId(String);

impl ProviderPaymentMethodId {
    pub fn parse(raw: &str) -> Result<Self, BillingError> {
        provider_reference(raw).map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderEventKind {
    PaymentMethodReady {
        customer: ProviderCustomerId,
        payment_method: ProviderPaymentMethodId,
    },
    InvoicePaid {
        invoice: ProviderInvoiceId,
    },
    PaymentFailed {
        invoice: ProviderInvoiceId,
    },
    InvoiceUncollectible {
        invoice: ProviderInvoiceId,
    },
    Ignored,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderEvent {
    pub id: ProviderEventId,
    pub kind: ProviderEventKind,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_event_id_follows_the_provider_reference_rule() {
        assert_eq!(ProviderEventId::parse(" evt_1 ").unwrap().as_str(), "evt_1");

        for raw in ["", "  ", "evt 1", &"e".repeat(256)] {
            assert!(
                matches!(
                    ProviderEventId::parse(raw),
                    Err(BillingError::InvalidProviderReference { .. })
                ),
                "{raw:?}"
            );
        }
    }

    #[test]
    fn a_payment_method_id_follows_the_provider_reference_rule() {
        assert_eq!(
            ProviderPaymentMethodId::parse("pm_1").unwrap().as_str(),
            "pm_1"
        );
        assert!(ProviderPaymentMethodId::parse("").is_err());
        assert!(ProviderPaymentMethodId::parse("pm 1").is_err());
    }

    #[test]
    fn an_event_carries_its_id_and_its_kind() {
        let event = ProviderEvent {
            id: ProviderEventId::parse("evt_1").unwrap(),
            kind: ProviderEventKind::Ignored,
        };

        assert_eq!(event.kind, ProviderEventKind::Ignored);
        assert_eq!(event.id.as_str(), "evt_1");
    }
}
