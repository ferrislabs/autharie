#[derive(Default)]
pub(crate) struct Form<'a> {
    pairs: Vec<(&'static str, &'a str)>,
}

impl<'a> Form<'a> {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn field(mut self, key: &'static str, value: &'a str) -> Self {
        self.pairs.push((key, value));
        self
    }

    pub(crate) fn optional(self, key: &'static str, value: Option<&'a str>) -> Self {
        match value {
            Some(value) => self.field(key, value),
            None => self,
        }
    }

    pub(crate) fn pairs(&self) -> &[(&'static str, &'a str)] {
        &self.pairs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_keep_their_order_and_optional_ones_are_skipped_when_absent() {
        let form = Form::new()
            .field("a", "1")
            .optional("b", None)
            .optional("c", Some("3"))
            .field("d", "4");

        let keys: Vec<_> = form.pairs().iter().map(|(key, _)| *key).collect();
        let values: Vec<_> = form.pairs().iter().map(|(_, value)| *value).collect();

        assert_eq!(keys, ["a", "c", "d"]);
        assert_eq!(values, ["1", "3", "4"]);
    }
}
