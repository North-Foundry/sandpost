use super::*;

/// Flags convert losslessly between storage and protocol representations.
#[test]
fn flags_round_trip() {
    let stored = ImapMessageFlags {
        seen: true,
        answered: false,
        flagged: true,
        draft: false,
        deleted: true,
        keywords: vec!["$Forwarded".into(), "$Junk".into()],
    };
    let protocol = protocol_flags(stored.clone());
    assert!(protocol.contains(SystemFlag::Seen) && protocol.contains(SystemFlag::Deleted));
    assert!(!protocol.contains(SystemFlag::Answered));
    assert_eq!(stored_flags(&protocol), stored);
}
