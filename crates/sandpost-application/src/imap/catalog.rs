//! The mailboxes an IMAP account exposes, their names, and their membership definitions.
//!
//! An account always has `INBOX` and `Trash`, followed by Sandpost Views (every View the owner
//! can see when mirroring, otherwise the linked selection) and the account's local folders.
//! Names are derived deterministically: aliases or View and folder names, with `/` (the
//! hierarchy delimiter) replaced, and case-insensitive collisions resolved with " (2)", " (3)".
//!
//! Every definition is the owner's current authorization AND the mailbox's own filter, so no
//! mailbox can show mail its owner cannot read. INBOX aggregates: all of the owner's mail when the
//! account mirrors Views, otherwise the union of the linked Views and the local folders that
//! participate in INBOX (nothing when there are none).
use crate::{ApplicationError, QueryOptions, authorization};
use sandpost_core::{
    ImapAccount, ImapAccountMode, ImapFolderKind, ImapLinkedView, ImapLocalFolder,
    ImapMailboxSource, UserIdentifier, View,
};
use sandpost_query::Expression;
use sandpost_storage::{ImapMembershipDefinition, Storage};
use std::sync::Arc;

/// Name of the aggregated mailbox; reserved case-insensitively.
pub(crate) const INBOX_NAME: &str = "INBOX";
/// Name of the account's trash mailbox.
pub(crate) const TRASH_NAME: &str = "Trash";

/// One mailbox an account exposes, before identities are resolved.
#[derive(Debug, Clone)]
pub(crate) struct PlannedMailbox {
    pub(crate) source: ImapMailboxSource,
    pub(crate) name: String,
    pub(crate) view: Option<View>,
    pub(crate) folder: Option<ImapLocalFolder>,
}

/// Turn a display name into a flat mailbox name.
fn sanitized(name: &str) -> String {
    let replaced: String = name
        .chars()
        .map(|character| {
            if character == '/' || character.is_control() {
                '-'
            } else {
                character
            }
        })
        .collect();
    let trimmed = replaced.trim();
    if trimmed.is_empty() {
        "Untitled".to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// Reserve a unique name, appending a counter on a case-insensitive collision.
fn unique_name(taken: &mut Vec<String>, wanted: &str) -> String {
    let mut candidate = wanted.to_owned();
    let mut counter = 2;
    while taken
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(&candidate))
    {
        candidate = format!("{wanted} ({counter})");
        counter += 1;
    }
    taken.push(candidate.clone());
    candidate
}

/// Plan the mailboxes of an account from the owner's visible Views, the linked selection, and
/// the account's local folders.
pub(crate) fn plan(
    account: &ImapAccount,
    visible_views: &[View],
    _linked: &[ImapLinkedView],
    folders: &[ImapLocalFolder],
) -> Vec<PlannedMailbox> {
    let mut taken = vec![INBOX_NAME.to_owned(), TRASH_NAME.to_owned()];
    let mut planned = vec![
        PlannedMailbox {
            source: ImapMailboxSource::Inbox,
            name: INBOX_NAME.to_owned(),
            view: None,
            folder: None,
        },
        PlannedMailbox {
            source: ImapMailboxSource::Trash,
            name: TRASH_NAME.to_owned(),
            view: None,
            folder: None,
        },
    ];
    let mut views: Vec<(String, &View)> = if account.mode == ImapAccountMode::Mirror {
        visible_views
            .iter()
            .map(|view| (sanitized(&view.name), view))
            .collect()
    } else {
        Vec::new()
    };
    views.sort_by(|(left_name, left), (right_name, right)| {
        left_name
            .to_lowercase()
            .cmp(&right_name.to_lowercase())
            .then(left.identifier.cmp(&right.identifier))
    });
    for (name, view) in views {
        planned.push(PlannedMailbox {
            source: ImapMailboxSource::View(view.identifier),
            name: unique_name(&mut taken, &name),
            view: Some(view.clone()),
            folder: None,
        });
    }
    let mut folders: Vec<&ImapLocalFolder> = folders
        .iter()
        .filter(|folder| match account.mode {
            ImapAccountMode::Mirror => folder.kind != ImapFolderKind::Private,
            ImapAccountMode::Independent => folder.kind != ImapFolderKind::Manual,
        })
        .collect();
    folders.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then(left.filter.cmp(&right.filter))
            .then(left.include_in_inbox.cmp(&right.include_in_inbox))
            .then(left.created_at.cmp(&right.created_at))
            .then(left.identifier.cmp(&right.identifier))
    });
    for folder in folders {
        planned.push(PlannedMailbox {
            source: ImapMailboxSource::LocalFolder(folder.identifier),
            name: unique_name(
                &mut taken,
                &if folder.kind == ImapFolderKind::Smart {
                    sanitized(&folder.name)
                } else {
                    folder.name.clone()
                },
            ),
            view: None,
            folder: Some(folder.clone()),
        });
    }
    planned
}

/// Load the owner's visible Views, the linked selection, and local folders, then plan.
pub(crate) async fn load_plan(
    storage: &Arc<dyn Storage>,
    account: &ImapAccount,
) -> Result<Vec<PlannedMailbox>, ApplicationError> {
    let visible_views = storage.list_views(account.owner_identifier).await?;
    let linked = if account.mode == ImapAccountMode::Mirror {
        Vec::new()
    } else {
        storage.list_imap_linked_views(account.identifier).await?
    };
    let folders = storage.list_imap_local_folders(account.identifier).await?;
    Ok(plan(account, &visible_views, &linked, &folders))
}

/// The owner's current authorization: mail access, scopes with inherited filters, and the
/// personal filter, exactly as every other read path composes it.
pub(crate) async fn owner_authorization(
    storage: &Arc<dyn Storage>,
    owner: UserIdentifier,
) -> Result<Expression, ApplicationError> {
    let query = authorization::compose(storage, owner, &QueryOptions::default(), None).await?;
    Ok(Expression::And(vec![
        query.authorization.unwrap_or(Expression::True),
        query.filter,
    ]))
}

/// Compile a stored filter, failing closed to an empty mailbox if it no longer compiles.
fn stored_filter(source: &str) -> Expression {
    authorization::expression(source).unwrap_or_else(|error| {
        tracing::warn!(%error, "a stored IMAP mailbox filter no longer compiles; it matches nothing");
        Expression::False
    })
}

/// Compose the membership definition of one planned mailbox.
pub(crate) fn definition(
    account: &ImapAccount,
    _planned: &[PlannedMailbox],
    mailbox: &PlannedMailbox,
    authorization: &Expression,
) -> ImapMembershipDefinition {
    let own_filter = match mailbox.source {
        // INBOX is the general fallback regardless of which smart folders are exposed.
        ImapMailboxSource::Trash | ImapMailboxSource::Inbox => Expression::True,
        ImapMailboxSource::View(_) => mailbox
            .view
            .as_ref()
            .map_or(Expression::False, |view| stored_filter(&view.filter)),
        ImapMailboxSource::LocalFolder(_) => {
            mailbox
                .folder
                .as_ref()
                .map_or(Expression::False, |folder| match folder.kind {
                    ImapFolderKind::Smart => stored_filter(&folder.filter),
                    ImapFolderKind::Manual | ImapFolderKind::Private => Expression::True,
                })
        }
    };
    let (filter, fingerprint) =
        Expression::And(vec![authorization.clone(), own_filter]).into_canonical_with_fingerprint();
    ImapMembershipDefinition {
        expected_mode_generation: Some(account.mode_generation),
        filter,
        fingerprint,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sandpost_core::{ImapAccountIdentifier, ImapFolderIdentifier, ViewIdentifier};

    /// Build an account for planning.
    fn account(mirror_views: bool) -> ImapAccount {
        ImapAccount {
            identifier: ImapAccountIdentifier::new(),
            owner_identifier: UserIdentifier::new(),
            name: "Debug".into(),
            username: "debug".into(),
            enabled: true,
            mirror_views,
            mode: if mirror_views {
                ImapAccountMode::Mirror
            } else {
                ImapAccountMode::Independent
            },
            mode_generation: 0,
            created_at: 0,
            updated_at: 0,
            last_used_at: None,
        }
    }

    /// Build a View with a name and filter.
    fn view(name: &str, filter: &str) -> View {
        View {
            identifier: ViewIdentifier::new(),
            owner_identifier: None,
            name: name.into(),
            icon: None,
            filter: filter.into(),
        }
    }

    /// Build a local folder.
    fn folder(name: &str, filter: &str, include_in_inbox: bool) -> ImapLocalFolder {
        ImapLocalFolder {
            identifier: ImapFolderIdentifier::new(),
            account_identifier: ImapAccountIdentifier::new(),
            kind: ImapFolderKind::Smart,
            definition_identifier: None,
            name: name.into(),
            filter: filter.into(),
            include_in_inbox,
            created_at: 0,
            updated_at: 0,
        }
    }

    /// Mirroring lists INBOX, Trash, every View, then folders, with collisions disambiguated.
    #[test]
    fn mirror_plans_every_view_and_resolves_collisions() {
        let views = [
            view("Laravel", "true"),
            view("inbox", "true"),
            view("Billing/EU", "true"),
            view("laravel", "true"),
        ];
        let folders = [folder("Laravel", "true", true)];
        let names: Vec<String> = plan(&account(true), &views, &[], &folders)
            .into_iter()
            .map(|planned| planned.name)
            .collect();
        assert_eq!(names[..2], ["INBOX", "Trash"]);
        assert!(names.contains(&"Billing-EU".to_owned()));
        assert!(names.contains(&"inbox (2)".to_owned()));
        let laravel: Vec<_> = names
            .iter()
            .filter(|name| name.starts_with("Laravel") || name.starts_with("laravel"))
            .collect();
        assert_eq!(laravel.len(), 3, "{names:?}");
        assert!(names.last().unwrap().starts_with("Laravel ("), "{names:?}");
    }

    /// Independent mode ignores inherited linked Views.
    #[test]
    fn independent_mode_ignores_linked_views() {
        let visible = [view("Laravel", "true"), view("Billing", "true")];
        let hidden = view("Other user's", "true");
        let linked = [
            ImapLinkedView {
                view_identifier: visible[0].identifier,
                alias: Some("Framework".into()),
            },
            ImapLinkedView {
                view_identifier: hidden.identifier,
                alias: None,
            },
        ];
        let names: Vec<String> = plan(&account(false), &visible, &linked, &[])
            .into_iter()
            .map(|planned| planned.name)
            .collect();
        assert_eq!(names, ["INBOX", "Trash"]);
    }

    /// Definitions always include authorization; INBOX supplies fallback with no smart selection.
    #[test]
    fn definitions_compose_authorization() {
        let authorization = sandpost_query::compile("from.domain == \"allowed.test\"")
            .unwrap()
            .into_expression();
        let views = [view("Laravel", "subject contains \"laravel\"")];
        let folders = [
            folder("Resets", "subject contains \"reset\"", true),
            folder("Private", "subject contains \"secret\"", false),
        ];
        let selected = account(false);
        let linked = [ImapLinkedView {
            view_identifier: views[0].identifier,
            alias: None,
        }];
        let planned = plan(&selected, &views, &linked, &folders);
        let inbox = definition(&selected, &planned, &planned[0], &authorization);
        let facts = |subject: &str, domain: &str| sandpost_core::MessageFacts {
            subject: subject.into(),
            from: vec![sandpost_core::Mailbox {
                address: format!("a@{domain}"),
                domain: domain.into(),
            }],
            ..Default::default()
        };
        assert!(
            inbox
                .filter
                .evaluate(&facts("laravel news", "allowed.test"))
        );
        assert!(
            inbox
                .filter
                .evaluate(&facts("password reset", "allowed.test"))
        );
        assert!(inbox.filter.evaluate(&facts("secret", "allowed.test")));
        assert!(!inbox.filter.evaluate(&facts("laravel news", "other.test")));
        let empty = plan(&selected, &[], &[], &[]);
        assert_eq!(
            definition(&selected, &empty, &empty[0], &authorization).filter,
            authorization.clone().into_canonical_with_fingerprint().0
        );
        let mirror = account(true);
        let mirrored = plan(&mirror, &views, &[], &[]);
        let all = definition(&mirror, &mirrored, &mirrored[0], &authorization);
        assert!(all.filter.evaluate(&facts("anything", "allowed.test")));
        assert!(!all.filter.evaluate(&facts("anything", "other.test")));
        let trash = definition(&mirror, &mirrored, &mirrored[1], &authorization);
        assert_eq!(trash.fingerprint, all.fingerprint);
        let broken = [view("Broken", "subject ===")];
        let planned = plan(&mirror, &broken, &[], &[]);
        assert_eq!(
            definition(&mirror, &planned, &planned[2], &Expression::True).filter,
            Expression::False,
            "a filter that no longer compiles fails closed"
        );
    }
}
