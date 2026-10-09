# SMTP RFC coverage

This document maps the receiver's implemented behavior to focused tests. It is
an evidence map, not a declaration of complete conformance: SMTP has broad
MUST/SHOULD requirements beyond this catcher’s capture-only role. Section links
point to the official RFC Editor text; each RFC heading also links to its RFC
Editor errata search. The errata pages are the source of record for verified
corrections. This snapshot covers the requirements and policies listed below.

## Scope

The server accepts and captures mail; it does not route mail to MX hosts, retry
outbound delivery, or generate delivery status notifications (DSNs). SMTPUTF8,
CHUNKING/BDAT, BINARYMIME, DSN extensions, client-certificate authentication,
and other unimplemented standard extensions are neither announced nor
accepted. Unicode preparation for AUTH credentials is left to the application
authenticator; this crate frames and validates the SASL exchange. Credential
hashing and verification (including Argon2) are application responsibilities.
PLAIN over cleartext is intentionally configurable for development and local
capture; deployments can require TLS before AUTH and mail. The server does not
support client-certificate authentication. IANA registration and other
administrative actions are outside runtime behavior.

Generated trace fields are covered only for SMTP trace semantics and the
generated header/date syntax. MIME parsing and decoding are implemented and
tested in `sandpost-mime`, outside this crate's RFC 5322 claim.

## RFC 5321 — Simple Mail Transfer Protocol

Source: [RFC 5321](https://www.rfc-editor.org/rfc/rfc5321),
[RFC Editor errata](https://errata.rfc-editor.org/search/?rfc_number=5321).

| Section | Requirement / policy | Evidence |
| --- | --- | --- |
| [3.1, 4.1.1.1](https://www.rfc-editor.org/rfc/rfc5321#section-4.1.1.1) | Send a 220 greeting and distinguish HELO from EHLO operation. | [commands_receive_their_rfc_5321_reply_classes](src/tests/session_compliance.rs) |
| [3.3, 4.1.1.2–4.1.1.4](https://www.rfc-editor.org/rfc/rfc5321#section-4.1.1.2) | Enforce MAIL/RCPT/DATA sequencing and transaction reset behavior. | [null_sender_and_binary_data_are_preserved](src/tests/session_transaction.rs), [persistence_failure_and_success_both_reset_the_transaction](src/tests/session_transaction.rs) |
| [4.1.1.5, 4.1.1.10](https://www.rfc-editor.org/rfc/rfc5321#section-4.1.1.5) | Support RSET and QUIT session control. | [null_sender_and_binary_data_are_preserved](src/tests/session_transaction.rs), [quit_and_disconnect_before_data_do_not_persist](src/tests/session_transaction.rs) |
| [4.1.2, 4.1.3](https://www.rfc-editor.org/rfc/rfc5321#section-4.1.2) | Parse bounded ASCII envelope paths, including null reverse-path, quoted local-parts, source routes, and Postmaster handling. | [mailbox_paths_preserve_local_case_and_support_postmaster](src/tests/command.rs), [quoted_mailboxes_and_source_routes_are_parsed](src/tests/command.rs), [mailbox_grammar_rejects_invalid_bytes_and_enforces_lengths](src/tests/command.rs) |
| [4.1.4, 4.2.1, 4.2.4](https://www.rfc-editor.org/rfc/rfc5321#section-4.1.4) | Apply command sequencing and reply classes; return 500/501/502/252 for unknown, malformed, unimplemented, and non-disclosing VRFY behavior. | [commands_receive_their_rfc_5321_reply_classes](src/tests/session_compliance.rs), [unknown_unimplemented_and_malformed_commands_are_distinguished](src/tests/command.rs) |
| [4.2, 4.3](https://www.rfc-editor.org/rfc/rfc5321#section-4.2) | Return replies in protocol order and use the expected completion, transient, and permanent classes for tested paths. | [commands_receive_their_rfc_5321_reply_classes](src/tests/session_compliance.rs), [persistence_failure_and_success_both_reset_the_transaction](src/tests/session_transaction.rs) |
| [4.4](https://www.rfc-editor.org/rfc/rfc5321#section-4.4) | On capture, prepend one authoritative Return-Path and Received trace while retaining earlier Received fields and submitted content. | [capture_preserves_transport_addresses_and_adds_authoritative_trace](src/tests/session_compliance.rs) |
| [4.5.2](https://www.rfc-editor.org/rfc/rfc5321#section-4.5.2) | Remove SMTP dot transparency before storing message data. | [message_data_unstuffs_single_dot_and_preserves_following_command](src/tests/framing.rs) |
| [4.5.3.1.4–4.5.3.1.10](https://www.rfc-editor.org/rfc/rfc5321#section-4.5.3.1) | Enforce command, reply, DATA line, message, and recipient bounds. | [protocol_lines_enforce_exact_byte_limits_and_preserve_following_lines](src/tests/framing.rs), [message_data_accepts_exact_size_limit_and_rejects_next_byte](src/tests/framing.rs), [recipient_limit_accepts_one_hundred_recipients](src/tests/session_transaction.rs) |
| [4.5.3.2](https://www.rfc-editor.org/rfc/rfc5321#section-4.5.3.2) | Use a five-minute network I/O timeout policy. | [default_command_timeout_is_five_minutes](src/tests/session_compliance.rs) |
| [4.5.5](https://www.rfc-editor.org/rfc/rfc5321#section-4.5.5) | Accept a null reverse-path for captured messages. | [null_sender_and_binary_data_are_preserved](src/tests/session_transaction.rs) |
| [4.1.4](https://www.rfc-editor.org/rfc/rfc5321#section-4.1.4) | Repeated HELO/EHLO and RSET discard transactions; unimplemented extensions are neither advertised nor accepted. | [rfc5321_greetings_and_resets_discard_transactions](tests/rfc_conformance.rs), [rfc5321_unimplemented_extensions_are_not_advertised_or_accepted](tests/rfc_conformance.rs) |

## RFC 2920 — SMTP Command Pipelining

Source: [RFC 2920](https://www.rfc-editor.org/rfc/rfc2920),
[RFC Editor errata](https://errata.rfc-editor.org/search/?rfc_number=2920).

| Section | Requirement / policy | Evidence |
| --- | --- | --- |
| [2, 3](https://www.rfc-editor.org/rfc/rfc2920#section-3) | Advertise PIPELINING and preserve the command/reply boundary when an AUTH initial response is followed by a pipelined NOOP. | [ehlo_advertises_extensions_and_plain_and_login_authentication](src/tests/session_authentication.rs), [plain_initial_response_preserves_pipelined_command_boundary](src/tests/session_authentication.rs) |
| [3.2](https://www.rfc-editor.org/rfc/rfc2920#section-3.2) | Preserve ordered replies under fragmented MAIL/RCPT/DATA pipelines, retain commands after DATA, and avoid delivery when every recipient is rejected. | [rfc2920_pipelining_preserves_order_and_fragmented_input](tests/rfc_conformance.rs), [rfc2920_rejected_recipients_do_not_deliver_and_recovery_keeps_input](tests/rfc_conformance.rs) |

## RFC 1870 — SMTP Service Extension for Message Size Declaration

Source: [RFC 1870](https://www.rfc-editor.org/rfc/rfc1870),
[RFC Editor errata](https://errata.rfc-editor.org/search/?rfc_number=1870).

| Section | Requirement / policy | Evidence |
| --- | --- | --- |
| [2, 3](https://www.rfc-editor.org/rfc/rfc1870#section-3) | Advertise the configured SIZE limit and reject a declared size above it at MAIL; accept the exact declared boundary. | [ehlo_advertises_extensions_and_plain_and_login_authentication](src/tests/session_authentication.rs), [declared_sizes_over_the_limit_are_refused_early](src/tests/session_compliance.rs) |
| [3](https://www.rfc-editor.org/rfc/rfc1870#section-3) | Enforce the limit against bytes received in DATA and count the submitted message before adding generated trace fields. | [data_over_the_configured_limit_is_refused](src/tests/session_limits.rs), [generated_trace_does_not_consume_the_submitted_size_limit](src/tests/session_compliance.rs) |
| [3, 6](https://www.rfc-editor.org/rfc/rfc1870#section-6) | Count unstuffed DATA including CRLF; a declaration is an estimate, and zero capacity must not advertise unlimited SIZE 0. | [rfc1870_size_counts_unstuffed_content_and_enforces_actual_limit](tests/rfc_conformance.rs), [rfc1870_zero_capacity_does_not_advertise_unlimited_size](tests/rfc_conformance.rs) |

## RFC 6152 — SMTP Service Extension for 8-bit MIME Transport

Source: [RFC 6152](https://www.rfc-editor.org/rfc/rfc6152),
[RFC Editor errata](https://errata.rfc-editor.org/search/?rfc_number=6152).

| Section | Requirement / policy | Evidence |
| --- | --- | --- |
| [2, 3](https://www.rfc-editor.org/rfc/rfc6152#section-3) | Advertise 8BITMIME, recognize BODY=8BITMIME syntax, and preserve non-ASCII body octets through DATA. | [ehlo_advertises_extensions_and_plain_and_login_authentication](src/tests/session_authentication.rs), [mail_parameters_follow_size_body_and_auth_extensions](src/tests/command.rs), [null_sender_and_binary_data_are_preserved](src/tests/session_transaction.rs) |
| [2](https://www.rfc-editor.org/rfc/rfc6152#section-2) | Do not claim BINARYMIME support; reject BODY=BINARYMIME. | [commands_receive_their_rfc_5321_reply_classes](src/tests/session_compliance.rs) |
| [3](https://www.rfc-editor.org/rfc/rfc6152#section-3) | Exercise BODY=7BIT and BODY=8BITMIME over TCP, preserving all high-bit octets and dot transparency; reject duplicate and unsupported BODY values. | [rfc6152_eight_bit_mime_preserves_octets_and_transparency](tests/rfc_conformance.rs) |

## RFC 2034 — SMTP Service Extension for Returning Enhanced Error Codes

Source: [RFC 2034](https://www.rfc-editor.org/rfc/rfc2034),
[RFC Editor errata](https://errata.rfc-editor.org/search/?rfc_number=2034).

| Section | Requirement / policy | Evidence |
| --- | --- | --- |
| [2, 4, 5](https://www.rfc-editor.org/rfc/rfc2034#section-4) | Advertise ENHANCEDSTATUSCODES, include enhanced codes in eligible replies even without EHLO negotiation, and match the enhanced class to the SMTP reply class. | [ehlo_advertises_extensions_and_plain_and_login_authentication](src/tests/session_authentication.rs), [commands_receive_their_rfc_5321_reply_classes](src/tests/session_compliance.rs) |
| [4, 5](https://www.rfc-editor.org/rfc/rfc2034#section-4) | Check enhanced status grammar and classes over TCP, before and after EHLO, through authentication, transaction and command failures. | [rfc2034_enhanced_status_codes_match_reply_classes_without_negotiation](tests/rfc_conformance.rs) |

## RFC 3463 — Enhanced Mail System Status Codes

Source: [RFC 3463](https://www.rfc-editor.org/rfc/rfc3463),
[RFC Editor errata](https://errata.rfc-editor.org/search/?rfc_number=3463).

| Section | Requirement / policy | Evidence |
| --- | --- | --- |
| [2, 3](https://www.rfc-editor.org/rfc/rfc3463#section-2) | Format tested enhanced statuses as class.subject.detail and align the class with the SMTP reply class. | [commands_receive_their_rfc_5321_reply_classes](src/tests/session_compliance.rs), [declared_sizes_over_the_limit_are_refused_early](src/tests/session_compliance.rs) |
| [2](https://www.rfc-editor.org/rfc/rfc3463#section-2) | Independently check three numeric components, component lengths, no leading zeroes and matching reply classes. | [rfc2034_enhanced_status_codes_match_reply_classes_without_negotiation](tests/rfc_conformance.rs) |

## RFC 5248 — A Registry for SMTP Enhanced Mail System Status Codes

Source: [RFC 5248](https://www.rfc-editor.org/rfc/rfc5248),
[RFC Editor errata](https://errata.rfc-editor.org/search/?rfc_number=5248).

| Section | Requirement / policy | Evidence |
| --- | --- | --- |
| [2, 3](https://www.rfc-editor.org/rfc/rfc5248#section-3) | Use registered enhanced status values for tested authentication and protocol failures. | [per_credential_rules_answer_538_and_534](src/tests/session_authentication.rs), [commands_receive_their_rfc_5321_reply_classes](src/tests/session_compliance.rs) |
| [2](https://www.rfc-editor.org/rfc/rfc5248#section-2) | Check every emitted literal enhanced status against the IANA assignments used by this receiver; fail on new unreviewed assignments or mismatched classes. | [emitted_enhanced_statuses_use_registered_assignments_and_matching_classes](tests/rfc_coverage.rs) |

## RFC 4954 — SMTP Service Extension for Authentication

Source: [RFC 4954](https://www.rfc-editor.org/rfc/rfc4954),
[RFC Editor errata](https://errata.rfc-editor.org/search/?rfc_number=4954).

| Section | Requirement / policy | Evidence |
| --- | --- | --- |
| [4](https://www.rfc-editor.org/rfc/rfc4954#section-4) | Advertise only the configured AUTH mechanisms and require EHLO before AUTH. | [ehlo_advertises_extensions_and_plain_and_login_authentication](src/tests/session_authentication.rs), [malformed_and_out_of_sequence_authentication_is_refused](src/tests/session_authentication.rs) |
| [4](https://www.rfc-editor.org/rfc/rfc4954#section-4) | Support initial-response and challenge/response framing, including cancellation and invalid Base64 rejection. | [plain_authentication_binds_the_principal_to_delivered_mail](src/tests/session_authentication.rs), [challenge_based_plain_and_login_authenticate](src/tests/session_authentication.rs), [cancelled_plain_and_login_exchanges_allow_authentication_retry](src/tests/session_authentication.rs), [malformed_base64_and_empty_plain_fields_never_reach_authenticator](src/tests/session_authentication.rs) |
| [4, 5](https://www.rfc-editor.org/rfc/rfc4954#section-5) | Apply AUTH sequencing and configured TLS policy; forget pre-TLS negotiation and authentication after STARTTLS. | [start_tls_resets_greeting_and_authentication](src/tests/session_tls.rs), [required_tls_gates_authentication_and_mail](src/tests/session_tls.rs), [per_credential_rules_answer_538_and_534](src/tests/session_authentication.rs) |
| [5](https://www.rfc-editor.org/rfc/rfc4954#section-5) | Mark authenticated captured mail with the ESMTPA trace protocol identifier. | [authenticated_delivery_uses_esmtpa_trace_protocol](src/tests/session_authentication.rs) |

## RFC 4616 — The PLAIN SASL Mechanism

Source: [RFC 4616](https://www.rfc-editor.org/rfc/rfc4616),
[RFC Editor errata](https://errata.rfc-editor.org/search/?rfc_number=4616).

| Section | Requirement / policy | Evidence |
| --- | --- | --- |
| [2](https://www.rfc-editor.org/rfc/rfc4616#section-2) | Decode PLAIN's authorization identity, authentication identity, and password fields and pass the intended principal to the authenticator. | [plain_authentication_binds_the_principal_to_delivered_mail](src/tests/session_authentication.rs), [plain_authentication_accepts_matching_and_refuses_foreign_authorization_identity](src/tests/session_authentication.rs) |
| [2, 4](https://www.rfc-editor.org/rfc/rfc4616#section-2) | Enforce field framing and UTF-8 octet limits; Unicode preparation and credential verification belong to the application layer. | [plain_authentication_enforces_utf8_octet_limits](src/tests/session_authentication.rs), [plain_authentication_accepts_three_255_byte_fields](src/tests/session_authentication.rs) |

## RFC 4648 — The Base16, Base32, and Base64 Data Encodings

Source: [RFC 4648](https://www.rfc-editor.org/rfc/rfc4648),
[RFC Editor errata](https://errata.rfc-editor.org/search/?rfc_number=4648).

| Section | Requirement / policy | Evidence |
| --- | --- | --- |
| [3.1, 4](https://www.rfc-editor.org/rfc/rfc4648#section-4) | Use Base64 for SASL AUTH fields without MIME line wrapping and reject malformed encodings at the SMTP boundary. | [malformed_base64_and_empty_plain_fields_never_reach_authenticator](src/tests/session_authentication.rs), [challenge_based_plain_and_login_authenticate](src/tests/session_authentication.rs) |

## RFC 3207 — SMTP Service Extension for Secure SMTP over TLS

Source: [RFC 3207](https://www.rfc-editor.org/rfc/rfc3207),
[RFC Editor errata](https://errata.rfc-editor.org/search/?rfc_number=3207).

| Section | Requirement / policy | Evidence |
| --- | --- | --- |
| [2, 4](https://www.rfc-editor.org/rfc/rfc3207#section-4) | Advertise STARTTLS only when configured, accept it in sequence, and continue the SMTP session over TLS. | [start_tls_upgrades_the_session_and_mail_flows_encrypted](src/tests/session_tls.rs), [start_tls_is_refused_when_unavailable_or_out_of_sequence](src/tests/session_tls.rs) |
| [4](https://www.rfc-editor.org/rfc/rfc3207#section-4) | After a successful upgrade, require a fresh greeting and discard pre-TLS state; do not process commands sent before the TLS handshake. | [start_tls_resets_greeting_and_authentication](src/tests/session_tls.rs), [commands_pipelined_after_start_tls_close_the_connection](src/tests/session_tls.rs) |
| [4](https://www.rfc-editor.org/rfc/rfc3207#section-4) | Reject STARTTLS arguments and failed handshakes; reset the authentication failure budget after upgrading. | [start_tls_rejects_arguments_and_failed_handshakes](src/tests/session_tls.rs), [start_tls_resets_authentication_failure_budget](src/tests/session_tls.rs) |

## RFC 8314 — Cleartext Considered Obsolete: Use of TLS for Email Submission and Access

Source: [RFC 8314](https://www.rfc-editor.org/rfc/rfc8314),
[RFC Editor errata](https://errata.rfc-editor.org/search/?rfc_number=8314).

| Section | Requirement / policy | Evidence |
| --- | --- | --- |
| [3.3](https://www.rfc-editor.org/rfc/rfc8314#section-3.3) | Support implicit TLS with the handshake before the SMTP greeting. | [implicit_tls_handshakes_before_the_greeting](src/tests/session_tls.rs), [required_start_tls_over_tcp_delivers_authenticated_mail](tests/tls.rs) |
| [5](https://www.rfc-editor.org/rfc/rfc8314#section-5) | Permit deployments to require TLS before AUTH and mail; plaintext PLAIN remains an explicit configurable development/local policy. | [required_tls_gates_authentication_and_mail](src/tests/session_tls.rs), [start_tls_upgrades_the_session_and_mail_flows_encrypted](src/tests/session_tls.rs) |

## RFC 8997 — Deprecation of TLS 1.1 for Email Submission and Access

Source: [RFC 8997](https://www.rfc-editor.org/rfc/rfc8997),
[RFC Editor errata](https://errata.rfc-editor.org/search/?rfc_number=8997).

| Section | Requirement / policy | Evidence |
| --- | --- | --- |
| [2](https://www.rfc-editor.org/rfc/rfc8997#section-2) | Exercise SMTP over TLS 1.2 and TLS 1.3; the server configuration uses rustls safe default protocol versions. | [tls_12_and_tls_13_complete_handshake_and_smtp](src/tests/session_tls.rs) |
| [2](https://www.rfc-editor.org/rfc/rfc8997#section-2) | Refuse TLS 1.0 and 1.1 with a fatal protocol-version alert on both implicit TLS and STARTTLS. | [obsolete_tls_versions_receive_protocol_version_alerts](src/tests/session_tls.rs) |

## RFC 3848 — ESMTP and LMTP Transmission Types Registration

Source: [RFC 3848](https://www.rfc-editor.org/rfc/rfc3848),
[RFC Editor errata](https://errata.rfc-editor.org/search/?rfc_number=3848).

| Section | Requirement / policy | Evidence |
| --- | --- | --- |
| [1](https://www.rfc-editor.org/rfc/rfc3848#section-1) | Label generated Received traces ESMTP for unauthenticated EHLO sessions and ESMTPA for authenticated sessions. | [capture_preserves_transport_addresses_and_adds_authoritative_trace](src/tests/session_compliance.rs), [authenticated_delivery_uses_esmtpa_trace_protocol](src/tests/session_authentication.rs) |
| [1](https://www.rfc-editor.org/rfc/rfc3848#section-1) | Select SMTP/ESMTP/ESMTPA on plaintext and ESMTPS/ESMTPSA over implicit TLS. | [plaintext_received_protocol_tracks_greeting_and_authentication](src/tests/session_tls.rs), [implicit_tls_received_protocol_tracks_authentication](src/tests/session_tls.rs) |

## RFC 5322 — Internet Message Format (generated trace syntax only)

Source: [RFC 5322](https://www.rfc-editor.org/rfc/rfc5322),
[RFC Editor errata](https://errata.rfc-editor.org/search/?rfc_number=5322).

| Section | Requirement / policy | Evidence |
| --- | --- | --- |
| [2.2, 3.6.7](https://www.rfc-editor.org/rfc/rfc5322#section-3.6.7) | Generate syntactically parseable Received and Return-Path fields, including a parseable date; this claim excludes general message/MIME parsing. | [capture_preserves_transport_addresses_and_adds_authoritative_trace](src/tests/session_compliance.rs) |

## Verification and explicit limits

Run `cargo test -p sandpost-smtp --locked` from the workspace root. The RFC
profile includes all 15 specifications above. The
[receiver_rfc_profile_links_to_enabled_tests](tests/rfc_coverage.rs) test rejects
missing RFC sections, missing test evidence, and links to renamed or disabled
tests. The public TCP scenarios use a client that validates replies and DATA
transparency independently of the server's parsers. Tests run offline; the
status-code assignment snapshot comes from the
[IANA registry](https://www.iana.org/assignments/smtp-enhanced-status-codes).

Each listed RFC has executable evidence for the implemented receiver profile.
This does not mean every normative requirement in every RFC has an exhaustive
test. In particular:

- RFC 5321 outbound MX retry, forwarding, and DSN generation are outside this
  capture-only receiver's scope. Timeout recommendations, address grammar and
  interoperability are broader than the specific automated scenarios here.
- RFC 2034 reply tests cover normal transactions and representative failures;
  they do not enumerate every possible runtime failure or multiline response.
- RFC 5248 registry administration is outside runtime scope. Tests validate
  assignments emitted by this receiver, rather than the entire IANA registry.
- RFC 8314 TLS policy is configurable for local/development use. Client
  certificates are not supported.
- RFC 4616 Unicode preparation and credential verification belong to the
  application authenticator, outside this crate's framing tests.
- RFC 5322 coverage is limited to generated trace-field/date syntax. General
  MIME parsing and decoding are separately tested in `sandpost-mime`.

Passing this suite establishes the listed evidence, not universal RFC
conformance or an external certification.
