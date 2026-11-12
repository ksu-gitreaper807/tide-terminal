use anyhow::{Result, ensure};
use regex::Regex;
use std::time::Duration;

#[derive(Clone, Debug, Default)]
pub struct RuleSpec {
    pub timeout: Option<Duration>,
    pub idle: Option<Duration>,
    pub literal: Option<String>,
    pub pattern: Option<String>,
    pub grace: Duration,
    pub force: bool,
}

pub struct Rules {
    spec: RuleSpec,
    regex: Option<Regex>,
    fired: bool,
}

impl Rules {
    pub fn new(spec: RuleSpec) -> Result<Self> {
        ensure!(
            spec.literal
                .as_ref()
                .is_none_or(|s| !s.is_empty() && s.len() <= 4096),
            "Output marker must contain 1–4096 bytes"
        );
        ensure!(
            spec.pattern
                .as_ref()
                .is_none_or(|s| !s.is_empty() && s.len() <= 4096),
            "Regex must contain 1–4096 bytes"
        );
        let regex = spec
            .pattern
            .as_deref()
            .map(|p| regex::RegexBuilder::new(p).size_limit(2_000_000).build())
            .transpose()?;
        Ok(Self {
            spec,
            regex,
            fired: false,
        })
    }

    pub fn evaluate(&mut self, elapsed: Duration, quiet: Duration, output: &str) -> Option<String> {
        if self.fired {
            return None;
        }
        let reason = if self.spec.timeout.is_some_and(|d| elapsed >= d) {
            Some("Runtime limit reached")
        } else if self.spec.idle.is_some_and(|d| quiet >= d) {
            Some("Output inactivity limit reached")
        } else if self
            .spec
            .literal
            .as_ref()
            .is_some_and(|s| output.contains(s))
        {
            Some("Output marker matched")
        } else if self.regex.as_ref().is_some_and(|r| r.is_match(output)) {
            Some("Output pattern matched")
        } else {
            None
        };
        if reason.is_some() {
            self.fired = true;
        }
        reason.map(str::to_owned)
    }

    pub fn grace(&self) -> Duration {
        self.spec.grace
    }
    pub fn force(&self) -> bool {
        self.spec.force
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clocks_and_rules_are_once_per_run() {
        let mut r = Rules::new(RuleSpec {
            timeout: Some(Duration::from_secs(2)),
            ..Default::default()
        })
        .unwrap();
        assert!(
            r.evaluate(Duration::from_secs(1), Duration::ZERO, "")
                .is_none()
        );
        assert!(
            r.evaluate(Duration::from_secs(2), Duration::ZERO, "")
                .is_some()
        );
        assert!(
            r.evaluate(Duration::from_secs(3), Duration::ZERO, "")
                .is_none()
        );
        let mut next = Rules::new(RuleSpec::default()).unwrap();
        assert!(
            next.evaluate(Duration::from_secs(20), Duration::from_secs(20), "")
                .is_none()
        );
    }
    #[test]
    fn patterns_and_idle_are_independent() {
        let mut r = Rules::new(RuleSpec {
            pattern: Some("FATAL:[^\\n]+".into()),
            ..Default::default()
        })
        .unwrap();
        assert!(r.evaluate(Duration::ZERO, Duration::ZERO, "ok").is_none());
        assert!(
            r.evaluate(Duration::ZERO, Duration::ZERO, "ok\nFATAL: broken")
                .is_some()
        );
        let mut r = Rules::new(RuleSpec {
            idle: Some(Duration::from_secs(1)),
            ..Default::default()
        })
        .unwrap();
        assert!(
            r.evaluate(Duration::from_secs(5), Duration::from_millis(999), "")
                .is_none()
        );
        assert!(
            r.evaluate(Duration::from_secs(5), Duration::from_secs(1), "")
                .is_some()
        );
    }
}
