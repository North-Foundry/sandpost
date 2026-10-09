# MIME receiver profile

This document describes selected behavior of `sandpost-mime` when it captures
and interprets message bytes. It is an implementation profile, not a claim of
full compliance with any RFC, an exhaustive conformance test suite, or certification.
Evidence names regression tests for the specific behavior stated; it does not
prove every requirement in a referenced RFC. Parsing is intentionally
permissive for recoverable malformed input. `raw_message` retains the original
captured bytes, including bytes that the parser does not understand.

## Scope

The crate extracts message headers and selected text/HTML bodies, describes
attachments, and resolves `cid:` and `mid:` references within the supplied
message. Unknown transfer encodings and unsupported or unknown text charsets
are represented as opaque attachment data rather than decoded text. Multipart
handling includes nested-boundary recovery, related-root selection through
`start`, and choosing the last supported body in `multipart/alternative`.
Content references decode percent-escaped identifiers and return decoded
content bytes; a `mid:` reference can return the original whole message.

Sender-only behavior, MUA rendering and user-interface behavior, cryptographic
signing or verification, and complete message reassembly are outside this
receiver profile. In particular, embedded `message/rfc822` and `message/global`
parts can be captured and exposed as attachment data; that is not a claim that
the crate fully processes or reassembles those message formats.

## RFC profile

The evidence below is deliberately limited to the named regression cases.
`N/A` describes subject matter outside this parser's role; linked tests in those
rows demonstrate opaque media capture only and are not evidence of registry
administration behavior.

## RFC 2045 — MIME Part One: Format of Internet Message Bodies

Profile: selected transfer-encoding and charset interpretation, text decoding,
case-insensitive token handling, and opaque fallback for unknown transfer
encodings and unsupported/unknown charsets, including comments and folding in
transfer-encoding fields. This is not a complete test of RFC 2045 requirements.

Evidence: `rfc_conformance.rs::rfc2045_defaults_to_plain_ascii_and_seven_bit_identity`, `rfc_conformance.rs::rfc2045_identity_transfer_encodings_preserve_captured_octets`, `rfc_conformance.rs::rfc2045_decodes_base64_and_quoted_printable_text`, `rfc_conformance.rs::rfc2045_converts_declared_legacy_charsets_without_changing_raw_bytes`, `rfc_conformance.rs::rfc2045_unknown_transfer_encoding_recovers_without_decoding_as_text`, `rfc_conformance.rs::mime_type_subtype_charset_and_transfer_tokens_are_case_insensitive`.

Source: [RFC 2045](https://www.rfc-editor.org/rfc/rfc2045).

Evidence: `receiver_rules.rs::unknown_charset_and_encoding_are_opaque_attachments`, `receiver_rules.rs::transfer_encoding_comments_and_folding_do_not_change_decoding`.

## RFC 2046 — MIME Part Two: Media Types

Profile: selected nested multipart, preamble/epilogue, alternative, parallel,
unknown-subtype, and embedded-message capture behavior. The tests do not
establish complete media-type semantics or strict rejection of malformed
multipart input.

Evidence: `rfc_conformance.rs::rfc2046_mixed_multipart_nests_and_discards_preamble_and_epilogue`, `rfc_conformance.rs::rfc2046_alternative_multipart_extracts_text_and_html_representations`, `rfc_conformance.rs::rfc2046_parallel_multipart_keeps_text_parts_in_the_message_facts`, `rfc_conformance.rs::rfc2046_unknown_multipart_subtype_still_parses_its_parts`, `rfc_conformance.rs::rfc2046_nested_multipart_recovers_at_the_outer_boundary`, `rfc_conformance.rs::rfc2046_embedded_message_rfc822_preserves_and_extracts_inner_body`.

Source: [RFC 2046](https://www.rfc-editor.org/rfc/rfc2046).

Evidence: `receiver_rules.rs::outer_boundaries_recover_inside_embedded_messages`.

Evidence: `rfc_conformance.rs::rfc2046_digest_defaults_each_part_to_an_embedded_message`, `receiver_rules.rs::alternatives_choose_the_last_supported_body_of_each_kind`, `receiver_rules.rs::recovery_does_not_insert_bytes_into_multipart_attachments`, `receiver_rules.rs::deeply_nested_multipart_keeps_its_body_and_raw_bytes`, `boundaries.rs::recovers_one_missing_inner_close_before_outer_opening_delimiter`, `boundaries.rs::recovers_two_missing_nested_closes_before_outer_closing_delimiter`, `boundaries.rs::distinguishes_prefix_collisions_and_accepts_delimiter_trailing_whitespace`, `boundaries.rs::does_not_discover_multipart_headers_inside_a_text_body`.

## RFC 2047 — MIME Part Three: Message Header Extensions for Non-ASCII Text

Profile: selected Base64 and Q encoded-word decoding, including adjacent and
folded encoded words, in subjects, extension headers, and display names. This
does not cover every permitted header context or malformed encoded-word rule.

Evidence: `rfc_conformance.rs::rfc2047_decodes_base64_and_quoted_printable_header_words`, `rfc_conformance.rs::rfc2047_joins_adjacent_and_folded_encoded_words`.

Source: [RFC 2047](https://www.rfc-editor.org/rfc/rfc2047).

## RFC 2048 — MIME Registration Procedures (historical)

Profile: N/A. Registration procedures and registry administration are not
receiver parsing behavior. The receiver can retain opaque media without
asserting that a media type or encoding is registered or valid.

Evidence: `rfc_conformance.rs::rfc2045_unknown_transfer_encoding_recovers_without_decoding_as_text`.

Source: [RFC 2048](https://www.rfc-editor.org/rfc/rfc2048).

## RFC 2049 — MIME Part Five: Conformance Criteria and Examples

Profile: selected nested structure based on an Appendix A style message
fixture. The case checks extracted text and preservation of the captured end;
it does not validate every conformance criterion or reproduce a full RFC
certification suite.

Evidence: `rfc_conformance.rs::rfc2049_appendix_a_complex_multipart_fixture`.

Source: [RFC 2049](https://www.rfc-editor.org/rfc/rfc2049).

## RFC 2183 — Communicating Presentation Information in Internet Messages

Profile: selected attachment disposition and filename metadata, plus inline
related image classification. This is not a complete implementation of
presentation or disposition parameters.

Evidence: `attachments.rs::handles_missing_metadata_disposition_and_extended_filename_parameters`, `attachments.rs::classifies_related_inline_images_as_attachments_with_decoded_hashes`.

Source: [RFC 2183](https://www.rfc-editor.org/rfc/rfc2183).

## RFC 2231 — MIME Parameter Value and Encoded Word Extensions

Profile: UTF-8 and legacy charset extended filenames, language tags, encoded,
plain and mixed continuation segments, and language-tagged encoded words.

Evidence: `attachments.rs::handles_missing_metadata_disposition_and_extended_filename_parameters`.

Source: [RFC 2231](https://www.rfc-editor.org/rfc/rfc2231).

Evidence: `attachments.rs::filenames_decode_languages_legacy_charsets_and_mixed_continuations`.

## RFC 2387 — The MIME Multipart/Related Content-type

Profile: selected `multipart/related` root selection by the `start` parameter;
the selected HTML root is reflected in extracted markup. This does not claim
full handling of every related parameter or rendering behavior.

Evidence: `rfc_conformance.rs::rfc2387_related_root_and_rfc2392_inline_content_ids_remain_in_raw_mime`.

Source: [RFC 2387](https://www.rfc-editor.org/rfc/rfc2387).

Evidence: `receiver_rules.rs::related_start_selects_root_and_keeps_text_resources_as_attachments`.

## RFC 2392 — Content-ID and Message-ID Uniform Resource Locators

Profile: selected `cid:` and message-scoped `mid:` lookup, percent-decoded
identifiers, missing-reference errors, and duplicate IDs in alternatives
preferring the last representation. References resolve only inside the
provided message; there is no external fetch.

Evidence: `content_references.rs::cid_urls_resolve_transfer_decoded_inline_octets`, `content_references.rs::mid_urls_resolve_only_within_the_matching_message`, `content_references.rs::content_references_reject_invalid_escapes_and_missing_parts`, `content_references.rs::duplicate_content_ids_follow_alternative_preference_order`.

Source: [RFC 2392](https://www.rfc-editor.org/rfc/rfc2392).

## RFC 5322 — Internet Message Format

Profile: selected header unfolding, repeated fields, address extraction,
Message-ID preservation in the captured representation, and original-byte
retention. SMTP envelope handling is separate. This does not claim complete
header grammar validation or all obsolete syntax behavior.

Evidence: `raw_message.rs::parse_normalizes_addresses_preserves_headers_and_hashes_attachments`, `raw_message.rs::non_utf8_header_values_do_not_shift_later_headers`, `message_format.rs::quoted_address_headers_preserve_embedded_separators_and_groups`, `message_format.rs::repeated_address_headers_and_nested_comments_keep_mailbox_order`.

Source: [RFC 5322](https://www.rfc-editor.org/rfc/rfc5322).

## RFC 6532 — Internationalized Email Headers

Profile: selected direct UTF-8 header and body preservation. This is not a
complete internationalized email or transport implementation.

Evidence: `rfc_conformance.rs::rfc6532_preserves_utf8_headers_and_body_text`.

Source: [RFC 6532](https://www.rfc-editor.org/rfc/rfc6532).

## RFC 6533 — Internationalized Delivery Status and Disposition Notifications

Profile: N/A for notification generation or full notification parsing. The
crate recognizes and captures a `message/global` attachment as data; it does
not implement delivery-status or disposition-notification production, nor
claim complete internationalized message reassembly.

Evidence: `rfc_conformance.rs::rfc6533_message_global_is_captured_as_an_attachment`, `attachments.rs::handles_nested_rfc822_and_global_messages_in_encoded_and_raw_contexts`.

Source: [RFC 6533](https://www.rfc-editor.org/rfc/rfc6533).

Evidence: `receiver_rules.rs::arbitrary_media_types_preserve_opaque_payloads`.

## RFC 6838 — Media Type Specifications and Registration Procedures

Profile: N/A. Media-type specification and registry administration are outside
the receiver's scope. Unknown media can remain opaque; that behavior is not a
claim about registration status or registry conformance.

Evidence: `rfc_conformance.rs::rfc2046_unknown_multipart_subtype_still_parses_its_parts`.

Source: [RFC 6838](https://www.rfc-editor.org/rfc/rfc6838).

Evidence: `receiver_rules.rs::arbitrary_media_types_preserve_opaque_payloads`.
