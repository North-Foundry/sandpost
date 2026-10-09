use sandpost_mime::{MimeError, normalize_mailbox};

/// Verify dot-atoms, quoted strings, quoted pairs, and UTF-8 addr-specs normalize correctly.
#[test]
fn normalizes_quoted_and_utf8_mailboxes() {
    let valid_mailboxes = [
        (
            "First.Last+tag@Example.COM",
            "first.last+tag@example.com",
            "example.com",
        ),
        (
            "\"first@last\"@Example.COM",
            "\"first@last\"@example.com",
            "example.com",
        ),
        (
            "\"first\\@last\"@Example.COM",
            "\"first\\@last\"@example.com",
            "example.com",
        ),
        (
            "\"a\\\"b\\\\c\"@Example.COM",
            "\"a\\\"b\\\\c\"@example.com",
            "example.com",
        ),
        (
            "δοκιμή@παράδειγμα.δοκιμή",
            "δοκιμή@παράδειγμα.δοκιμή",
            "παράδειγμα.δοκιμή",
        ),
        ("用户@例子.测试", "用户@例子.测试", "例子.测试"),
        ("\"\"@example.test", "\"\"@example.test", "example.test"),
        (
            "\"first\".last@Example.TEST",
            "\"first\".last@example.test",
            "example.test",
        ),
        (
            "\"with spaces\" \t@ Example.TEST",
            "\"with spaces\"@example.test",
            "example.test",
        ),
        ("\"用@戶\"@例子.测试", "\"用@戶\"@例子.测试", "例子.测试"),
    ];

    for (input_address, expected_address, expected_domain) in valid_mailboxes {
        let mailbox = normalize_mailbox(input_address).unwrap();
        assert_eq!(mailbox.address, expected_address, "{input_address}");
        assert_eq!(mailbox.domain, expected_domain);
    }
}

/// Verify RFC domain literals accept valid IP addresses and syntactic general literals.
#[test]
fn normalizes_valid_domain_literals() {
    let valid_mailboxes = [
        ("user@[192.0.2.1]", "user@[192.0.2.1]"),
        ("user@[IPv6:2001:db8::1]", "user@[ipv6:2001:db8::1]"),
        ("user@[IPv6:::1]", "user@[ipv6:::1]"),
        ("user@[tag:value]", "user@[tag:value]"),
        ("user@[123]", "user@[123]"),
        ("user@[]", "user@[]"),
        ("user@[192.0.2.999]", "user@[192.0.2.999]"),
        ("user@[IPv6:not-an-address]", "user@[ipv6:not-an-address]"),
    ];

    for (input_address, expected_address) in valid_mailboxes {
        let mailbox = normalize_mailbox(input_address).unwrap();
        assert_eq!(mailbox.address, expected_address);
        assert_eq!(mailbox.domain, expected_address.rsplit_once('@').unwrap().1);
    }
}

/// Verify malformed addr-spec syntax, misplaced whitespace, and controls are rejected.
#[test]
fn rejects_malformed_mailboxes() {
    let invalid_mailboxes = [
        "",
        "local",
        "@example.test",
        "local@",
        "local@@example.test",
        "local part@example.test",
        "local@exam ple.test",
        "local..part@example.test",
        ".local@example.test",
        "local.@example.test",
        "\"unfinished@example.test",
        "\"unfinished\\\"@example.test",
        "\"bad\nlocal\"@example.test",
        "\"bad\"quote\"@example.test",
        "local@[IPv6:2001:db8::1",
        "local@example.test\textra",
        "local@example.test\u{7f}",
        "local@example.test\n",
    ];

    for input_address in invalid_mailboxes {
        assert_eq!(
            normalize_mailbox(input_address),
            Err(MimeError::InvalidMailbox(input_address.to_owned())),
            "{input_address:?}"
        );
    }
}
