# IMAP organization and lifecycle implementation report

This report records the implementation and final verification against the supplied
“IMAP Mirror, Independent, Smart Folders & Message Lifecycle” specification.
The behavioral contract is [imap.md](imap.md); the original audit and operation
matrix are [imap-operation-matrix.md](imap-operation-matrix.md).

## Existing behavior retained

The existing implementation already provided shared physical MIME storage, current
owner authorization composition, durable mailbox UIDs and UIDVALIDITY, account-wide
flags, transactional COPY/MOVE, subscriptions, Web Trash, APPEND rejection, and a
durable search outbox. These components were extended rather than replaced.

The principal discrepancies were that `mirror_views` selected smart Views rather
than shared primary organization; EXPUNGE could affect the primary user globally;
Independent state inherited Web changes; deleting a manual folder restored Inbox;
and filing mail could incorrectly remove it from smart results. Physical lifetime
was not governed by durable Independent references.

## Requirement review

| Specification | Implemented behavior and evidence |
| --- | --- |
| 1. Ownership | One physical mail pool. Primary user and Independent organization are separate. Current canonical owner authorization bounds every account; materialized UIDs and retention never authorize access. Storage organization helpers and application handle-access tests cover this boundary. |
| 2. Modes | Explicit Mirror/Independent types flow through storage, application, HTTP, native bridge and Vue. Mirror uses canonical primary folder definitions, copies, flags and deletion state; account catalogs are protocol projections. Independent owns separate organization and retained references. Mixed-mode wire and storage tests verify isolation/sharing. |
| 3. Switching | Both states survive toggles. New arrivals seed dormant Independent references. Each mode has its own persistent mailbox identities and counters. Generation fencing revokes connected sessions and rejects stale writes, including an away-and-back transition. Wire tests verify BYE, reconnection, restored flags/UIDs and restart. |
| 4. Initial history | Account creation atomically snapshots currently authorized primary history, normal folders, copies, Trash and flags. Primary tombstones exclude mail retained only elsewhere. Smart definitions are not imported. Snapshot tests check historical organization and later independence. |
| 5. Deletion | Web Delete moves the underlying primary message to Trash; second Delete/Empty Trash releases that view. IMAP Deleted is occurrence-local. EXPUNGE removes marked occurrences, promotes surviving normal copies, sends a final normal occurrence to Trash, and releases a final Trash occurrence. Independent never tombstones primary or peers. COPY then source EXPUNGE preserves its destination. |
| 6. Retention | Durable Independent references survive dormant mode and permission loss. Primary untombstoned references conservatively retain content. Final release/account cleanup collects raw mail and cascading facts/attachment metadata in the same transaction as durable search-deletion enqueueing. A service test checks removal of the actual global Tantivy document after catch-up. |
| 7. Copies/flags | COPY adds memberships without duplicating physical content. Seen, Answered, Flagged, Draft and bounded keywords are shared within a view. Deleted stays occurrence-local. Primary Web and Mirror share flags; Independent flags remain local. Duplicate destinations fail atomically with CANNOT except cross-mailbox Trash transfers, which retain existing occurrences/UIDs and omit COPYUID for coalesced batches. |
| 8. Smart folders | Dynamic query results ignore normal placement and legacy manual exclusions. Active-view Trash is excluded. Mirror uses primary smart definitions; Independent settings provide account-local CRUD and canonical query editing. Smart SELECT allows Seen only; EXAMINE is read-only. Membership mutations and other flag writes are refused, COPY out is allowed. Web deletion acts on the underlying primary message. |
| 9. Folder deletion | Normal-folder deletion removes that definition and sends otherwise-unfiled messages to the relevant Trash. Surviving normal copies are preserved/promoted. Hierarchical rename preserves descendant IDs and UIDs; implicit parents are nonselectable LIST entries. |
| 10. Imports | APPEND remains rejected before accepting uploaded content. Existing-mail COPY/MOVE remain supported. |
| 11. Permissions | Commands and IDLE refresh reload account/owner policy and reconcile canonical facts. FETCH, SEARCH, SELECT, content/metadata, COPY/MOVE, smart queries and stale handles cannot use retained membership to bypass revoked access. In-flight work that already read authorized facts retains the existing concurrency boundary; subsequent operations revalidate. |
| 12. Account deletion | Deleting a credential cascades its private folders, smart definitions, flags, copies, references, mailbox identities and subscriptions; primary/peer organization survives. Sessions revoke. Final-only content can be collected. The UI requires explicit destructive confirmation naming the lost Independent state. |
| 13. UI | Existing Vue/shadcn-vue patterns are retained. Create/edit expose mode and concise ownership descriptions, prevent empty mode selection, explain reconnection, show Independent smart settings conditionally, and remove obsolete linked/exposure configuration from the workflow. Primary COPY and supported flag controls use both browser and native clients. |
| 14. Matrix/races | The operation matrix covers both modes, including Empty Trash. Immediate SQLite transactions serialize references, UID allocation, generation changes, folder mutations and collection. Tests cover competing COPY, duplicate-batch rollback, stale catalogs/handles, live revocation, mode changes, restart and sequence-number handling. |
| 15. Tests | Coverage mapping and executed commands are recorded below. Both memory and file-backed storage conformance pass. |
| 16–17. Architecture/execution | Existing crate boundaries and storage traits are retained. The consolidated pre-release migrations create the current schema directly. Audit, design, implementation, validation and final review are recorded in the matrix, contracts and this report. No general-purpose import or unrelated feature was added. |

## Affected components

- `sandpost-core`: explicit organization mode and account generation.
- `sandpost-storage`: mode/initial-authorization DTOs, primary COPY/flags,
  generation-aware atomic operations, lifecycle and backend contracts.
- `sandpost-storage-sqlite`: account initialization, canonical primary versus local
  organization, copies, flags, smart membership, placement/Trash, folder hierarchy,
  transactional generation fences, deletion/collection and migration registration.
- `sandpost-application`: account management, catalog/session authorization,
  mode-aware mutation routing, notifications, primary Web COPY/flags and read status.
- `sandpost-imap`: mailbox permission reporting, smart Seen restrictions, mutation
  refusals and protocol documentation. Existing UID/sequence handling is retained.
- `sandpost-web-api`, `sandpost-node`, desktop IPC/preload and frontend API clients:
  explicit modes and primary COPY/flag operations with equivalent native/HTTP behavior.
- Vue IMAP pages/dialogs, mail COPY/flags controls, summaries, mocks and tests.
- Storage conformance, migration, service, protocol, runtime, frontend, desktop and
  real native/binary smoke tests; corresponding `.agents/docs/` contracts.

The SQLite migrations now form a consolidated 46-script pre-release baseline. Account and
folder modes, mode-specific mailbox indexes, and the primary flag cursor are part of the
original table definitions. Historical development backfills and trigger replacements were
removed; the final deletion membership trigger is created once after its dependencies.
Earlier development sequences require explicit database recreation. Migration tests verify
fresh initialization, supported baseline prefixes, batch rollback, and preserved data.

## Automated coverage mapping

Names below identify concrete tests in the current tree. Most rows have additional
coverage; this table identifies representative behavioral checks.

| Required test case | Evidence |
| --- | --- |
| 1, 3, 5. Bidirectional, multiple and mixed Mirror accounts | `imap_mode_runtime_tests::mirror_wire_mutations_share_web_organization_and_preserve_independent_state` |
| 2, 4, 13. Independent organizations and read-state isolation | SQLite `mirror_sharing_and_independent_isolation`; application `accounts_are_isolated_and_new_mail_is_recent`; mixed-mode wire test above |
| 6–8. Two-stage deletion, EXPUNGE and Trash | Application `expunge_and_folder_deletion_apply_the_occurrence_lifecycle_in_both_modes`; protocol `expunge_and_uid_expunge`; HTTP `trash_http_moves_restores_and_empties_mail` |
| 9. Retention across views | SQLite `garbage_collection_waits_for_all_retention_references`, `primary_retention_survives_independent_permanent_deletion` |
| 10. Final physical collection | Application `final_view_release_removes_the_physical_search_document`, `final_independent_trash_expunge_collects_the_last_reference`; SQLite `garbage_collection_removes_related_metadata_and_queues_search_deletion` |
| 11. COPY/MOVE | Protocol `copy_reports_copyuid`, `move_expunges_from_source`; application `web_copy_round_trips_multiple_mirrors_and_normal_folders` |
| 12. Shared Seen across copies | Application `flags_are_shared_across_mailboxes`, `deleting_populated_account_preserves_primary_and_peer_organization`; SQLite `shared_seen_is_visible_to_mirror_accounts_and_web`; `concurrent_mirror_copy_is_atomic_and_seen_is_shared` |
| 14. Smart queries and local definitions | Application `independent_mode_ignores_legacy_views_and_keeps_smart_definitions_account_local`; SQLite `smart_results_follow_each_organization_and_do_not_grant_retention` |
| 15–16. Smart restrictions/Seen | Runtime `smart_mailbox_protocol_rights_are_seen_only`; `mirror_manual_structure_and_smart_insertion_rights` |
| 17. Smart Trash exclusions | SQLite `smart_results_follow_each_organization_and_do_not_grant_retention` directly checks matching mail disappears from the smart result when sole-folder deletion places it in account Trash |
| 18. Populated folder deletion | Application lifecycle test above; SQLite `independent_folder_deletion_preserves_normal_survivors`, `primary_folder_delete_without_mirror_projections_routes_to_trash` |
| 19–20. Reversible switch/dormant arrivals | Runtime `mode_switches_revoke_wire_sessions_and_restore_dormant_uids`; SQLite `mode_switches_preserve_dormant_arrivals_and_identity_across_reopen` |
| 21. Historical initialization | Application `independent_creation_snapshots_current_authorized_primary_history`; SQLite `initial_history_and_primary_organization_are_snapshotted_once` |
| 22. No primary-deletion resurrection | Application `final_release_collects_only_after_dormant_views_release_and_never_resurrects` creates a fresh credential while older dormant state retains content and checks the new credential remains empty |
| 23. Credential cleanup | Application `deleting_populated_account_preserves_primary_and_peer_organization` checks revoked sessions, missing account/credential/folders/mailboxes/members/subscriptions, and unchanged peer/Mirror flags and content; final-reference tests check collection |
| 24. Permission revocation | Runtime `revoking_a_scope_expunges_from_idle_clients`; application `revoked_scope_blocks_direct_content_metadata_and_flag_writes` |
| 25. Rejected imports | Protocol `append_is_refused_without_upload` |
| 26. Concurrent operations | SQLite `concurrent_mirror_copies_refuse_the_duplicate_shared_occurrence`, `duplicate_in_copy_batch_rolls_back_every_new_occurrence`; application `web_copy_duplicate_batches_and_concurrent_retries_are_atomic`; `concurrent_mirror_copy_is_atomic_and_seen_is_shared` |
| 27. Restart | Runtime `private_mailbox_crud_uidvalidity_and_restart`, `primary_deletion_and_independent_expunge_are_isolated_across_restart`, mode-switch test above |
| 28. UIDs/sequence/synchronization | Restart/mode tests above; protocol `foreign_expunges_respect_sequence_number_rules`, `copy_and_move_preserve_targets_across_foreign_expunges`; SQLite `stale_catalog_omission_preserves_live_folder_uids_and_pending_flags` |

The concurrent wire regression runs two Mirror clients against one destination,
requires one success and one CANNOT, checks one Web/IMAP destination occurrence,
and verifies Seen propagates from copied folders to Inbox in both modes while
Independent remains unread after Mirror updates.

Generation tests additionally exercise stale CREATE, reconciliation, synchronization,
STORE, COPY/MOVE, EXPUNGE and folder operations, including disabled credentials and
restored modes. Pool tests cover rollback/cancellation and unchanged membership under
a competing writer lock. Migration tests cover supported upgrades and atomic failure.

## Executed verification

| Command | Result |
| --- | --- |
| `cargo test --workspace --locked` | Passed: 623 tests, zero failed/ignored, including conformance, migration, runtime and protocol suites. This run preceded the final additional focused tests below. |
| `cargo test -p sandpost-application final_view_release_removes_the_physical_search_document --lib --locked` | Passed: 1 test; verifies actual Tantivy removal. |
| `cargo test -p sandpost-application deleting_populated_account_preserves_primary_and_peer_organization --lib --locked` | Passed: 1 test. |
| `cargo test -p sandpost concurrent_mirror_copy_is_atomic_and_seen_is_shared --lib --locked` | Passed: 1 real-wire concurrency/Seen test. |
| `cargo test -p sandpost-web-api --lib --locked` | Passed: 31 tests. |
| `cargo check --workspace --all-targets --locked` | Passed. |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | Passed after replacing one redundant mode-default closure. |
| `cargo build --workspace --locked` | Passed. |
| `cargo fmt --all -- --check` | Passed after formatting the final search-collection test. |
| `yarn frontend:test` | Passed: 181 tests in 27 files. |
| `yarn frontend:build` | Passed. |
| `yarn desktop:check` | Passed. |
| `yarn desktop:test` | Passed: 29 tests. |
| `yarn native:build` and `yarn native:test` | Passed with the real darwin-arm64 addon, SMTP, modes, primary COPY/flags, Trash, HTTP parity, events and restart. |
| `python3 scripts/smoke.py --frontend apps/frontend/dist` | Passed using the real server and independent Python imaplib, including IDLE, MOVE, UID/flag persistence, HTTP/search/SSE, static assets and restart. The script now validates migration filenames and explicitly selects Independent for its local MOVE scenario. |
| `git diff --check` | Passed. |

Detailed local logs use `/tmp/sandpost-*-final.log`. They are local verification
artifacts rather than committed project dependencies.

## Compatibility policies and limits

On 14 October 2026, Apple Mail connection logs showed bulk `UID COPY ... Trash`
refused with `CANNOT` because one selected Inbox message already had a Trash copy.
The client restored the selection instead of proceeding to Deleted/EXPUNGE. The
Trash-only coalescing policy fixes this without allocating another occurrence or
advertising reused UIDs as fresh COPYUID mappings. Focused real-runtime COPY and
MOVE regressions cover both modes and both primary/copy arrangements, retries,
reconnection, preserved destination UIDs and UIDNEXT. All 17 runtime IMAP tests,
10 storage mode tests, 7 atomicity tests, and the storage conformance suite passed.

The rebuilt macOS ARM64 desktop was manually verified through Apple Mail using
Select All and Delete on 73 Inbox messages, including the existing Trash copy.
Inbox became empty and stayed empty after selecting Trash and returning; Trash
contained all 73 messages among 100 total occurrences. A before/after SQLite
comparison confirmed the physical mail count remained 723 and the preexisting
Trash UID was preserved. This verifies that deletion scenario, not full GUI
interoperability certification.

- New credentials default to Mirror; the consolidated baseline does not upgrade older development databases.
  Legacy `mirror_views`, linked/exposure inputs remain accepted for compatibility,
  but the UI presents explicit mode. Independent never imports primary smart folders.
- Switching modes disconnects clients; reconnect is required. UIDs are meaningful
  only within the mode-specific mailbox identity and UIDVALIDITY.
- Product deletion extends mailbox-local IMAP deletion with Trash retention for
  the final normal occurrence. Protocol EXPUNGE removes the selected occurrence;
  other normal copies survive. Web Delete/Empty Trash operates on the underlying
  primary message. This distinction is intentional and documented.
- COPY into an already-present destination is rejected atomically with CANNOT except
  cross-mailbox Trash transfers, which retain existing occurrences/UIDs and omit COPYUID
  for coalesced batches. There is at most one occurrence of a physical message per mailbox;
  Trash coalescing is a compatibility policy rather than general duplicate IMAP copies.
- Smart SELECT advertises only Seen in PERMANENTFLAGS; EXAMINE is fully read-only.
  Some clients may not support updating Seen in such virtual mailboxes. No mailbox
  membership is sacrificed to force client behavior.
- Dormant/temporarily unauthorized Independent references and untombstoned primary
  users conservatively retain content. Collection may therefore be delayed; it
  never treats a transient permission loss as release of a view's history.
- Query definitions use the existing canonical facts-only DSL. Explicit
  Trash-inclusive smart queries remain unsupported; the specification made this
  optional. APPEND/import remains deliberately unsupported.
- Apple Mail, Thunderbird and Outlook GUI interoperability was not manually or
  automatically certified. Real command-level tests and imaplib cover protocol
  behavior, including COPY/Deleted/EXPUNGE strategies. Existing unsupported
  extensions such as CONDSTORE/QRESYNC and IMAP4rev2 remain out of scope.
- Platform evidence here is macOS ARM64. No new claim is made for installers,
  signing, other operating systems or packaged-client GUI certification.

No required functional behavior remains knowingly unimplemented. No critical
cross-account authorization or premature-collection defect remains known after
transaction-fence and lifecycle review. These statements are bounded by the
executed automated checks and the client/platform limits above.
