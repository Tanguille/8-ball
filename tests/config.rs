use eightball::config::Config;

const MINIMAL: &str = "default = 'd3'\n[endpoints.d3]\nurl = 'http://h:1/v1/'\n";

#[test]
fn minimal_ok() {
    let endpoint_config = Config::parse(MINIMAL, "test").unwrap();
    assert_eq!(endpoint_config.endpoints["d3"].url, "http://h:1");
}

#[test]
fn unknown_key_rejected() {
    assert!(
        Config::parse(&format!("{MINIMAL}colour = 'red'\n"), "test")
            .unwrap_err()
            .to_string()
            .contains("colour")
    );
}

#[test]
fn default_must_exist() {
    assert!(
        Config::parse(
            &MINIMAL.replace("default = 'd3'", "default = 'nope'"),
            "test"
        )
        .unwrap_err()
        .to_string()
        .contains("nope")
    );
}

#[test]
fn spec_parses() {
    let endpoint_config = Config::parse(&format!("{MINIMAL}\n[specs.team]\ntype = 'choice'\ninstructions = 'Which team?'\ncriteria = {{ billing = '', shipping = 'Parcels' }}\n"), "test").unwrap();
    assert_eq!(
        endpoint_config.specs["team"].kind.labels(),
        ["billing", "shipping"]
    );
}

#[test]
fn bad_spec_name() {
    assert!(
        Config::parse(
            &format!("{MINIMAL}\n[specs.Bad_Name]\ntype = 'noul'\ninstructions = 'x'\n"),
            "test"
        )
        .unwrap_err()
        .to_string()
        .contains("Bad_Name")
    );
}

#[test]
fn logprob_needs_model() {
    assert!(
        Config::parse(&format!("{MINIMAL}kind = 'logprob'\n"), "test")
            .unwrap_err()
            .to_string()
            .contains("model")
    );
}

#[test]
fn zero_concurrency() {
    assert!(Config::parse(&format!("{MINIMAL}concurrency = 0\n"), "test").is_err());
}

#[test]
fn retention_default() {
    assert_eq!(Config::parse(MINIMAL, "test").unwrap().retention_days, 90);
}
