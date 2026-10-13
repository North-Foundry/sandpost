use super::*;
use crate::protocol::{
    command::{CommandKind, FetchAttribute, SearchKey, Section, SectionText, StatusItem},
    flags::{FlagOperation, SystemFlag},
    sequence::SequenceSet,
};

/// Parse a command written as text.
fn parse(text: &str) -> Result<Command, ParseError> {
    parse_command(text.as_bytes())
}

/// Parse a command and return only its kind.
fn kind(text: &str) -> CommandKind {
    parse(text)
        .unwrap_or_else(|error| panic!("{text}: {error:?}"))
        .kind
}

/// Simple commands parse case-insensitively and keep their tag.
#[test]
fn simple_commands_and_tags() {
    let command = parse("a001 capability").unwrap();
    assert_eq!(command.tag, "a001");
    assert_eq!(command.kind, CommandKind::Capability);
    assert_eq!(kind("A.1 NOOP"), CommandKind::Noop);
    assert_eq!(kind("x LOGOUT"), CommandKind::Logout);
    assert_eq!(
        kind("x Uid Expunge 1:3").clone(),
        CommandKind::Expunge {
            uids: SequenceSet::parse("1:3"),
        }
    );
}

/// Bad tags produce untagged errors; bad commands keep their tag.
#[test]
fn errors_carry_the_tag_when_known() {
    assert_eq!(parse("").unwrap_err().tag, None);
    assert_eq!(parse("+tag NOOP").unwrap_err().tag, None);
    assert_eq!(
        parse("a1 FROBNICATE").unwrap_err().tag.as_deref(),
        Some("a1")
    );
    assert!(parse("a1 NOOP extra").is_err());
    assert!(parse("a1  NOOP").is_err(), "double space");
}

/// LOGIN accepts atoms, quoted strings with escapes, and literals.
#[test]
fn login_arguments_use_astring_forms() {
    assert_eq!(
        kind("a LOGIN fred \"pa\\\"ss\\\\word\""),
        CommandKind::Login {
            username: "fred".into(),
            password: "pa\"ss\\word".into()
        }
    );
    assert_eq!(
        kind("a LOGIN {11}\r\nFRED FOOBAR {7+}\r\nfat man"),
        CommandKind::Login {
            username: "FRED FOOBAR".into(),
            password: "fat man".into()
        }
    );
    assert!(parse("a LOGIN \"unterminated").is_err());
    assert!(parse("a LOGIN \"bad\\escape\" x").is_err());
    assert!(parse("a LOGIN {20}\r\nshort").is_err());
    assert!(parse("a LOGIN {1}\nx {1}\ny").is_err());
    assert!(parse("a LOGIN {1}\r\n\0 {1}\r\nx").is_err());
}

/// Mailbox names normalize INBOX and decode modified UTF-7.
#[test]
fn mailbox_names_are_decoded() {
    assert_eq!(
        kind("a SELECT inbox"),
        CommandKind::Select {
            mailbox: "INBOX".into(),
            read_only: false
        }
    );
    assert_eq!(
        kind("a EXAMINE \"Caf&AOk-\""),
        CommandKind::Select {
            mailbox: "Caf\u{e9}".into(),
            read_only: true
        }
    );
    assert!(parse("a SELECT &Jjo").is_err());
    assert!(parse("a SELECT INBOX (CONDSTORE)").is_err());
}

/// LIST accepts wildcards and empty strings; LIST-EXTENDED syntax is refused.
#[test]
fn list_patterns_and_extended_syntax() {
    assert_eq!(
        kind("a LIST \"\" *"),
        CommandKind::List {
            reference: String::new(),
            pattern: "*".into(),
            subscribed_only: false
        }
    );
    assert_eq!(
        kind("a LSUB \"\" %"),
        CommandKind::List {
            reference: String::new(),
            pattern: "%".into(),
            subscribed_only: true
        }
    );
    assert!(parse("a LIST (SPECIAL-USE) \"\" *").is_err());
    assert!(parse("a LIST \"\" * RETURN (CHILDREN)").is_err());
}

/// STATUS items deduplicate and unknown items are refused.
#[test]
fn status_items() {
    assert_eq!(
        kind("a STATUS INBOX (MESSAGES UNSEEN messages)"),
        CommandKind::Status {
            mailbox: "INBOX".into(),
            items: vec![StatusItem::Messages, StatusItem::Unseen]
        }
    );
    assert!(parse("a STATUS INBOX (HIGHESTMODSEQ)").is_err());
    assert!(parse("a STATUS INBOX ()").is_err());
}

/// FETCH parses macros, sections, partial ranges, and header field lists.
#[test]
fn fetch_items_and_sections() {
    let CommandKind::Fetch { attributes, .. } = kind("a FETCH 1:* FAST") else {
        panic!()
    };
    assert_eq!(attributes.len(), 3);
    let CommandKind::Fetch {
        uid, attributes, ..
    } = kind(
        "a UID FETCH 1:* (UID FLAGS BODY.PEEK[HEADER.FIELDS (From Subject)] BODY[1.2.MIME] BODY[]<0.1024> BODY[TEXT] BODY)",
    )
    else {
        panic!()
    };
    assert!(uid);
    assert_eq!(
        attributes[2],
        FetchAttribute::Section {
            peek: true,
            section: Section {
                part: vec![],
                text: Some(SectionText::HeaderFields {
                    not: false,
                    fields: vec!["From".into(), "Subject".into()]
                })
            },
            partial: None
        }
    );
    assert_eq!(
        attributes[3],
        FetchAttribute::Section {
            peek: false,
            section: Section {
                part: vec![1, 2],
                text: Some(SectionText::Mime)
            },
            partial: None
        }
    );
    assert_eq!(
        attributes[4],
        FetchAttribute::Section {
            peek: false,
            section: Section {
                part: vec![],
                text: None
            },
            partial: Some((0, 1024))
        }
    );
    assert_eq!(attributes[6], FetchAttribute::Body);
    for invalid in [
        "a FETCH 1 (BODY[MIME])",
        "a FETCH 1 (BODY[0])",
        "a FETCH 1 (BODY[TEXT.1])",
        "a FETCH 1 (BODY[]<0.0>)",
        "a FETCH 1 (BINARY[1])",
        "a FETCH 0 FLAGS",
        "a FETCH 1 ()",
    ] {
        assert!(parse(invalid).is_err(), "{invalid}");
    }
}

/// STORE parses operations, silence, bare and parenthesized flags.
#[test]
fn store_operations_and_flags() {
    let CommandKind::Store { change, silent, .. } =
        kind("a STORE 2:4 +FLAGS.SILENT (\\Deleted $Junk)")
    else {
        panic!()
    };
    assert!(silent);
    assert_eq!(change.operation, FlagOperation::Add);
    assert!(change.flags.contains(SystemFlag::Deleted));
    assert_eq!(change.flags.keywords(), ["$Junk"]);
    let CommandKind::Store { change, silent, .. } = kind("a UID STORE 9 -FLAGS \\Seen \\Flagged")
    else {
        panic!()
    };
    assert!(!silent);
    assert_eq!(change.operation, FlagOperation::Remove);
    assert!(change.flags.contains(SystemFlag::Flagged));
    let CommandKind::Store { change, .. } = kind("a STORE 1 FLAGS ()") else {
        panic!()
    };
    assert!(change.flags.is_empty());
    assert!(parse("a STORE 1 +FLAGS (\\Recent)").is_err());
    assert!(parse("a STORE 1 +FLAGS (\\*)").is_err());
    assert!(parse("a STORE 1 X-GM-LABELS (x)").is_err());
}

/// SEARCH supports nesting, OR, NOT, dates, sequence sets, and CHARSET.
#[test]
fn search_keys() {
    let CommandKind::Search {
        charset, criteria, ..
    } = kind(
        "a SEARCH CHARSET utf-8 OR (FROM alice SINCE 1-Feb-1994) NOT 2:4 HEADER X-App \"bravo\" UID 1:*",
    )
    else {
        panic!()
    };
    assert_eq!(charset.as_deref(), Some("UTF-8"));
    let SearchKey::And(keys) = criteria else {
        panic!()
    };
    assert_eq!(keys.len(), 3);
    assert!(matches!(keys[0], SearchKey::Or(..)));
    assert_eq!(keys[1], SearchKey::Header("X-App".into(), "bravo".into()));
    assert!(parse("a SEARCH").is_err());
    assert!(parse("a SEARCH FROM").is_err());
    assert!(parse("a SEARCH SINCE 31-Feb-2024").is_err());
    assert!(parse("a SEARCH RETURN (MIN) ALL").is_err());
    let deep = format!("a SEARCH {}ALL{}", "(".repeat(40), ")".repeat(40));
    assert!(parse(&deep).is_err());
    let wide = format!("a SEARCH {}", vec!["SEEN"; 300].join(" "));
    assert!(parse(&wide).is_err());
}

/// AUTHENTICATE accepts an optional base64 initial response.
#[test]
fn authenticate_initial_response() {
    assert_eq!(
        kind("a AUTHENTICATE plain AGZyZWQAc2VjcmV0"),
        CommandKind::Authenticate {
            mechanism: "PLAIN".into(),
            initial_response: Some("AGZyZWQAc2VjcmV0".into())
        }
    );
    assert_eq!(
        kind("a AUTHENTICATE PLAIN ="),
        CommandKind::Authenticate {
            mechanism: "PLAIN".into(),
            initial_response: Some("=".into())
        }
    );
    assert!(parse("a AUTHENTICATE PLAIN YQ==").is_ok());
    for invalid in ["A", "YQ=", "YQ===", "=AAA", "AA=A"] {
        assert!(parse(&format!("a AUTHENTICATE PLAIN {invalid}")).is_err());
    }
    assert!(parse("a AUTHENTICATE PLAIN YQ!").is_err());
}

/// APPEND validates flags, date-time, and its literal; ID validates its parameters.
#[test]
fn append_and_id() {
    assert_eq!(
        kind("a APPEND Trash (\\Seen) \" 7-Feb-1994 21:52:25 -0800\" {5}\r\nHello"),
        CommandKind::Append {
            mailbox: "Trash".into()
        }
    );
    assert!(parse("a APPEND Trash \"bad date\" {1}\r\nx").is_err());
    assert_eq!(
        kind("a ID (\"name\" \"Thunderbird\" \"version\" NIL)"),
        CommandKind::Id
    );
    assert_eq!(kind("a ID NIL"), CommandKind::Id);
    assert_eq!(kind("a ID ()"), CommandKind::Id);
    assert!(parse("a ID (\"name\")").is_err());
    assert!(parse("a ID (\"name\" NIL \"NAME\" NIL)").is_err());
    assert!(parse(&format!("a ID (\"name\" \"{}\")", "x".repeat(1024))).is_ok());
    assert!(parse(&format!("a ID (\"name\" \"{}\")", "x".repeat(1025))).is_err());
}

/// Prefix parsing identifies commands before their literals arrive.
#[test]
fn prefixes_identify_partial_commands() {
    assert_eq!(
        command_prefix(b"a1 APPEND INBOX {300}"),
        Some(("a1".into(), "APPEND".into()))
    );
    assert_eq!(
        command_prefix(b"a1 uid search CHARSET UTF-8 TEXT {4}"),
        Some(("a1".into(), "UID SEARCH".into()))
    );
    assert_eq!(
        append_mailbox_prefix(b"a1 APPEND Trash {300}").as_deref(),
        Some("Trash")
    );
}
