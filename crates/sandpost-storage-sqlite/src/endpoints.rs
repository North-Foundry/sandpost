//! SMTP endpoint persistence and per-user memberships on SQLite.
use crate::SqliteStorage;
use crate::error::StorageResult;
use crate::records::{invalid_column, parse_identifier};
use async_trait::async_trait;
use rusqlite::{Connection, OptionalExtension, params};
use sandpost_core::{
    EndpointIdentifier, EndpointMembership, EndpointRole, MailAccess, SmtpEndpoint, UserIdentifier,
};
use sandpost_storage::{EndpointStorage, StorageError};

const MEMBERSHIP_COLUMNS: &str = "user_identifier,endpoint_identifier,role,mail_access";

/// Return the stored text for an endpoint role.
pub(crate) fn endpoint_role_text(role: EndpointRole) -> &'static str {
    match role {
        EndpointRole::Admin => "admin",
        EndpointRole::Member => "member",
        EndpointRole::Viewer => "viewer",
    }
}

/// Return the stored text for a mail access mode.
pub(crate) fn mail_access_text(access: MailAccess) -> &'static str {
    match access {
        MailAccess::All => "all",
        MailAccess::Scoped => "scoped",
    }
}

fn parse_endpoint_role(value: &str) -> Option<EndpointRole> {
    match value {
        "admin" => Some(EndpointRole::Admin),
        "member" => Some(EndpointRole::Member),
        "viewer" => Some(EndpointRole::Viewer),
        _ => None,
    }
}

fn parse_mail_access(value: &str) -> Option<MailAccess> {
    match value {
        "all" => Some(MailAccess::All),
        "scoped" => Some(MailAccess::Scoped),
        _ => None,
    }
}

fn decode_endpoint(row: &rusqlite::Row<'_>) -> rusqlite::Result<SmtpEndpoint> {
    let identifier: String = row.get(0)?;
    Ok(SmtpEndpoint {
        identifier: parse_identifier(&identifier).map_err(|_| invalid_column(0, "identifier"))?,
        name: row.get(1)?,
    })
}

fn decode_membership(row: &rusqlite::Row<'_>) -> rusqlite::Result<EndpointMembership> {
    let user: String = row.get(0)?;
    let endpoint: String = row.get(1)?;
    let role: String = row.get(2)?;
    let access: String = row.get(3)?;
    Ok(EndpointMembership {
        user_identifier: user
            .parse()
            .map_err(|_| invalid_column(0, "user_identifier"))?,
        endpoint_identifier: endpoint
            .parse()
            .map_err(|_| invalid_column(1, "endpoint_identifier"))?,
        role: parse_endpoint_role(&role).ok_or_else(|| invalid_column(2, "role"))?,
        mail_access: parse_mail_access(&access).ok_or_else(|| invalid_column(3, "mail_access"))?,
    })
}

/// List configured endpoints in stable identifier order.
fn list_endpoints_blocking(connection: &Connection) -> Result<Vec<SmtpEndpoint>, StorageError> {
    let mut statement = connection
        .prepare("SELECT identifier, name FROM endpoints ORDER BY identifier")
        .storage()?;
    statement
        .query_map([], decode_endpoint)
        .storage()?
        .collect::<Result<Vec<_>, _>>()
        .storage()
}

/// Load one endpoint by identifier.
fn get_endpoint_blocking(
    connection: &Connection,
    identifier: EndpointIdentifier,
) -> Result<Option<SmtpEndpoint>, StorageError> {
    connection
        .query_row(
            "SELECT identifier, name FROM endpoints WHERE identifier=?1",
            [identifier.to_string()],
            decode_endpoint,
        )
        .optional()
        .storage()
}

/// Insert or update an endpoint while preserving its identifier.
fn save_endpoint_blocking(
    connection: &Connection,
    endpoint: &SmtpEndpoint,
) -> Result<(), StorageError> {
    connection
        .execute(
            "INSERT INTO endpoints(identifier,name) VALUES (?1,?2) ON CONFLICT(identifier) DO UPDATE SET name=excluded.name",
            params![endpoint.identifier.to_string(), endpoint.name],
        )
        .storage()?;
    Ok(())
}

/// Delete an endpoint, reporting whether a row was removed.
fn delete_endpoint_blocking(
    connection: &Connection,
    identifier: EndpointIdentifier,
) -> Result<bool, StorageError> {
    Ok(connection
        .execute(
            "DELETE FROM endpoints WHERE identifier=?1",
            [identifier.to_string()],
        )
        .storage()?
        > 0)
}

/// Load one user's membership on an endpoint.
fn get_membership_blocking(
    connection: &Connection,
    user: UserIdentifier,
    endpoint: EndpointIdentifier,
) -> Result<Option<EndpointMembership>, StorageError> {
    let query = format!(
        "SELECT {MEMBERSHIP_COLUMNS} FROM endpoint_memberships WHERE user_identifier=?1 AND endpoint_identifier=?2"
    );
    connection
        .query_row(
            &query,
            params![user.to_string(), endpoint.to_string()],
            decode_membership,
        )
        .optional()
        .storage()
}

/// List every membership of one endpoint ordered by user identifier.
fn list_endpoint_memberships_blocking(
    connection: &Connection,
    endpoint: EndpointIdentifier,
) -> Result<Vec<EndpointMembership>, StorageError> {
    let query = format!(
        "SELECT {MEMBERSHIP_COLUMNS} FROM endpoint_memberships WHERE endpoint_identifier=?1 ORDER BY user_identifier"
    );
    let mut statement = connection.prepare(&query).storage()?;
    statement
        .query_map([endpoint.to_string()], decode_membership)
        .storage()?
        .collect::<Result<Vec<_>, _>>()
        .storage()
}

/// List every endpoint membership held by one user.
fn list_user_endpoint_memberships_blocking(
    connection: &Connection,
    user: UserIdentifier,
) -> Result<Vec<EndpointMembership>, StorageError> {
    let query = format!(
        "SELECT {MEMBERSHIP_COLUMNS} FROM endpoint_memberships WHERE user_identifier=?1 ORDER BY endpoint_identifier"
    );
    let mut statement = connection.prepare(&query).storage()?;
    statement
        .query_map([user.to_string()], decode_membership)
        .storage()?
        .collect::<Result<Vec<_>, _>>()
        .storage()
}

/// Insert or update a membership on its composite key.
fn set_membership_blocking(
    connection: &Connection,
    membership: &EndpointMembership,
) -> Result<(), StorageError> {
    connection
        .execute(
            "INSERT INTO endpoint_memberships(user_identifier,endpoint_identifier,role,mail_access) VALUES (?1,?2,?3,?4) ON CONFLICT(user_identifier,endpoint_identifier) DO UPDATE SET role=excluded.role, mail_access=excluded.mail_access",
            params![
                membership.user_identifier.to_string(),
                membership.endpoint_identifier.to_string(),
                endpoint_role_text(membership.role),
                mail_access_text(membership.mail_access),
            ],
        )
        .storage()?;
    Ok(())
}

/// Remove a membership and every dependent scope membership for that endpoint.
///
/// Scope membership requires endpoint membership, so both are removed atomically to preserve
/// the membership invariant.
fn remove_membership_blocking(
    connection: &mut Connection,
    user: UserIdentifier,
    endpoint: EndpointIdentifier,
) -> Result<bool, StorageError> {
    let transaction = connection.transaction().storage()?;
    transaction
        .execute(
            "DELETE FROM scope_memberships WHERE user_identifier=?1 AND scope_identifier IN (SELECT identifier FROM scopes WHERE endpoint_identifier=?2)",
            params![user.to_string(), endpoint.to_string()],
        )
        .storage()?;
    let deleted = transaction
        .execute(
            "DELETE FROM endpoint_memberships WHERE user_identifier=?1 AND endpoint_identifier=?2",
            params![user.to_string(), endpoint.to_string()],
        )
        .storage()?
        > 0;
    transaction.commit().storage()?;
    Ok(deleted)
}

#[async_trait]
impl EndpointStorage for SqliteStorage {
    async fn get_endpoint(
        &self,
        identifier: EndpointIdentifier,
    ) -> Result<Option<SmtpEndpoint>, StorageError> {
        self.run(move |connection| get_endpoint_blocking(connection, identifier))
            .await
    }

    async fn list_endpoints(&self) -> Result<Vec<SmtpEndpoint>, StorageError> {
        self.run(|connection| list_endpoints_blocking(connection))
            .await
    }

    async fn save_endpoint(&self, endpoint: &SmtpEndpoint) -> Result<(), StorageError> {
        let endpoint = endpoint.clone();
        self.run(move |connection| save_endpoint_blocking(connection, &endpoint))
            .await
    }

    async fn delete_endpoint(&self, identifier: EndpointIdentifier) -> Result<bool, StorageError> {
        self.run(move |connection| delete_endpoint_blocking(connection, identifier))
            .await
    }

    async fn get_endpoint_membership(
        &self,
        user: UserIdentifier,
        endpoint: EndpointIdentifier,
    ) -> Result<Option<EndpointMembership>, StorageError> {
        self.run(move |connection| get_membership_blocking(connection, user, endpoint))
            .await
    }

    async fn list_endpoint_memberships(
        &self,
        endpoint: EndpointIdentifier,
    ) -> Result<Vec<EndpointMembership>, StorageError> {
        self.run(move |connection| list_endpoint_memberships_blocking(connection, endpoint))
            .await
    }

    async fn list_user_endpoint_memberships(
        &self,
        user: UserIdentifier,
    ) -> Result<Vec<EndpointMembership>, StorageError> {
        self.run(move |connection| list_user_endpoint_memberships_blocking(connection, user))
            .await
    }

    async fn set_endpoint_membership(
        &self,
        membership: &EndpointMembership,
    ) -> Result<(), StorageError> {
        let membership = membership.clone();
        self.run(move |connection| set_membership_blocking(connection, &membership))
            .await
    }

    async fn remove_endpoint_membership(
        &self,
        user: UserIdentifier,
        endpoint: EndpointIdentifier,
    ) -> Result<bool, StorageError> {
        self.run(move |connection| remove_membership_blocking(connection, user, endpoint))
            .await
    }
}
