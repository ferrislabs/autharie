use std::fmt;

use tracing::{Event, Subscriber, field::Field};
use tracing_subscriber::{
    EnvFilter, Layer,
    filter::{FilterExt, filter_fn},
    fmt::{FmtContext, FormatEvent, FormatFields, MakeWriter, format::Writer},
    layer::SubscriberExt,
    registry::LookupSpan,
    util::SubscriberInitExt,
};

use crate::decision::DECISION_TARGET;

pub struct MessageOnly;

struct Message<'a, 'w>(&'a mut Writer<'w>, fmt::Result);

impl tracing::field::Visit for Message<'_, '_> {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            self.1 = write!(self.0, "{value:?}");
        }
    }
}

impl<S, N> FormatEvent<S, N> for MessageOnly
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        _ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        let mut visitor = Message(&mut writer, Ok(()));
        event.record(&mut visitor);
        let written = visitor.1;
        written?;
        writeln!(writer)
    }
}

pub fn decision_layer<S, W>(writer: W) -> impl Layer<S>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    W: for<'w> MakeWriter<'w> + 'static,
{
    tracing_subscriber::fmt::layer()
        .event_format(MessageOnly)
        .with_writer(writer)
        .with_filter(filter_fn(|meta| meta.target() == DECISION_TARGET))
}

pub fn init() {
    let application = tracing_subscriber::fmt::layer().with_filter(
        EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new("info"))
            .and(filter_fn(|meta| meta.target() != DECISION_TARGET)),
    );
    tracing_subscriber::registry()
        .with(application)
        .with(decision_layer(std::io::stdout))
        .init();
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::decision::{DecisionRecord, DecisionSink, KIND, TracingSink};

    #[derive(Clone, Default)]
    struct Buffer(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for Buffer {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for Buffer {
        type Writer = Buffer;

        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    #[test]
    fn a_decision_is_one_bare_json_line_and_other_events_stay_out() {
        let buffer = Buffer::default();
        let subscriber = tracing_subscriber::registry().with(decision_layer(buffer.clone()));
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!("not a decision");
            TracingSink.record(&DecisionRecord {
                kind: KIND,
                time: "t".into(),
                server: "s".into(),
                method: "tools/call".into(),
                tool: Some("x".into()),
                subject_type: "Client",
                subject_id: "c".into(),
                agent: "c".into(),
                delegated: false,
                decision: "deny",
                policies: vec![],
                reason: Some("pdp_error"),
                request_id: "r".into(),
            });
        });
        let written = String::from_utf8(buffer.0.lock().unwrap().clone()).unwrap();
        let lines: Vec<&str> = written.lines().collect();
        assert_eq!(lines.len(), 1);
        let value: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(value["kind"], "agent_decision");
        assert_eq!(value["decision"], "deny");
    }
}
