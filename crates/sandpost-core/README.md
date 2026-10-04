# `sandpost-core`

Infrastructure-independent domain types for Sand Post. The crate has no
workspace dependencies and keeps its public exports at the crate root while
organizing implementation by domain responsibility.

## Domain boundaries

Core models facts and relationships; it does not parse mail, compile filters,
implement authorization, or depend on HTTP or SQL. Mail normalization belongs
to ingest (`sandpost-mail`). Scope and inbox filters, along with a user's
personal filter, are plain source strings here; parsing, validation and
compilation happen in other crates. User records and scope memberships describe
the domain and do not implement authentication or permission evaluation.

Mailboxes and envelope facts are separate from message identity and payload.
`MessageFacts` carries normalized envelope sender/recipients independently of
the MIME From, To and Cc fields, plus subject, bodies, timestamp, size, header
values and an attachment count. Duplicate header values are retained under
lowercase header names. A `Message` combines those facts with raw MIME bytes and
attachment metadata (`filename`, content type, size and content hash). The raw
message preserves the original MIME content; attachment bytes are not fields of
`Attachment`.

`User` has a name and optional personal filter. `Membership` associates a user
with a scope and a `Role` (`Owner`, `Administrator`, `Member` or `Viewer`). An
`Inbox` belongs to a user and has a name and plain string filter. These records
express domain data only; visibility and inherited administrative grants live
in `sandpost-auth`.

## Identifiers and serialized names

`MessageIdentifier`, `ScopeIdentifier`, `UserIdentifier` and `InboxIdentifier`
are distinct transparent UUID wrappers. Each can generate a random UUID with
`new` or `Default`, display in canonical UUID form, and parse from a UUID
string. `MessageSequence` is a separate transparent `u64` used as a numeric
message sequence, for example as an internal persistence or pagination key; it
is not an opaque UUID. The storage boundary is responsible for database integer
conversion.

Serde field names are part of the external representation. Depending on the
record, they include `id`, `cc`, `html`, `message_id`, `user_id` and
`scope_id`; the Rust fields remain fully named. These names describe serialized
data, not a promise about SQLite storage layout.

## Scope hierarchy

`ScopeTree` indexes `Scope` records by identifier and stores ordered adjacency
lists. `ScopeTree::new` rejects duplicate identifiers, missing parents,
cycles (including disconnected cyclic components), and nodes deeper than the
optional maximum depth. Roots are at depth zero, so `Some(0)` permits roots but
no children; `None` imposes no configured depth limit. Validation and traversal
use explicit stacks for validation and subtree traversal rather than recursion.

Siblings, including roots, are ordered by `position` and then identifier for a
stable tie-break. `roots` and `children` return those ordered identifiers.
`ancestors` returns parents nearest-first and excludes the requested scope;
unknown identifiers return `TreeError::Unknown`. `subtree` returns preorder,
including the selected root, and preserves the sibling ordering. `get` returns
an optional scope reference. `scopes` iterates all records in unspecified
order, because its backing index is a hash map.

`move_scope` validates the full proposed hierarchy before replacing the tree.
`set_filter` changes the selected scope's source filter. Both return the
affected subtree in preorder and advance policy versions only for that subtree,
so callers can recompile policies and refresh materializations at the relevant
boundary. Failures are atomic: invalid topology, unknown identifiers, or a
policy-version overflow leave the existing tree unchanged. Filter syntax is
outside core, so callers validate/compile it before applying the edit.

```rust
use sandpost_core::{Scope, ScopeIdentifier, ScopeTree};

let root_identifier = ScopeIdentifier::new();
let child_identifier = ScopeIdentifier::new();
let root_scope = Scope {
    identifier: root_identifier,
    parent: None,
    name: "team".into(),
    description: None,
    filter: String::new(),
    position: 0,
    policy_version: 1,
};
let child_scope = Scope {
    identifier: child_identifier,
    parent: Some(root_identifier),
    name: "alerts".into(),
    description: None,
    filter: String::new(),
    position: 0,
    policy_version: 1,
};
let mut tree = ScopeTree::new([root_scope, child_scope], Some(4))?;

// Validate the filter in the query layer before recording its source here.
let affected = tree.set_filter(root_identifier, "from.address == \"ops@example.test\"".into())?;
assert_eq!(affected, vec![root_identifier, child_identifier]);
assert_eq!(tree.get(child_identifier).unwrap().policy_version, 2);
# Ok::<(), sandpost_core::TreeError>(())
```

## Module map

- `identifiers.rs`: four opaque UUID identifiers and `MessageSequence`.
- `messages.rs`: `Mailbox`, `MessageFacts`, `Attachment` and `Message`.
- `users.rs`: `User`, `Role`, `Membership` and `Inbox`.
- `scopes.rs`: `Scope`, `TreeError` and indexed `ScopeTree`.
- `lib.rs`: re-exports the public API from the crate root.

The scope hierarchy tests are integration tests in `tests/scope_tree.rs`.

## Verification

```sh
cargo test --locked -p sandpost-core
cargo clippy --locked -p sandpost-core --all-targets -- -D warnings
```
