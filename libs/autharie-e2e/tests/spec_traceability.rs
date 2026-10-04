use std::{collections::BTreeMap, fs, path::Path};

const TAG: &str = "@spec-ccp-";

fn read(relative: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn number_of(tag: &str) -> Option<u32> {
    tag.strip_prefix(TAG)?.parse().ok()
}

fn spec_lines(spec: &str) -> BTreeMap<u32, String> {
    spec.lines()
        .filter_map(|line| {
            let (tag, rest) = line.trim().split_once(char::is_whitespace)?;
            let number = number_of(tag)?;
            let name = rest.trim().strip_prefix("Scenario:")?.trim();
            Some((number, name.to_string()))
        })
        .collect()
}

fn feature_scenarios(feature: &str) -> BTreeMap<u32, String> {
    let mut scenarios = BTreeMap::new();
    let mut pending: Vec<u32> = Vec::new();

    for line in feature.lines().map(str::trim) {
        if line.starts_with('@') {
            pending.extend(line.split_whitespace().filter_map(number_of));
        } else if let Some(name) = line
            .strip_prefix("Scenario Outline:")
            .or_else(|| line.strip_prefix("Scenario:"))
        {
            for number in pending.drain(..) {
                scenarios.insert(number, name.trim().to_string());
            }
        } else if !line.is_empty() && !line.starts_with('#') {
            pending.clear();
        }
    }
    scenarios
}

#[test]
fn every_acceptance_line_has_a_scenario_and_every_scenario_an_acceptance_line() {
    let spec = spec_lines(&read("../../docs/specs/customer-cloud-provider.md"));
    let scenarios = feature_scenarios(&read(
        "tests/features/customer_cloud/customer_cloud.feature",
    ));

    let mut problems = Vec::new();
    for (number, name) in &spec {
        match scenarios.get(number) {
            None => problems.push(format!(
                "{TAG}{number} is in the spec but has no scenario: {name}"
            )),
            Some(found) if found != name => problems.push(format!(
                "{TAG}{number} is named differently\n    spec:     {name}\n    scenario: {found}"
            )),
            Some(_) => {}
        }
    }
    for (number, name) in &scenarios {
        if !spec.contains_key(number) {
            problems.push(format!(
                "{TAG}{number} tags a scenario with no line in the spec: {name}"
            ));
        }
    }
    if spec.is_empty() {
        problems.push("the spec holds no acceptance line, is the path right?".to_string());
    }

    assert!(
        problems.is_empty(),
        "the spec and the scenarios disagree:\n  {}",
        problems.join("\n  ")
    );
}

#[test]
fn a_scenario_tagged_twice_or_a_tag_left_without_a_scenario_is_not_missed() {
    let feature = "@spec-ccp-1 @spec-ccp-2\nScenario: shared\n\n@spec-ccp-3\nGiven something\n";

    let found = feature_scenarios(feature);

    assert_eq!(
        found.keys().copied().collect::<Vec<_>>(),
        vec![1, 2],
        "{found:?}"
    );
}

#[test]
fn the_spec_parser_reads_the_numbered_lines_and_nothing_else() {
    let spec = "  @spec-ccp-1  Scenario: first one\n@spec-ccp-12 Scenario: a `second` one\nScenario: no tag\n";

    let lines = spec_lines(spec);

    assert_eq!(lines.len(), 2);
    assert_eq!(lines[&1], "first one");
    assert_eq!(lines[&12], "a `second` one");
}
