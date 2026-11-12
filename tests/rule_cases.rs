use std::time::Duration;
use tide_terminal::rules::{RuleSpec, Rules};
const ZERO: Duration = Duration::ZERO;
fn ms(value: u64) -> Duration {
    Duration::from_millis(value)
}
fn evaluate(spec: RuleSpec, elapsed: u64, quiet: u64, text: &str) -> Option<String> {
    Rules::new(spec)
        .unwrap()
        .evaluate(ms(elapsed), ms(quiet), text)
}
#[test]
fn disabled_rules_never_stop_even_after_long_runtime() {
    assert!(evaluate(RuleSpec::default(), u64::MAX, u64::MAX, "FATAL: DONE").is_none());
}
#[test]
fn runtime_inclusive_boundary() {
    for elapsed in [0, 999, 1000, 1001, u64::MAX] {
        assert_eq!(
            evaluate(
                RuleSpec {
                    timeout: Some(ms(1000)),
                    ..Default::default()
                },
                elapsed,
                0,
                ""
            )
            .is_some(),
            elapsed >= 1000
        );
    }
}
#[test]
fn idle_uses_quiet_time_instead_of_total_time() {
    assert!(
        evaluate(
            RuleSpec {
                idle: Some(ms(1000)),
                ..Default::default()
            },
            1_000_000,
            999,
            ""
        )
        .is_none()
    );
    assert!(
        evaluate(
            RuleSpec {
                idle: Some(ms(1000)),
                ..Default::default()
            },
            0,
            1000,
            ""
        )
        .is_some()
    );
}
#[test]
fn runtime_takes_precedence_when_conditions_coincide() {
    let result = evaluate(
        RuleSpec {
            timeout: Some(ms(1000)),
            idle: Some(ms(1000)),
            literal: Some("DONE".into()),
            pattern: Some("DONE".into()),
            ..Default::default()
        },
        1000,
        1000,
        "DONE",
    );
    assert_eq!(result.as_deref(), Some("Runtime limit reached"));
}
#[test]
fn output_rule_works_before_either_timer_expires() {
    assert_eq!(
        evaluate(
            RuleSpec {
                timeout: Some(ms(1000)),
                idle: Some(ms(1000)),
                literal: Some("DONE".into()),
                ..Default::default()
            },
            5,
            5,
            "DONE"
        )
        .as_deref(),
        Some("Output marker matched")
    );
}
#[test]
fn literal_is_case_sensitive_and_not_a_regex() {
    for (text, expected) in [
        ("done", false),
        ("prefixDONEsuffix", true),
        ("DO.NE", false),
    ] {
        assert_eq!(
            evaluate(
                RuleSpec {
                    literal: Some("DONE".into()),
                    ..Default::default()
                },
                0,
                0,
                text
            )
            .is_some(),
            expected
        );
    }
    assert!(
        evaluate(
            RuleSpec {
                literal: Some("[0-9]".into()),
                ..Default::default()
            },
            0,
            0,
            "123"
        )
        .is_none()
    );
}
#[test]
fn regex_supports_unicode_and_explicit_case_flags() {
    assert!(
        evaluate(
            RuleSpec {
                pattern: Some(r"\p{Greek}+".into()),
                ..Default::default()
            },
            0,
            0,
            "λμ"
        )
        .is_some()
    );
    assert!(
        evaluate(
            RuleSpec {
                pattern: Some("(?i)fatal".into()),
                ..Default::default()
            },
            0,
            0,
            "FATAL"
        )
        .is_some()
    );
}
#[test]
fn regex_multiline_anchors_are_explicit() {
    assert!(
        evaluate(
            RuleSpec {
                pattern: Some("^DONE$".into()),
                ..Default::default()
            },
            0,
            0,
            "one\nDONE\ntwo"
        )
        .is_none()
    );
    assert!(
        evaluate(
            RuleSpec {
                pattern: Some("(?m)^DONE$".into()),
                ..Default::default()
            },
            0,
            0,
            "one\nDONE\ntwo"
        )
        .is_some()
    );
}
#[test]
fn regex_and_literal_are_or_conditions() {
    for text in ["literal", "123", "neither"] {
        assert_eq!(
            evaluate(
                RuleSpec {
                    literal: Some("literal".into()),
                    pattern: Some("[0-9]+".into()),
                    ..Default::default()
                },
                0,
                0,
                text
            )
            .is_some(),
            text != "neither"
        );
    }
}
#[test]
fn literal_length_limits_count_utf8_bytes() {
    for (text, valid) in [
        ("x".repeat(4096), true),
        ("x".repeat(4097), false),
        ("界".repeat(1365), true),
        ("界".repeat(1366), false),
        (String::new(), false),
    ] {
        assert_eq!(
            Rules::new(RuleSpec {
                literal: Some(text),
                ..Default::default()
            })
            .is_ok(),
            valid
        );
    }
}
#[test]
fn regex_size_and_syntax_rejected_before_use() {
    for pattern in [
        "".into(),
        "[".into(),
        "(?=x)".into(),
        "x".repeat(4097),
        "a{999999999}".into(),
    ] {
        assert!(
            Rules::new(RuleSpec {
                pattern: Some(pattern),
                ..Default::default()
            })
            .is_err()
        );
    }
}
#[test]
fn failed_evaluation_does_not_consume_a_rule() {
    let mut rules = Rules::new(RuleSpec {
        literal: Some("DONE".into()),
        ..Default::default()
    })
    .unwrap();
    for _ in 0..100 {
        assert!(rules.evaluate(ZERO, ZERO, "working").is_none());
    }
    assert!(rules.evaluate(ZERO, ZERO, "DONE").is_some());
    assert!(rules.evaluate(ZERO, ZERO, "DONE").is_none());
}
#[test]
fn new_instance_has_independent_fired_state() {
    let spec = RuleSpec {
        pattern: Some("DONE".into()),
        ..Default::default()
    };
    let mut first = Rules::new(spec.clone()).unwrap();
    assert!(first.evaluate(ZERO, ZERO, "DONE").is_some());
    let mut second = Rules::new(spec).unwrap();
    assert!(second.evaluate(ZERO, ZERO, "DONE").is_some());
}
#[test]
fn force_and_grace_configuration_is_preserved() {
    for force in [false, true] {
        for grace in [ZERO, ms(1), Duration::from_secs(86400)] {
            let rules = Rules::new(RuleSpec {
                force,
                grace,
                ..Default::default()
            })
            .unwrap();
            assert_eq!(rules.force(), force);
            assert_eq!(rules.grace(), grace);
        }
    }
}
