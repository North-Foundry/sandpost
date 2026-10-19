# IMAP lifecycle audit and implementation matrix

The October 2026 audit found durable UID allocation, owner authorization composition,
account-wide flags, transactional COPY/MOVE, retained subscriptions, APPEND rejection,
and Web Trash already implemented. Legacy `mirror_views` selected smart Views; it did
not select shared organization. EXPUNGE was owner-global, deleting a manual folder
restored Inbox, and smart results disappeared after filing. This audit precedes the
organization redesign; the matrix below records the implemented contract.

| Operation | Mirror | Independent | Audit gap |
| --- | --- | --- | --- |
| Receive | Primary state | Account Inbox, also while dormant | No durable retention |
| Create/rename normal folder | Primary folder | Account folder | All protocol folders private |
| Delete normal folder | Trash sole occurrences, preserve copies | Same, local | Restored Inbox |
| COPY/MOVE | Primary memberships | Account memberships | Always account local |
| Seen and supported flags | Primary shared flags | Account shared flags | Web had no durable flags |
| Trash | Primary Trash | Account Trash | Web inherited by all accounts |
| EXPUNGE normal | Remove occurrence; final occurrence enters Trash | Same, local | Owner-global deletion |
| EXPUNGE Trash | Release primary view if no normal copy survives | Release account view only | Owner-global deletion |
| Empty Trash | Permanently release primary Trash messages | Expunge local Trash occurrences; preserve surviving normal copies | No explicit view-local contract |
| Smart definitions/results | Primary state | Account definitions/state | Owner-shared definitions |
| Smart membership mutations | Refuse; Seen permitted | Refuse; Seen permitted | Smart-source writes permitted |
| Switch mode | Preserve dormant account state | Restore dormant state | No full view mode |
| Create account | Current primary history | Snapshot primary history/organization | Physical-pool fallback |
| Delete account | Remove account state | Remove account state | Existing cascading cleanup |
| Permission revoked | Current owner authorization | Current owner authorization | Retain revalidation |
| Garbage collection | Only after every view releases content | Dormant references retain content | No collector |

The mutation rows apply only to Owners and Members. Viewers may read existing mailboxes and search accessible mail, but cannot change flags, folders, subscriptions or credentials. Viewer SELECT is read-only and FETCH does not set Seen, including after role demotion in an existing selected session. Scope and membership resolution is unchanged.

Physical message content stays shared. Primary placement/Trash/deletions/flags/copies
are distinct from Independent account placement/flags/copies/retention. Arrival seeds
dormant Independent retention too. Permission composition always bounds retained data.

Mailbox identity includes credential, mode, and source. Mode changes invalidate old
sessions using a monotonic generation, including a switch away and back between
commands. Dormant identities and UID counters remain durable. Reconnection selects
state with the appropriate UIDVALIDITY; no UIDs identify messages across modes.

`\Deleted` stays a per-occurrence marker. EXPUNGE removes flagged occurrences, promotes
surviving copies, moves a final normal occurrence into Trash, and permanently releases
a final Trash occurrence. This is a documented application retention policy. COPY
then source EXPUNGE remains compatible with clients that do not use MOVE. Smart
SELECT permits only Seen in PERMANENTFLAGS; EXAMINE remains read-only. Smart source
MOVE and smart destination COPY/MOVE are refused. APPEND remains unsupported.

Concurrent writers serialize in immediate SQLite transactions. Failed mutations roll
back all references and UIDs. Authorization and credential validation run for each
command and IDLE refresh; previously issued UIDs cannot bypass current visibility.

Upgrade preserves original account-local folder IDs, UIDs, flags, and placements as
Independent state, separating new Mirror projections from canonical primary state.
Legacy duplicate folder names require deterministic display disambiguation.

Validation covers normal copies and shared Seen, surviving memberships, Trash removal,
primary deletion while Independent retains mail, final-reference collection, arrivals
while Mirror, live mode changes, scope revocation, restart and UID consistency.
Transaction rollback, competing Mirror COPY, stale generation mutations, writer-lock
behavior and stale catalog omission have focused regression coverage. The detailed
requirement and test evidence is in [imap-implementation-report.md](imap-implementation-report.md).
