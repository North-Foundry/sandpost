# IMAP RFC coverage

This document maps the server's implemented behavior to focused automated tests.
It is an evidence map, not a declaration of complete conformance. IMAP has
requirements beyond this server's role: Sandpost serves virtual stores of captured mail. Private mailbox creation, renaming,
and deletion are supported; APPEND and changes to Web-managed structures are refused. Section links
point to the RFC Editor text; each RFC heading also links to its errata search.

Every integration test (in `tests/`) runs over real TCP, and its client validates
every greeting and response with [`imap-codec`](https://crates.io/crates/imap-codec),
an independent implementation of the IMAP grammar. Unit tests live next to the
code (`src/**`). Behavior that depends on Sandpost's application (accounts,
authorization, mailbox semantics) is tested in `sandpost-application`,
`sandpost-storage-conformance`, and the runtime crate; see the last sections.

## Advertised capabilities

`IMAP4rev1`, `STARTTLS` (when offered and not yet encrypted), `LOGINDISABLED`
(while TLS is required and not active) or `AUTH=PLAIN` with `SASL-IR`,
`LITERAL-`, `ID`, `IDLE`, `NAMESPACE`, `UIDPLUS`, `MOVE`, `UNSELECT`, `CHILDREN`.
After login, authentication capabilities are omitted. The exact lists are
asserted by
[capability_lists_only_implemented_extensions](tests/session.rs) and
[required_start_tls_disables_login_until_upgraded](tests/tls.rs).

Not advertised and not implemented: `IMAP4rev2` (RFC 9051), `ENABLE` (RFC 5161),
`CONDSTORE`/`QRESYNC` (RFC 7162), `LIST-EXTENDED` (RFC 5258) and the
`SPECIAL-USE` capability (RFC 6154 section 2 only requires it for extended LIST),
`LIST-STATUS`, `ESEARCH`, `SEARCHRES`, `SORT`/`THREAD`, `BINARY` (RFC 3516),
`UTF8=ACCEPT` (RFC 6855), `MULTIAPPEND`, `CATENATE`, `APPENDLIMIT`, `QUOTA`,
`ACL`, `METADATA`, `COMPRESS=DEFLATE`, `NOTIFY`, `CREATE-SPECIAL-USE`,
`LOGIN-REFERRALS`, `LITERAL+` (LITERAL- is advertised instead; RFC 7888 forbids
both), and SASL mechanisms other than PLAIN (stored passwords are one-way
hashes, so challenge-response mechanisms cannot be verified).

## RFC 3501 — IMAP4rev1

Source: [RFC 3501](https://www.rfc-editor.org/rfc/rfc3501),
[RFC Editor errata](https://errata.rfc-editor.org/search/?rfc_number=3501).

| Section | Requirement / policy | Evidence |
| --- | --- | --- |
| [2.2.1, 5.5](https://www.rfc-editor.org/rfc/rfc3501#section-5.5) | Commands are framed from partial TCP reads; pipelined commands complete in order. | [partial_reads_and_pipelining](tests/session.rs), [partial_reads_join_into_lines](src/session/connection.rs), [cancelled_reads_keep_partial_input](src/session/connection.rs) |
| [2.2.2, 7.1](https://www.rfc-editor.org/rfc/rfc3501#section-7.1) | Status responses carry tags and bracketed response codes; untagged BAD when the tag is unreadable. | [status_lines_are_well_formed](src/protocol/response.rs), [errors_carry_the_tag_when_known](src/protocol/parser/tests.rs), [state_rules_and_malformed_commands](tests/session.rs) |
| [2.3.1.1](https://www.rfc-editor.org/rfc/rfc3501#section-2.3.1.1) | UIDs ascend and stay stable; UIDVALIDITY and UIDNEXT are reported. | [select_and_examine](tests/mailboxes.rs), [uid_fetch_and_sequence_bounds](tests/messages.rs), application tests below |
| [2.3.1.2, 7.4.1](https://www.rfc-editor.org/rfc/rfc3501#section-7.4.1) | Sequence numbers renumber on EXPUNGE; foreign removals are deferred during sequence-based FETCH/STORE/COPY/MOVE and all SEARCH commands; removals are announced highest-first. | [foreign_expunges_respect_sequence_number_rules](tests/messages.rs), [copy_and_move_preserve_targets_across_foreign_expunges](tests/messages.rs), [uid_search_sequence_keys_defer_foreign_expunges](tests/messages.rs), [expunges_are_deferred_then_announced](src/session/selected.rs) |
| [2.3.2](https://www.rfc-editor.org/rfc/rfc3501#section-2.3.2) | System flags, keywords, and session-only `\Recent`; `\Recent` cannot be stored. | [system_flags_are_recognized_without_recent](src/protocol/flags.rs), [store_operations_and_flags](src/protocol/parser/tests.rs), [store_flags_and_keywords](tests/messages.rs) |
| [3, 6](https://www.rfc-editor.org/rfc/rfc3501#section-3) | Not Authenticated, Authenticated, Selected, and Logout states; commands in the wrong state receive BAD. | [state_rules_and_malformed_commands](tests/session.rs), [select_and_examine](tests/mailboxes.rs) |
| [4.3, 9](https://www.rfc-editor.org/rfc/rfc3501#section-4.3) | Atoms, quoted strings with escapes, synchronizing literals with continuation, astring and list-mailbox forms; output strings choose quoted or literal form so data never breaks framing. | [login_arguments_use_astring_forms](src/protocol/parser/tests.rs), [login_failures_are_bounded](tests/session.rs), [strings_choose_quoted_or_literal_form](src/protocol/response.rs), [atom_characters_exclude_specials](src/protocol/syntax.rs) |
| [5.1, 5.1.3](https://www.rfc-editor.org/rfc/rfc3501#section-5.1.3) | Case-insensitive INBOX; modified UTF-7 mailbox names in both directions; malformed names refused. | [modified_utf7_round_trips_rfc_example](src/protocol/mailbox_name.rs), [malformed_modified_utf7_is_rejected](src/protocol/mailbox_name.rs), [mailbox_names_are_decoded](src/protocol/parser/tests.rs), [status_reports_counts](tests/mailboxes.rs) |
| [5.4](https://www.rfc-editor.org/rfc/rfc3501#section-5.4) | Autologout after at least 30 minutes once authenticated; a shorter pre-authentication timer (allowed by RFC 9051 section 5.4). | [unauthenticated_sessions_time_out](tests/session.rs), `Limits::authenticated_timeout` default |
| [6.1.1](https://www.rfc-editor.org/rfc/rfc3501#section-6.1.1) | CAPABILITY returns one untagged list containing IMAP4rev1; the greeting carries it as a response code. | [capability_lists_only_implemented_extensions](tests/session.rs), [greeting_announces_capabilities](tests/session.rs) |
| [6.1.2, 6.1.3](https://www.rfc-editor.org/rfc/rfc3501#section-6.1.2) | NOOP reports pending changes; LOGOUT sends BYE before the tagged OK and closes. | [foreign_changes_are_announced](tests/messages.rs), [any_state_commands_and_logout](tests/session.rs) |
| [6.2.1](https://www.rfc-editor.org/rfc/rfc3501#section-6.2.1) | STARTTLS: capabilities change after TLS; STARTTLS again or on implicit TLS is BAD; data pipelined after STARTTLS is discarded (RFC 9051 section 6.2.1, option 2). | [required_start_tls_disables_login_until_upgraded](tests/tls.rs), [implicit_tls_greets_after_handshake](tests/tls.rs), [implicit_tls_connection_limit_sends_no_plaintext](tests/tls.rs) |
| [6.2.2](https://www.rfc-editor.org/rfc/rfc3501#section-6.2.2) | AUTHENTICATE with continuation, cancellation with `*`, unsupported mechanisms refused with NO. | [authenticate_plain_variants](tests/session.rs), [authenticate_rejects_malformed_base64_continuations](tests/session.rs), [plain_messages_are_validated](src/session/authentication.rs) |
| [6.2.3, 7.2.1, 11.2](https://www.rfc-editor.org/rfc/rfc3501#section-6.2.3) | LOGINDISABLED while TLS is required: LOGIN and AUTHENTICATE refused with PRIVACYREQUIRED; failures delayed and bounded per session. | [required_start_tls_disables_login_until_upgraded](tests/tls.rs), [login_failures_are_bounded](tests/session.rs), [throttled_and_unavailable_authentication](tests/session.rs), [revoked_accounts_cannot_bypass_validation_with_append](tests/session.rs) |
| [6.3.1, 6.3.2](https://www.rfc-editor.org/rfc/rfc3501#section-6.3.1) | SELECT/EXAMINE return FLAGS, PERMANENTFLAGS, EXISTS, RECENT, UNSEEN, UIDVALIDITY, UIDNEXT and READ-WRITE/READ-ONLY; a failed SELECT leaves no mailbox selected. | [select_and_examine](tests/mailboxes.rs), [selection_responses](src/session/selected.rs) |
| [6.3.3–6.3.5](https://www.rfc-editor.org/rfc/rfc3501#section-6.3.3) | CREATE, DELETE, RENAME: INBOX cannot be deleted; existing and missing names get ALREADYEXISTS/NONEXISTENT; the backend routes normal structure to primary Mirror or private Independent state; smart structures are protected. Normal hierarchy paths report parents and retain identity through subtree rename. | [create_delete_rename_are_refused_validly](tests/mailboxes.rs) |
| [6.3.6–6.3.9](https://www.rfc-editor.org/rfc/rfc3501#section-6.3.8) | SUBSCRIBE/UNSUBSCRIBE drive LSUB; subscribed names survive deleted or unexposed mailboxes as Noselect entries; LIST wildcards `*` and `%`, the delimiter query for an empty pattern, `\HasNoChildren` and special-use attributes. | [list_patterns_and_attributes](tests/mailboxes.rs), [subscriptions_drive_lsub](tests/mailboxes.rs), [list_patterns_follow_rfc_wildcards](src/protocol/mailbox_name.rs), [adversarial_list_patterns_are_bounded](src/protocol/mailbox_name.rs) |
| [6.3.10](https://www.rfc-editor.org/rfc/rfc3501#section-6.3.10) | STATUS reports MESSAGES, RECENT, UIDNEXT, UIDVALIDITY, UNSEEN. | [status_reports_counts](tests/mailboxes.rs), [status_items](src/protocol/parser/tests.rs) |
| [6.3.11](https://www.rfc-editor.org/rfc/rfc3501#section-6.3.11) | APPEND: TRYCREATE for a missing mailbox; CANNOT otherwise; a synchronizing literal is refused before upload and a non-synchronizing one is read and discarded. | [append_is_refused_without_upload](tests/mailboxes.rs), [append_and_id](src/protocol/parser/tests.rs) |
| [6.4.1, 6.4.2](https://www.rfc-editor.org/rfc/rfc3501#section-6.4.2) | CHECK; CLOSE expunges silently unless read-only and returns to the authenticated state. Sandpost retains the final normal occurrence in the active view’s Trash; final Trash removal releases only that view. | [unselect_and_close](tests/mailboxes.rs), [mirror_move_and_trash_expunge_preserve_other_views_and_normal_copies](../sandpost-application/src/imap/tests.rs) |
| [6.4.3](https://www.rfc-editor.org/rfc/rfc3501#section-6.4.3) | EXPUNGE removes `\Deleted` messages with untagged EXPUNGE responses; read-only mailboxes refuse. Sandpost removes only active-view occurrences, retaining surviving copies and using two-stage Trash policy; Independent does not delete primary state. | [expunge_and_uid_expunge](tests/messages.rs), [mirror_move_and_trash_expunge_preserve_other_views_and_normal_copies](../sandpost-application/src/imap/tests.rs) |
| [6.4.4](https://www.rfc-editor.org/rfc/rfc3501#section-6.4.4) | Every SEARCH key, nesting, OR/NOT, sequence sets, CHARSET US-ASCII and UTF-8 (others: BADCHARSET); case-insensitive substring matching with decoded headers and bodies. | [search_criteria](tests/messages.rs), [search_uid_star_ignores_pending_highest_uid_removal](tests/messages.rs), [sent_search_keys_do_not_fall_back_to_internal_date](tests/messages.rs), [search_keys](src/protocol/parser/tests.rs), [search_evaluation](src/session/search.rs), [header_criteria_decode_and_fold_case](src/message/searchable.rs), [text_search_preserves_header_separators](src/message/searchable.rs), [nested_messages_are_searched](src/message/searchable.rs), [body_search_normalizes_mime_and_transfer_encoding](src/message/searchable.rs) |
| [6.4.5, 7.4.2](https://www.rfc-editor.org/rfc/rfc3501#section-6.4.5) | FETCH items and macros, BODY[section] with part numbers, HEADER, HEADER.FIELDS(.NOT), TEXT, MIME, partial fetches reported by origin, BODY.PEEK, implicit `\Seen` reported in FLAGS, NIL for missing sections. | [fetch_items_and_implicit_seen](tests/messages.rs), [fetch_items_and_sections](src/protocol/parser/tests.rs), [sections_follow_rfc_numbering](src/message/structure.rs), [header_field_subsets_keep_blank_line](src/message/structure.rs), [sections_and_partials](src/session/fetch.rs) |
| [7.4.2](https://www.rfc-editor.org/rfc/rfc3501#section-7.4.2) | ENVELOPE from raw header fields (groups, routes, Sender/Reply-To default to From); BODY and BODYSTRUCTURE with parameters, encodings, sizes, line counts, encapsulated messages, and extension data. | [envelope_defaults_sender_and_reply_to](src/message/envelope.rs), [groups_are_bracketed](src/message/envelope.rs), [multipart_with_attachment](src/message/body_structure.rs), [encapsulated_message](src/message/body_structure.rs), [structured_mime_tokens_are_normalized](src/message/body_structure.rs), [depth_overflow_renders_valid_opaque_structure](src/message/body_structure.rs), [multipart_overflow_renders_valid_opaque_structure](src/message/body_structure.rs), [empty_quoted_local_parts_are_preserved](src/message/envelope.rs) |
| [6.4.6](https://www.rfc-editor.org/rfc/rfc3501#section-6.4.6) | STORE replace/add/remove, `.SILENT`, unsolicited FETCH for flags others changed, FLAGS re-announced for new keywords. | [store_flags_and_keywords](tests/messages.rs), [foreign_changes_are_announced](tests/messages.rs), [additions_and_keywords_are_announced](src/session/selected.rs) |
| [6.4.7](https://www.rfc-editor.org/rfc/rfc3501#section-6.4.7) | COPY to a missing mailbox returns TRYCREATE; refused destinations return CANNOT. | [copy_reports_copyuid](tests/messages.rs) |
| [6.4.8](https://www.rfc-editor.org/rfc/rfc3501#section-6.4.8) | UID FETCH/STORE/SEARCH/COPY include UID data, ignore unknown UIDs, and `n:*` includes the highest UID. | [uid_fetch_and_sequence_bounds](tests/messages.rs), [star_range_includes_largest_below_start](src/protocol/sequence.rs) |
| [9](https://www.rfc-editor.org/rfc/rfc3501#section-9) | Message sequence numbers past the mailbox size receive BAD; `*` on an empty mailbox matches nothing. | [uid_fetch_and_sequence_bounds](tests/messages.rs), [sequence_sets_resolve_to_indexes](src/session/selected.rs) |

## Extensions

| RFC | Requirement / policy | Evidence |
| --- | --- | --- |
| [2177 IDLE](https://www.rfc-editor.org/rfc/rfc2177) | `+` continuation, EXISTS/RECENT/EXPUNGE/FETCH pushed from change notifications without polling, DONE ends with tagged OK, other input ends with BAD, keepalives, a time limit, revocation and shutdown during IDLE. Failed or timed-out refreshes send BYE [UNAVAILABLE] and require reconnection. | [idle_pushes_changes_until_done](tests/idle.rs), [idle_requires_done](tests/idle.rs), [keepalives_and_idle_time_limit](tests/idle.rs), [revocation_during_idle_ends_the_session](tests/idle.rs), [concurrent_sessions_all_receive_updates](tests/idle.rs), [shutdown_during_idle](tests/idle.rs), [idle_validation_failure_requires_reconnect](tests/idle.rs), [idle_synchronization_failure_requires_reconnect](tests/idle.rs), [idle_validation_timeout_requires_reconnect](tests/idle.rs) |
| [4315 UIDPLUS](https://www.rfc-editor.org/rfc/rfc4315) | UID EXPUNGE restricted to a UID set; COPYUID in tagged OK with positionally matching sets. APPENDUID is never sent because APPEND is refused. Successful copies allocate fresh UIDs. Destinations already containing the message are refused with NO [CANNOT]. | [expunge_and_uid_expunge](tests/messages.rs), [copy_reports_copyuid](tests/messages.rs), [ordered_sets_collapse_ascending_runs](src/protocol/sequence.rs) |
| [6851 MOVE](https://www.rfc-editor.org/rfc/rfc6851) | MOVE and UID MOVE: untagged OK with COPYUID, then EXPUNGE responses, then the tagged OK; refused on read-only mailboxes. MOVE changes mailbox membership only and does not trigger Sandpost user deletion. COPY followed by STORE/EXPUNGE of the primary source transfers like MOVE when the copy survives; expunging a copy preserves unrelated occurrences, and a final normal occurrence enters active-view Trash. | [move_expunges_from_source](tests/messages.rs), [mirror_move_and_trash_expunge_preserve_other_views_and_normal_copies](../sandpost-application/src/imap/tests.rs) |
| [3691 UNSELECT](https://www.rfc-editor.org/rfc/rfc3691) | Deselect without expunging. | [unselect_and_close](tests/mailboxes.rs) |
| [2342 NAMESPACE](https://www.rfc-editor.org/rfc/rfc2342) | One personal namespace with the `/` delimiter. | [any_state_commands_and_logout](tests/session.rs) |
| [2971 ID](https://www.rfc-editor.org/rfc/rfc2971) | Parameters validated and ignored: at most 30 pairs, field names at most 30 octets, values at most 1,024 octets, and no repeated field names; the server identifies as Sandpost. | [any_state_commands_and_logout](tests/session.rs), [append_and_id](src/protocol/parser/tests.rs) |
| [3348 CHILDREN](https://www.rfc-editor.org/rfc/rfc3348) | The flat namespace lists every mailbox with `\HasNoChildren`. | [list_patterns_and_attributes](tests/mailboxes.rs) |
| [6154](https://www.rfc-editor.org/rfc/rfc6154#section-2) | `\Trash` in non-extended LIST, which needs no capability. | [list_patterns_and_attributes](tests/mailboxes.rs) |
| [7888 LITERAL-](https://www.rfc-editor.org/rfc/rfc7888#section-5) | Non-synchronizing literals up to 4096 octets accepted without continuation; larger ones read, discarded, and refused with BAD [TOOBIG]; oversized synchronizing literals refused before upload. | [non_synchronizing_literals_follow_literal_minus](tests/session.rs), [oversized_input_is_refused](tests/session.rs), [command_size_includes_lines_after_literals](tests/session.rs), [literal_announcements](src/session/command_input.rs) |
| [4959 SASL-IR](https://www.rfc-editor.org/rfc/rfc4959), [4616 PLAIN](https://www.rfc-editor.org/rfc/rfc4616) | Initial responses, `=` for empty, foreign authorization identities refused. | [authenticate_plain_variants](tests/session.rs), [authenticate_initial_response](src/protocol/parser/tests.rs) |
| [5530](https://www.rfc-editor.org/rfc/rfc5530) | AUTHENTICATIONFAILED, UNAVAILABLE, PRIVACYREQUIRED, CANNOT, NONEXISTENT, ALREADYEXISTS, EXPUNGEISSUED response codes. | [login_failures_are_bounded](tests/session.rs), [create_delete_rename_are_refused_validly](tests/mailboxes.rs), [foreign_expunges_respect_sequence_number_rules](tests/messages.rs), [copy_and_move_preserve_targets_across_foreign_expunges](tests/messages.rs), [uid_search_sequence_keys_defer_foreign_expunges](tests/messages.rs) |
| [8314](https://www.rfc-editor.org/rfc/rfc8314#section-3.2) | Implicit TLS performs the handshake before the greeting. | [implicit_tls_greets_after_handshake](tests/tls.rs), [implicit_tls_connection_limit_sends_no_plaintext](tests/tls.rs) |

## Sandpost semantics (outside this crate)

| Behavior | Evidence |
| --- | --- |
| Visible mail = owner authorization AND mailbox definition, for INBOX, Views, local folders, Trash, SEARCH, FETCH, UID FETCH, and STATUS; revoked scopes remove mail at the next synchronization. | `sandpost-application` [mailboxes_never_exceed_owner_permissions](../sandpost-application/src/imap/tests.rs), runtime [revoking_a_scope_expunges_from_idle_clients](../sandpost/src/imap_runtime_tests.rs) |
| Mirror mode, selected Views with aliases, smart/private/manual folders, authorized INBOX fallback, renames keep UIDVALIDITY, filter changes add/remove members without renumbering, deleted sources disappear. | [mirror_mode_follows_views](../sandpost-application/src/imap/tests.rs), [independent_mode_ignores_views_and_keeps_private_folders](../sandpost-application/src/imap/tests.rs), [definitions_compose_authorization](../sandpost-application/src/imap/catalog.rs) |
| Ownership, disabled accounts, credential rotation, deletion, owner deletion, throttling. | [accounts_are_owned](../sandpost-application/src/imap/tests.rs), [credentials_and_revocation](../sandpost-application/src/imap/tests.rs), [deleting_the_owner_revokes_sessions](../sandpost-application/src/imap/tests.rs), [failed_logins_are_throttled](../sandpost-application/src/imap/tests.rs) |
| Persistent UIDs, UIDVALIDITY allocation, membership reconciliation, shared flags, per-mailbox `\Deleted`, owner-level deletion tombstones, mailbox exclusions, moves, backend-independent contract. | `sandpost-storage-conformance` [imap_mailbox_identities, imap_membership_synchronization, imap_flags_and_expunge](../sandpost-storage-conformance/src/imap.rs) |
| SMTP mail pushed over IDLE through the real runtime; UIDVALIDITY and UIDs survive restart. | [smtp_mail_is_pushed_to_idle_clients_and_uids_persist](../sandpost/src/imap_runtime_tests.rs) |

## Interoperability

Tested with independent implementations:

- [`imap-codec`](https://crates.io/crates/imap-codec) parses every response in the
  integration tests.
- Python's standard-library `imaplib` drives the real `sandpost` binary in
  `scripts/smoke.py`: capabilities, failed and successful LOGIN, LIST, SELECT,
  UID FETCH, byte-exact BODY[], SEARCH, STORE, IDLE with a message pushed from
  SMTP, UID MOVE, STATUS, and UIDVALIDITY/UID/flag persistence across restart.

Apple Mail, Thunderbird, and Outlook have **not** been exercised by automated GUI
tests. The command set they use for synchronization (UID FETCH with FLAGS,
RFC822.SIZE, INTERNALDATE, BODY.PEEK[HEADER.FIELDS], BODYSTRUCTURE, IDLE, MOVE,
UIDPLUS, STATUS polling) is covered above, but no claim of verified compatibility
with those clients is made.

## Known limitations

- APPEND is refused because Sandpost captures incoming mail. CREATE, DELETE, and RENAME
  operate on active-view normal folders; smart structures are protected.
- One occurrence of a captured message is stored per mailbox. COPY/MOVE to a mailbox
  already containing it (including the selected mailbox) returns NO [CANNOT].
- EXPUNGE/CLOSE/UID EXPUNGE remove active-view occurrences with two-stage Trash retention. MOVE only
  changes the source account organization. Clients using COPY then EXPUNGE of the
  source (Apple Mail) transfer without user deletion, because a surviving COPY occurrence
  becomes the account placement.
- Ordinary IMAP4rev1 literals exclude NUL octets. FETCH content requests for such messages
  return NO [CANNOT]; FLAGS, UID, and RFC822.SIZE remain available and captured bytes are unchanged.
  [Regression](tests/messages.rs): `fetch_refuses_nul_content_without_changing_captured_bytes`.
- MIME structure parsing is bounded to depth 32 and 2,048 entities. Composite content beyond
  those limits, and transfer-encoded embedded messages, are exposed as an opaque application/octet-stream leaf; raw FETCH remains available,
  while nested sections and SEARCH content inside the opaque fallback are unavailable.
- SEARCH evaluates content criteria by reading the selected mailbox's messages;
  it does not use the derived Tantivy index.
- No CONDSTORE/QRESYNC: clients resynchronize flags with UID FETCH.
- Flat namespace: View and folder names containing `/` are shown with `-`.
- `\Recent` is tracked per mailbox with a watermark; a read-only EXAMINE reports
  messages newer than the watermark as recent without claiming them.

## Isolated mailbox architecture coverage

Real runtime command tests in [imap_mailbox_runtime_tests.rs](../sandpost/src/imap_mailbox_runtime_tests.rs)
cover private CREATE/RENAME/DELETE, UIDVALIDITY persistence and recreation, UID COPY/MOVE,
Web exposure and unexposed fallback, protected smart/manual structure, matching smart
insertion refusal, and private-folder fallback. [imap_runtime_tests.rs](../sandpost/src/imap_runtime_tests.rs)
checks owner-wide deletion across HTTP and multiple IMAP accounts, independent users,
IDLE EXPUNGE, scope revocation, and restart.

Application [mailbox tests](../sandpost-application/src/imap/tests/mailbox.rs) cover
account isolation, Web inheritance and override priority, tombstone non-resurrection,
folder deletion, authorization loss/regrant, durable general COPY/Trash targets,
and moving a copied occurrence without replacing unrelated placements.
