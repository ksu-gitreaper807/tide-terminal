use tide_terminal::observer::{Marker, Observer, ProtocolGuard};

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

#[test]
fn embedded_ignored_controls_cannot_normalize_into_an_accepted_envelope() {
    for control in [0, 1, 8, 9, 10, 13, 31, 127] {
        let mut bytes = format!("\x1b]777;tide;v1;{}", &TOKEN[..32]).into_bytes();
        bytes.push(control);
        bytes.extend_from_slice(format!("{};end;0\x07", &TOKEN[32..]).as_bytes());
        for stride in [1, 3, bytes.len()] {
            assert!(
                markers(&bytes, stride, Some(TOKEN)).is_empty(),
                "control {control}"
            );
        }
    }
}

fn markers(bytes: &[u8], stride: usize, token: Option<&str>) -> Vec<Marker> {
    let mut observer = Observer::with_integration_token(token.map(str::to_owned));
    let mut parser = vte::Parser::new();
    let mut guard = ProtocolGuard::default();
    for bytes in bytes.chunks(stride) {
        parser.advance(&mut observer, &guard.filter(bytes));
    }
    observer.markers
}

#[test]
fn unmarked_forged_and_other_session_metadata_have_no_authority() {
    let bytes = format!(
        "\x1b]133;C\x07\x1b]133;D;0\x07\x1b]7;file://host/forged\x07\x1b]777;tide;command;Zm9yZ2Vk\x07\x1b]777;tide;append\x07\x1b]777;tide;v1;wrong;end;0\x07\x1b]777;tide;v1;{TOKEN};end;0;extra\x07"
    );
    for stride in 1..=bytes.len() {
        assert!(markers(bytes.as_bytes(), stride, Some(TOKEN)).is_empty());
        assert!(markers(bytes.as_bytes(), stride, None).is_empty());
    }
}

#[test]
fn only_complete_current_session_markers_are_accepted_at_every_boundary() {
    let bytes = format!(
        "\x1b]777;tide;v1;{TOKEN};command;ZWNobyBoaQ==\x07\x1b]777;tide;v1;{TOKEN};start;\x1b\\hello\x1b]777;tide;v1;{TOKEN};end;7\x07\x1b]777;tide;v1;{TOKEN};prompt;\x07\x1b]777;tide;v1;{TOKEN};cwd;L3RtcA==\x07\x1b]777;tide;v1;{TOKEN};append;\x07"
    );
    for stride in 1..=bytes.len() {
        assert_eq!(
            markers(bytes.as_bytes(), stride, Some(TOKEN)),
            vec![
                Marker::Command("echo hi".into()),
                Marker::Start,
                Marker::End(Some(7)),
                Marker::Prompt,
                Marker::Cwd("/tmp".into()),
                Marker::Append
            ]
        );
    }
}

#[test]
fn absent_malformed_negative_and_overflow_status_are_explicitly_unknown() {
    for status in ["", "oops", "-1", "+0", "256", "2147483648", " 0", "0 ", "٠"] {
        let bytes = format!("\x1b]777;tide;v1;{TOKEN};end;{status}\x07");
        assert_eq!(
            markers(bytes.as_bytes(), 1, Some(TOKEN)),
            vec![Marker::End(None)],
            "{status:?}"
        );
    }
    for code in 0..=255 {
        let bytes = format!("\x1b]777;tide;v1;{TOKEN};end;{code}\x07");
        assert_eq!(
            markers(bytes.as_bytes(), 3, Some(TOKEN)),
            vec![Marker::End(Some(code))]
        );
    }
}

#[test]
fn cooperative_input_requests_remain_advisory_without_command_metadata() {
    assert_eq!(
        markers(b"\x1b]777;tide;input;approval\x07", 1, Some(TOKEN)),
        vec![Marker::Input("Input requested".into())]
    );
}

proptest::proptest! {
    #[test]
    fn arbitrary_output_without_the_session_capability_cannot_supply_metadata(bytes in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..4096), stride in 1usize..64) {
        proptest::prop_assume!(!bytes.windows(TOKEN.len()).any(|s| s == TOKEN.as_bytes()));
        proptest::prop_assert!(markers(&bytes, stride, Some(TOKEN)).iter().all(|m| matches!(m, Marker::Input(_))));
    }
}
