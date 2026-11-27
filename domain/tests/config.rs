use domain::config::Config;

const MINIMAL: &str = "default = 'd3'\n[endpoints.d3]\nurl = 'http://h:1/v1/'\n";

fn error(text: &str) -> String {
    Config::parse(text, "test").unwrap_err().to_string()
}

#[test]
fn minimal_ok() {
    let endpoint_config = Config::parse(MINIMAL, "test").unwrap();
    assert_eq!(endpoint_config.endpoints["d3"].url, "http://h:1");
}

#[test]
fn unknown_key_rejected() {
    assert!(error(&format!("{MINIMAL}colour = 'red'\n")).contains("colour"));
}

#[test]
fn default_must_exist() {
    assert!(error(&MINIMAL.replace("default = 'd3'", "default = 'nope'")).contains("nope"));
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
        error(&format!(
            "{MINIMAL}\n[specs.Bad_Name]\ntype = 'noul'\ninstructions = 'x'\n"
        ))
        .contains("Bad_Name")
    );
}

#[test]
fn logprob_needs_model() {
    assert!(error(&format!("{MINIMAL}kind = 'logprob'\n")).contains("model"));
}

#[test]
fn zero_rejected() {
    for field in ["concurrency", "timeout_secs", "requests_per_minute"] {
        assert!(
            error(&format!("{MINIMAL}{field} = 0\n")).contains("nonzero"),
            "{field}"
        );
    }
}

#[test]
fn retention_default() {
    assert_eq!(Config::parse(MINIMAL, "test").unwrap().retention_days, 90);
}

#[test]
fn learning_defaults_and_overrides() {
    let learning = Config::parse(MINIMAL, "test").unwrap().learning;
    assert!(learning.report);
    assert!(learning.remember);
    assert!((learning.explore - 0.05).abs() < 1e-9);
    assert_eq!(learning.recalibrate_after, 20);
    assert!(learning.ad_hoc);
    let learning = Config::parse(
        &format!("{MINIMAL}[learning]\nexplore = 0.0\nad_hoc = false\n"),
        "test",
    )
    .unwrap()
    .learning;
    assert!(learning.explore.abs() < 1e-9);
    assert!(!learning.ad_hoc);
}

#[test]
fn learning_explore_range() {
    for explore in ["1.5", "-0.1", "nan"] {
        assert!(
            error(&format!("{MINIMAL}[learning]\nexplore = {explore}\n"))
                .contains("learning.explore must be between 0 and 1"),
            "explore = {explore}"
        );
    }
}

#[test]
fn learning_unknown_key_rejected() {
    assert!(error(&format!("{MINIMAL}[learning]\ncolour = 1\n")).contains("colour"));
}

/// The CLI prints errors with `{e:#}`, which appends the source; the message must not repeat it.
#[test]
fn read_error_prints_the_io_error_once() {
    let error = Config::load(std::path::Path::new("/nonexistent/8ball.toml")).unwrap_err();
    assert_eq!(error.to_string(), "cannot read /nonexistent/8ball.toml");
    assert!(std::error::Error::source(&error).is_some());
}

const TWO_ENDPOINTS: &str =
    "default = 'd3'\n[endpoints.d3]\nurl = 'http://h:1'\n[endpoints.big]\nurl = 'http://h:2'\n";

fn escalation_error(escalation: &str) -> String {
    error(&format!("{TWO_ENDPOINTS}[escalation]\n{escalation}"))
}

#[test]
fn escalation_parses_with_default_budget() {
    let config = Config::parse(
        &format!("{TWO_ENDPOINTS}[escalation]\nendpoint = 'big'\n"),
        "test",
    )
    .unwrap();
    let escalation = config.escalation.unwrap();
    assert_eq!(escalation.endpoint, "big");
    assert_eq!(escalation.per_minute.get(), 30);
}

#[test]
fn escalation_unknown_endpoint_rejected() {
    assert!(escalation_error("endpoint = 'nope'\n").contains("escalation: unknown endpoint nope"));
}

#[test]
fn escalation_to_default_rejected() {
    assert!(escalation_error("endpoint = 'd3'\n").contains("is the default endpoint"));
}

#[test]
fn escalation_zero_budget_rejected() {
    assert!(escalation_error("endpoint = 'big'\nper_minute = 0\n").contains("nonzero"));
}

#[test]
fn escalation_unknown_key_rejected() {
    assert!(escalation_error("endpoint = 'big'\nmargin = 0.2\n").contains("margin"));
}

#[test]
fn spec_escalate_parses() {
    let spec = |extra: &str| {
        Config::parse(
            &format!("{MINIMAL}\n[specs.s]\ntype = 'noul'\ninstructions = 'x'\n{extra}"),
            "test",
        )
        .unwrap()
        .specs["s"]
            .escalate
    };
    assert_eq!(spec("escalate = false\n"), Some(false));
    assert_eq!(spec(""), None);
}

#[test]
fn max_state_chars_below_two_rejected() {
    for maximum in [0, 1] {
        let text = format!("{MINIMAL}max_state_chars = {maximum}\n");
        assert!(
            error(&text).contains("max_state_chars must be at least 2"),
            "{maximum}"
        );
    }
}
