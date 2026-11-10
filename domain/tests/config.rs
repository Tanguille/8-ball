use domain::config::Config;

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

#[test]
fn learning_defaults() {
    let learning = Config::parse(MINIMAL, "test").unwrap().learning;
    assert!(learning.report);
    assert!(learning.remember);
    assert!((learning.explore - 0.05).abs() < 1e-9);
    assert_eq!(learning.recalibrate_after, 20);
    assert!(learning.ad_hoc);
}

#[test]
fn learning_overrides() {
    let learning = Config::parse(
        &format!("{MINIMAL}[learning]\nexplore = 0.0\nad_hoc = false\n"),
        "test",
    )
    .unwrap()
    .learning;
    assert!(learning.report);
    assert!(learning.remember);
    assert!(learning.explore.abs() < 1e-9);
    assert_eq!(learning.recalibrate_after, 20);
    assert!(!learning.ad_hoc);
}

#[test]
fn learning_explore_range() {
    for explore in ["1.5", "-0.1", "nan"] {
        assert!(
            Config::parse(
                &format!("{MINIMAL}[learning]\nexplore = {explore}\n"),
                "test"
            )
            .unwrap_err()
            .to_string()
            .contains("learning.explore must be between 0 and 1"),
            "explore = {explore}"
        );
    }
}

#[test]
fn learning_unknown_key_rejected() {
    assert!(
        Config::parse(&format!("{MINIMAL}[learning]\ncolour = 1\n"), "test")
            .unwrap_err()
            .to_string()
            .contains("colour")
    );
}
