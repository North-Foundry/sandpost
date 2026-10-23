//! One SMTP mail transaction with per-recipient outcomes, and connection tests.
use crate::destination::DestinationPolicy;
use crate::failure::{Phase, classify, failure, is_transient, timeout};
use crate::session::{OutboundConnection, Session, open};
use lettre::Address;
use lettre::transport::smtp::authentication::Mechanism;
use lettre::transport::smtp::commands::{Data, Mail, Rcpt};
use lettre::transport::smtp::extension::{Extension, MailBodyParameter, MailParameter};
use lettre::transport::smtp::response::Response;
use sandpost_core::{OutboundFailure, OutboundFailureKind};
use std::net::SocketAddr;
use std::str::FromStr;
use std::time::{Duration, Instant};

/// The time RFC 5321 recommends waiting for the reply to the end of the message data.
const MINIMUM_DATA_TIMEOUT: Duration = Duration::from_secs(600);

/// The SMTP envelope of one delivery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboundEnvelope {
    /// The reverse path; `None` sends the null reverse path `<>`.
    pub sender: Option<String>,
    /// The forward paths, each offered with its own `RCPT TO`.
    pub recipients: Vec<String>,
}

/// What happened to one recipient of a delivery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecipientOutcome {
    /// The server accepted the message for this recipient; the final reply is kept.
    Delivered { response: String },
    /// Not delivered for a transient reason; a later attempt may succeed.
    Deferred(OutboundFailure),
    /// Not delivered and retrying will not help.
    Failed(OutboundFailure),
}

/// The result of one delivery attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryReport {
    /// The server address the session used, when one was reached.
    pub address: Option<SocketAddr>,
    /// One outcome per envelope recipient, in envelope order.
    pub recipients: Vec<(String, RecipientOutcome)>,
    /// The failure that affected the whole transaction, if any.
    pub error: Option<OutboundFailure>,
}

/// The result of a connection test.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionReport {
    /// Whether connection, TLS and authentication all succeeded.
    pub success: bool,
    pub address: Option<SocketAddr>,
    /// Whether the session ended up protected by TLS.
    pub encrypted: bool,
    pub authenticated: bool,
    /// The ESMTP extensions the server announced that Sandpost recognizes.
    pub extensions: Vec<String>,
    pub duration: Duration,
    pub error: Option<OutboundFailure>,
}

/**
 * Turn a failure into the outcome of a recipient that it prevented from being delivered.
 */
fn undelivered(failure: OutboundFailure) -> RecipientOutcome {
    if is_transient(&failure) {
        RecipientOutcome::Deferred(failure)
    } else {
        RecipientOutcome::Failed(failure)
    }
}

/**
 * Format an SMTP reply as its code followed by its text lines.
 */
fn reply_text(response: &Response) -> String {
    let lines: Vec<&str> = response.message().collect();
    format!("{} {}", u16::from(response.code()), lines.join(" "))
        .trim_end()
        .to_owned()
}

/**
 * Build a report in which every recipient shares the same failure.
 */
fn whole_failure(
    envelope: &OutboundEnvelope,
    address: Option<SocketAddr>,
    failure: OutboundFailure,
) -> DeliveryReport {
    DeliveryReport {
        address,
        recipients: envelope
            .recipients
            .iter()
            .map(|recipient| (recipient.clone(), undelivered(failure.clone())))
            .collect(),
        error: Some(failure),
    }
}

/**
 * Deliver `message` to the envelope's recipients through one SMTP session.
 *
 * Every recipient is offered with its own `RCPT TO`: refused ones get their own permanent or
 * deferred outcome, and the message data goes only to the accepted ones. A failure before the
 * transaction (resolution, connection, TLS, authentication, `MAIL FROM`) or of the data itself
 * applies to every recipient still undecided. Message data containing 8-bit bytes is sent with
 * `BODY=8BITMIME` and refused permanently by servers that do not announce 8BITMIME.
 *
 * When the connection drops or times out after the data was sent, the outcome is deferred even
 * though the server may already have accepted the message: retrying can then deliver a
 * duplicate, which SMTP cannot rule out.
 */
pub async fn deliver(
    connection: &OutboundConnection,
    policy: &DestinationPolicy,
    envelope: &OutboundEnvelope,
    message: &[u8],
) -> DeliveryReport {
    let sender = match envelope
        .sender
        .as_deref()
        .map(Address::from_str)
        .transpose()
    {
        Ok(sender) => sender,
        Err(error) => {
            return whole_failure(
                envelope,
                None,
                failure(
                    OutboundFailureKind::Configuration,
                    format!("the envelope sender is not a valid address: {error}"),
                ),
            );
        }
    };
    let mut session = match open(connection, policy).await {
        Ok(session) => session,
        Err(refusal) => return whole_failure(envelope, refusal.address, refusal.failure),
    };
    let address = Some(session.address);
    let mut parameters = Vec::new();
    if !message.is_ascii() {
        if !session
            .connection
            .server_info()
            .supports_feature(Extension::EightBitMime)
        {
            session.close(connection.command_timeout).await;
            return whole_failure(
                envelope,
                address,
                failure(
                    OutboundFailureKind::Permanent,
                    "the message contains 8-bit data but the server does not announce 8BITMIME",
                ),
            );
        }
        parameters.push(MailParameter::Body(MailBodyParameter::EightBitMime));
    }
    match tokio::time::timeout(
        connection.command_timeout,
        session.connection.command(Mail::new(sender, parameters)),
    )
    .await
    {
        Ok(Ok(_)) => {}
        Ok(Err(error)) => {
            session.close(connection.command_timeout).await;
            return whole_failure(envelope, address, classify(&error, Phase::Envelope));
        }
        Err(_) => {
            abandon_in_place(&mut session).await;
            return whole_failure(
                envelope,
                address,
                timeout(Phase::Envelope, connection.command_timeout),
            );
        }
    }
    let mut outcomes: Vec<(String, Option<RecipientOutcome>)> = Vec::new();
    let mut broken: Option<OutboundFailure> = None;
    for recipient in &envelope.recipients {
        if let Some(failure) = &broken {
            outcomes.push((recipient.clone(), Some(undelivered(failure.clone()))));
            continue;
        }
        let parsed = match Address::from_str(recipient) {
            Ok(parsed) => parsed,
            Err(error) => {
                outcomes.push((
                    recipient.clone(),
                    Some(RecipientOutcome::Failed(failure(
                        OutboundFailureKind::Permanent,
                        format!("the recipient is not a valid address: {error}"),
                    ))),
                ));
                continue;
            }
        };
        match tokio::time::timeout(
            connection.command_timeout,
            session.connection.command(Rcpt::new(parsed, Vec::new())),
        )
        .await
        {
            Ok(Ok(_)) => outcomes.push((recipient.clone(), None)),
            Ok(Err(error)) => {
                let failure = classify(&error, Phase::Envelope);
                if failure.code.is_none() {
                    broken = Some(failure.clone());
                }
                outcomes.push((recipient.clone(), Some(undelivered(failure))));
            }
            Err(_) => {
                let failure = timeout(Phase::Envelope, connection.command_timeout);
                broken = Some(failure.clone());
                outcomes.push((recipient.clone(), Some(undelivered(failure))));
            }
        }
    }
    let accepted = outcomes
        .iter()
        .filter(|(_, outcome)| outcome.is_none())
        .count();
    let data_result = if broken.is_some() {
        abandon_in_place(&mut session).await;
        Err(broken.clone().expect("checked above"))
    } else if accepted == 0 {
        session.close(connection.command_timeout).await;
        return DeliveryReport {
            address,
            recipients: outcomes
                .into_iter()
                .map(|(recipient, outcome)| (recipient, outcome.expect("every recipient refused")))
                .collect(),
            error: None,
        };
    } else {
        send_data(&mut session, connection, message).await
    };
    let error = data_result.as_ref().err().cloned();
    let recipients = outcomes
        .into_iter()
        .map(|(recipient, outcome)| {
            let outcome = outcome.unwrap_or_else(|| match &data_result {
                Ok(response) => RecipientOutcome::Delivered {
                    response: response.clone(),
                },
                Err(failure) => undelivered(failure.clone()),
            });
            (recipient, outcome)
        })
        .collect();
    if data_result.is_ok() {
        session.close(connection.command_timeout).await;
    }
    DeliveryReport {
        address,
        recipients,
        error,
    }
}

/**
 * Abandon a session that a phase left unusable, without waiting for a broken server.
 */
async fn abandon_in_place(session: &mut Session) {
    let _ = tokio::time::timeout(Duration::from_secs(1), session.connection.abort()).await;
}

/**
 * Send `DATA` and the message, returning the final reply text.
 *
 * The transport adds the terminating `CRLF . CRLF`, so one final CRLF of the message is not sent
 * twice. The data phase waits at least the ten minutes RFC 5321 recommends. A failed or timed-out
 * data phase aborts the connection.
 */
async fn send_data(
    session: &mut Session,
    connection: &OutboundConnection,
    message: &[u8],
) -> Result<String, OutboundFailure> {
    match tokio::time::timeout(connection.command_timeout, session.connection.command(Data)).await {
        Ok(Ok(_)) => {}
        Ok(Err(error)) => {
            abandon_in_place(session).await;
            return Err(classify(&error, Phase::Data));
        }
        Err(_) => {
            abandon_in_place(session).await;
            return Err(timeout(Phase::Data, connection.command_timeout));
        }
    }
    let content = message.strip_suffix(b"\r\n").unwrap_or(message);
    let limit = connection.command_timeout.max(MINIMUM_DATA_TIMEOUT);
    match tokio::time::timeout(limit, session.connection.message(content)).await {
        Ok(Ok(response)) => Ok(reply_text(&response)),
        Ok(Err(error)) => {
            abandon_in_place(session).await;
            Err(classify(&error, Phase::Data))
        }
        Err(_) => {
            abandon_in_place(session).await;
            Err(timeout(Phase::Data, limit))
        }
    }
}

/**
 * Connect, negotiate TLS and authenticate exactly as a delivery would, then quit without
 * starting a mail transaction, so nothing is delivered.
 */
pub async fn test_connection(
    connection: &OutboundConnection,
    policy: &DestinationPolicy,
) -> ConnectionReport {
    let started = Instant::now();
    match open(connection, policy).await {
        Ok(session) => {
            let information = session.connection.server_info();
            let mut extensions = Vec::new();
            for (extension, name) in [
                (Extension::StartTls, "STARTTLS"),
                (Extension::EightBitMime, "8BITMIME"),
                (Extension::SmtpUtfEight, "SMTPUTF8"),
            ] {
                if information.supports_feature(extension) {
                    extensions.push(name.to_owned());
                }
            }
            for (mechanism, name) in [
                (Mechanism::Plain, "AUTH PLAIN"),
                (Mechanism::Login, "AUTH LOGIN"),
                (Mechanism::Xoauth2, "AUTH XOAUTH2"),
            ] {
                if information.supports_auth_mechanism(mechanism) {
                    extensions.push(name.to_owned());
                }
            }
            let report = ConnectionReport {
                success: true,
                address: Some(session.address),
                encrypted: session.encrypted,
                authenticated: session.authenticated,
                extensions,
                duration: Duration::ZERO,
                error: None,
            };
            session.close(connection.command_timeout).await;
            ConnectionReport {
                duration: started.elapsed(),
                ..report
            }
        }
        Err(refusal) => ConnectionReport {
            success: false,
            address: refusal.address,
            encrypted: refusal.encrypted,
            authenticated: false,
            extensions: Vec::new(),
            duration: started.elapsed(),
            error: Some(refusal.failure),
        },
    }
}
