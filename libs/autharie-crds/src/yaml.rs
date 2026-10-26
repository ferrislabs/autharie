use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;

const YAML_1_1_BOOLEANS: [&str; 16] = [
    "y", "Y", "yes", "Yes", "YES", "n", "N", "no", "No", "NO", "on", "On", "ON", "off", "Off",
    "OFF",
];

fn quoted(word: &str) -> String {
    if YAML_1_1_BOOLEANS.contains(&word) {
        format!("\"{word}\"")
    } else {
        word.to_string()
    }
}

fn quote_line(line: &str) -> String {
    let body = line.trim_start();
    let indent = &line[..line.len() - body.len()];
    let (dash, rest) = match body.strip_prefix("- ") {
        Some(rest) => ("- ", rest),
        None => ("", body),
    };

    let rewritten = match rest.split_once(": ") {
        Some((key, value)) => format!("{}: {}", quoted(key), quoted(value)),
        None => match rest.strip_suffix(':') {
            Some(key) => format!("{}:", quoted(key)),
            None => quoted(rest),
        },
    };

    format!("{indent}{dash}{rewritten}")
}

/// A CRD as YAML that kubectl reads the way the schema means it.
///
/// `serde_yaml` writes YAML 1.2, where a bare `n` is a string. kubectl reads
/// YAML 1.1, where it is `false`: the JWK field `n` became a property named
/// `false` and a `required` entry that is a boolean, and the API server
/// refused the whole CRD. Words that 1.1 reads as booleans are quoted.
pub fn crd_to_yaml(crd: &CustomResourceDefinition) -> Result<String, serde_yaml::Error> {
    let yaml = serde_yaml::to_string(crd)?;

    Ok(yaml.lines().map(quote_line).collect::<Vec<_>>().join("\n") + "\n")
}

#[cfg(test)]
mod tests {
    use kube::CustomResourceExt;

    use super::*;
    use crate::v1alpha::identity_instance::IdentityInstance;

    #[test]
    fn a_key_that_yaml_1_1_reads_as_a_boolean_is_quoted() {
        assert_eq!(
            quote_line("                            n:"),
            "                            \"n\":"
        );
    }

    #[test]
    fn a_list_item_that_yaml_1_1_reads_as_a_boolean_is_quoted() {
        assert_eq!(
            quote_line("                          - n"),
            "                          - \"n\""
        );
    }

    #[test]
    fn a_value_that_yaml_1_1_reads_as_a_boolean_is_quoted() {
        assert_eq!(quote_line("  required: no"), "  required: \"no\"");
    }

    #[test]
    fn ordinary_words_are_left_alone() {
        for line in ["  name: node", "  - number", "  nothing:", "type: string"] {
            assert_eq!(quote_line(line), line);
        }
    }

    #[test]
    fn the_instance_crd_has_no_bare_word_yaml_1_1_would_turn_into_a_boolean() {
        let yaml = crd_to_yaml(&IdentityInstance::crd()).expect("serialises");

        for line in yaml.lines() {
            let word = line
                .trim()
                .trim_start_matches("- ")
                .trim_end_matches(':')
                .rsplit(": ")
                .next()
                .unwrap_or_default();
            assert!(
                !YAML_1_1_BOOLEANS.contains(&word),
                "bare `{word}` in `{line}`"
            );
        }
    }
}
