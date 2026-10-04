# sandpost-auth

This crate evaluates authorization decisions from trusted, already loaded domain
data. Authorization answers what a user may see or manage; authentication,
which establishes who the user is, is outside this crate.

`can_view` allows access when at least one membership belongs to the requested
user and its scope is among the caller-supplied current matches. Every
membership role contributes to visibility. An optional compiled personal query
then further restricts the message facts. Missing memberships or no matching
membership deny access.

`can_manage` allows owners and administrators to manage their own scope and its
descendants, based on the valid `ScopeTree` ancestry of the target. Members and
viewers receive no administrative grant. An unknown target is denied; grants
do not pass to siblings or ancestors.

Callers must supply the trusted user identity, current memberships, and current
scope matches. This crate does not fetch SQLite data, authenticate users, or
compute policy matches. The running application currently exposes an all-mail HTTP API. Integrating
these checks into authenticated API and event-stream access is a separate milestone.

The crate depends on `sandpost-core` domain types and `sandpost-query` compiled
personal filters. Its two cohesive decision helpers remain together in `lib.rs`;
separate modules would add structure without clarifying this small API.

```rust
use sandpost_auth::{can_manage, can_view};
use sandpost_core::{
    Membership, MessageFacts, Role, Scope, ScopeIdentifier, ScopeTree, UserIdentifier,
};
use std::collections::HashSet;

let user_identifier = UserIdentifier::new();
let parent_identifier = ScopeIdentifier::new();
let child_identifier = ScopeIdentifier::new();
let memberships = [Membership {
    user_identifier,
    scope_identifier: parent_identifier,
    role: Role::Administrator,
}];
let scope = |identifier, parent| Scope {
    identifier,
    parent,
    name: "example".into(),
    description: None,
    filter: String::new(),
    position: 0,
    policy_version: 1,
};
let tree = ScopeTree::new(
    [scope(parent_identifier, None), scope(child_identifier, Some(parent_identifier))],
    None,
)?;
let current_matches = HashSet::from([parent_identifier]);
let personal_filter = sandpost_query::compile("subject contains \"urgent\"")?;
let facts = MessageFacts {
    subject: "urgent request".into(),
    ..MessageFacts::default()
};

assert!(can_view(
    user_identifier, &memberships, &current_matches, Some(&personal_filter), &facts,
));
assert!(can_manage(user_identifier, child_identifier, &memberships, &tree));
assert!(!can_view(user_identifier, &[], &current_matches, None, &facts));
# Ok::<(), Box<dyn std::error::Error>>(())
```

## Verification

From the workspace root:

```sh
cargo test --locked -p sandpost-auth
cargo clippy --locked -p sandpost-auth --all-targets -- -D warnings
```

The inline regression test covers membership unions, personal restrictions,
viewer permissions, inherited administrative grants, and unknown targets.
The example above also runs as a Rust documentation test.
