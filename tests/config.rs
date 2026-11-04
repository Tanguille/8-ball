use std::io::Write;

use eightball::config::Config;

fn load(text: &str) -> Result<Config, eightball::config::ConfigError> {
    let mut file = tempfile::Builder::new()
        .prefix("8ball-")
        .tempfile()
        .unwrap();
    file.write_all(text.as_bytes()).unwrap();

    Config::load(file.path())
}

const MINIMAL: &str = "default = 'd3'\n[endpoints.d3]\nurl = 'http://h:1/v1/'\n";

#[test]
fn minimal_ok() {
    let endpoint_config = load(MINIMAL).unwrap();
    assert_eq!(endpoint_config.endpoints["d3"].url, "http://h:1");
}

#[test]
fn unknown_key_rejected() {
    assert!(
        load(&format!("{MINIMAL}colour = 'red'\n"))
            .unwrap_err()
            .to_string()
            .contains("colour")
    );
}

#[test]
fn default_must_exist() {
    assert!(
        load(&MINIMAL.replace("default = 'd3'", "default = 'nope'"))
            .unwrap_err()
            .to_string()
            .contains("nope")
    );
}

#[test]
fn spec_parses() {
    let endpoint_config = load(&format!("{MINIMAL}\n[specs.team]\ntype = 'choice'\ninstructions = 'Which team?'\ncriteria = {{ billing = '', shipping = 'Parcels' }}\n")).unwrap();
    assert_eq!(
        endpoint_config.specs["team"].kind.labels(),
        ["billing", "shipping"]
    );
}

#[test]
fn bad_spec_name() {
    assert!(
        load(&format!(
            "{MINIMAL}\n[specs.Bad_Name]\ntype = 'noul'\ninstructions = 'x'\n"
        ))
        .unwrap_err()
        .to_string()
        .contains("Bad_Name")
    );
}

#[test]
fn logprob_needs_model() {
    assert!(
        load(&format!("{MINIMAL}kind = 'logprob'\n"))
            .unwrap_err()
            .to_string()
            .contains("model")
    );
}

#[test]
fn zero_concurrency() {
    assert!(load(&format!("{MINIMAL}concurrency = 0\n")).is_err());
}

#[test]
fn retention_default() {
    assert_eq!(load(MINIMAL).unwrap().retention_days, 90);
}
