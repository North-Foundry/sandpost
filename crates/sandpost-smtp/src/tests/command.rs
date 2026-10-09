//! Command line syntax, independent of session state.
use crate::session::{
    ArgumentError, Command, MailRequest,
    mailbox::{parse_mailbox, parse_path},
};
use sandpost_core::Mailbox;

/// Parse a mailbox that the test declares valid.
fn mailbox(address: &str) -> Mailbox {
    parse_mailbox(address).unwrap()
}

/// Build the expected request for a sender without ESMTP parameters.
fn sender(address: Option<&str>) -> Command<'static> {
    Command::Mail {
        request: Ok(MailRequest {
            sender: address.map(mailbox),
            declared_size: None,
            has_parameters: false,
        }),
    }
}

/// Command names are case-insensitive and may follow leading ASCII whitespace.
#[test]
fn commands_are_recognized_in_any_case_after_leading_ascii_whitespace() {
    assert_eq!(
        Command::parse("EHLO client"),
        Command::Hello {
            extended: true,
            domain: "client"
        }
    );
    assert_eq!(
        Command::parse("  helo [IPv6:2001:db8::1]"),
        Command::Hello {
            extended: false,
            domain: "[IPv6:2001:db8::1]"
        }
    );
    assert_eq!(
        Command::parse("\tauth PLAIN  AGFiYwBkZWY= "),
        Command::Authenticate {
            argument: "PLAIN  AGFiYwBkZWY="
        }
    );
    assert_eq!(Command::parse("starttls"), Command::StartTls);
    assert_eq!(Command::parse("data"), Command::Data);
    assert_eq!(Command::parse("RSET"), Command::Reset);
    assert_eq!(Command::parse("noop anything"), Command::NoOperation);
    assert_eq!(Command::parse("Quit"), Command::Quit);
    assert_eq!(Command::parse("VRFY postmaster"), Command::Verify);
    assert_eq!(Command::parse("help"), Command::Help);
}

/// RFC 5321 reply classes distinguish unknown, unimplemented, and malformed commands.
#[test]
fn unknown_unimplemented_and_malformed_commands_are_distinguished() {
    for command_line in ["", "   ", "FROBNICATE", "X-CUSTOM 1"] {
        assert_eq!(
            Command::parse(command_line),
            Command::Unrecognized,
            "{command_line:?}"
        );
    }
    for command_line in ["EXPN list", "TURN", "ETRN example.test", "BDAT 10 LAST"] {
        assert_eq!(
            Command::parse(command_line),
            Command::NotImplemented,
            "{command_line:?}"
        );
    }
    for command_line in [
        "EHLO",
        "HELO  ",
        "EHLO bad..domain",
        "EHLO client trailing",
        "STARTTLS now",
        "MAIL TO:<a@b.c>",
        "RCPT FROM:<a@b.c>",
        "DATA now",
        "RSET everything",
        "QUIT now",
        "VRFY",
    ] {
        assert!(
            matches!(Command::parse(command_line), Command::InvalidSyntax(_)),
            "{command_line:?}"
        );
    }
}

/// SMTP paths preserve local-part case and lowercase only the domain.
#[test]
fn mailbox_paths_preserve_local_case_and_support_postmaster() {
    assert_eq!(Command::parse("MAIL FROM:<>"), sender(None));
    assert_eq!(
        Command::parse("mail from: <App@Example.TEST>"),
        sender(Some("App@example.test"))
    );
    assert_eq!(
        Command::parse("RCPT TO:<QA@Example.test>"),
        Command::Recipient {
            recipient: Ok(mailbox("QA@example.test"))
        }
    );
    assert_eq!(
        Command::parse("RCPT TO:<Postmaster>"),
        Command::Recipient {
            recipient: Ok(Mailbox {
                address: "Postmaster".to_owned(),
                domain: String::new()
            })
        }
    );
}

/// Quoted local-parts may contain separators and escapes, and obsolete routes are validated.
#[test]
fn quoted_mailboxes_and_source_routes_are_parsed() {
    assert_eq!(
        Command::parse("RCPT TO:<\"A@b>c\\\"d\"@Example.TEST>"),
        Command::Recipient {
            recipient: Ok(mailbox("\"A@b>c\\\"d\"@example.test"))
        }
    );
    assert_eq!(
        Command::parse("RCPT TO:<@route.test,@route2.test:User@DEST.test>"),
        Command::Recipient {
            recipient: Ok(mailbox("User@dest.test"))
        }
    );
    assert_eq!(
        Command::parse("RCPT TO:<User@[192.0.2.1]>"),
        Command::Recipient {
            recipient: Ok(mailbox("User@[192.0.2.1]"))
        }
    );
    assert_eq!(
        Command::parse("RCPT TO:<@bad..route:User@dest.test>"),
        Command::Recipient {
            recipient: Err(ArgumentError::InvalidPath)
        }
    );
}

/// ESMTP parameters are parsed once, with presence retained for session negotiation checks.
#[test]
fn mail_parameters_follow_size_body_and_auth_extensions() {
    assert_eq!(
        Command::parse("MAIL FROM:<App@example.test> size=2048 BODY=8BITMIME AUTH=<>"),
        Command::Mail {
            request: Ok(MailRequest {
                sender: Some(mailbox("App@example.test")),
                declared_size: Some(2048),
                has_parameters: true,
            })
        }
    );
    assert_eq!(
        Command::parse("MAIL FROM:<> BODY=7bit"),
        Command::Mail {
            request: Ok(MailRequest {
                sender: None,
                declared_size: None,
                has_parameters: true,
            })
        }
    );
    assert_eq!(
        Command::parse("MAIL FROM:<> AUTH=User+40Example.TEST"),
        Command::Mail {
            request: Ok(MailRequest {
                sender: None,
                declared_size: None,
                has_parameters: true,
            })
        }
    );
    assert_eq!(
        Command::parse("MAIL FROM:<> SIZE=99999999999999999999"),
        Command::Mail {
            request: Ok(MailRequest {
                sender: None,
                declared_size: Some(usize::MAX),
                has_parameters: true,
            })
        }
    );
    for (command_line, error) in [
        ("MAIL FROM:<> SIZE=", ArgumentError::InvalidParameter),
        ("MAIL FROM:<> SIZE=+5", ArgumentError::InvalidParameter),
        (
            "MAIL FROM:<> SIZE=123456789012345678901",
            ArgumentError::InvalidParameter,
        ),
        (
            "MAIL FROM:<> SIZE=1 SIZE=2",
            ArgumentError::InvalidParameter,
        ),
        (
            "MAIL FROM:<> BODY=BINARYMIME",
            ArgumentError::InvalidParameter,
        ),
        (
            "MAIL FROM:<> BODY=7BIT BODY=8BITMIME",
            ArgumentError::InvalidParameter,
        ),
        ("MAIL FROM:<> AUTH=", ArgumentError::InvalidParameter),
        (
            "MAIL FROM:<> AUTH=user+4Gexample.test",
            ArgumentError::InvalidParameter,
        ),
        ("MAIL FROM:<> AUTH=user+", ArgumentError::InvalidParameter),
        (
            "MAIL FROM:<> AUTH=Postmaster",
            ArgumentError::InvalidParameter,
        ),
        (
            "MAIL FROM:<> AUTH=@route.test:user@example.test",
            ArgumentError::InvalidParameter,
        ),
        (
            "MAIL FROM:<> AUTH=user+40bad..domain",
            ArgumentError::InvalidParameter,
        ),
        (
            "MAIL FROM:<> AUTH=user+00@example.test",
            ArgumentError::InvalidParameter,
        ),
        (
            "MAIL FROM:<> AUTH=user@example.test AUTH=<>",
            ArgumentError::InvalidParameter,
        ),
        ("MAIL FROM:<> -SIZE=1", ArgumentError::InvalidParameter),
        ("MAIL FROM:<> SIZE=1=2", ArgumentError::InvalidParameter),
        (
            "MAIL FROM:<> SIZE=one\u{00a0}BODY=7BIT",
            ArgumentError::InvalidParameter,
        ),
        (
            "MAIL FROM:<> SIZE=1\u{00a0}",
            ArgumentError::InvalidParameter,
        ),
        ("MAIL FROM:<> SMTPUTF8", ArgumentError::UnsupportedParameter),
        ("MAIL FROM:<> trailing", ArgumentError::UnsupportedParameter),
    ] {
        assert_eq!(
            Command::parse(command_line),
            Command::Mail {
                request: Err(error)
            },
            "{command_line:?}"
        );
    }
    assert_eq!(
        Command::parse("RCPT TO:<qa@example.test> NOTIFY=NEVER"),
        Command::Recipient {
            recipient: Err(ArgumentError::UnsupportedParameter)
        }
    );
}

/// Mailbox syntax enforces SMTP ASCII and the local, domain, and path byte limits.
#[test]
fn mailbox_grammar_rejects_invalid_bytes_and_enforces_lengths() {
    for address in [
        "a..b@example.test",
        ".a@example.test",
        "a.@example.test",
        "a b@example.test",
        "\"unterminated@example.test",
        "\"bad\\\nlocal\"@example.test",
        "a@bad..domain",
        "a@-bad.test",
        "a@bad-.test",
        "a@例え.test",
        "Postmaster",
        "@route.test:a@example.test",
        "a@[invalid:literal]",
        "a@[256.0.0.1]",
    ] {
        assert!(parse_mailbox(address).is_err(), "{address:?}");
    }
    assert!(parse_mailbox("a@[001.002.003.004]").is_ok());
    assert!(parse_path("@route.test,@second.test:a@example.test").is_ok());
    assert!(parse_path("@[192.0.2.1]:a@example.test").is_err());
    assert!(parse_mailbox(&format!("{}@example.test", "a".repeat(64))).is_ok());
    assert!(parse_mailbox(&format!("{}@example.test", "a".repeat(65))).is_err());
    assert!(
        parse_mailbox(&format!(
            "a@{}",
            [
                "a".repeat(63),
                "a".repeat(63),
                "a".repeat(63),
                "a".repeat(59)
            ]
            .join(".")
        ))
        .is_ok()
    );
    assert!(
        parse_mailbox(&format!(
            "a@{}",
            (0..4).map(|_| "a".repeat(63)).collect::<Vec<_>>().join(".") + ".a"
        ))
        .is_err()
    );
    let route_domain = [
        "a".repeat(63),
        "b".repeat(63),
        "c".repeat(63),
        "d".repeat(57),
    ]
    .join(".");
    assert_eq!(route_domain.len(), 249);
    assert!(parse_path(&format!("@{route_domain}:a@x")).is_ok());
    assert!(parse_path(&format!("@{route_domain}:aa@x")).is_err());
}

/// Invalid greeting domains and non-ASCII command whitespace do not pass grammar checks.
#[test]
fn greeting_and_path_control_bytes_are_rejected() {
    for command_line in ["EHLO bad_domain", "EHLO [IPv6:nope]", "EHLO clïent"] {
        assert!(matches!(
            Command::parse(command_line),
            Command::InvalidSyntax(_)
        ));
    }
    for path in ["a@b\n.test", "a@b\u{007f}.test", "a@bücher.test"] {
        assert_eq!(
            Command::parse(&format!("RCPT TO:<{path}>")),
            Command::Recipient {
                recipient: Err(ArgumentError::InvalidPath)
            },
            "{path:?}"
        );
    }
}

/// Malformed paths remain errors for the session to report after its state checks.
#[test]
fn malformed_paths_are_invalid() {
    for path in [
        "sender@example.com",
        "<sender@example.com",
        "<sender@example.com>glued",
        "<no-domain@>",
        "<Postmaster>",
    ] {
        assert_eq!(
            Command::parse(&format!("MAIL FROM:{path}")),
            Command::Mail {
                request: Err(ArgumentError::InvalidPath)
            },
            "{path}"
        );
    }
    for path in [
        "sender@example.com",
        "<sender@example.com",
        "<sender@example.com>glued",
        "<no-domain@>",
        "<>",
    ] {
        assert_eq!(
            Command::parse(&format!("RCPT TO:{path}")),
            Command::Recipient {
                recipient: Err(ArgumentError::InvalidPath)
            },
            "{path}"
        );
    }
}
