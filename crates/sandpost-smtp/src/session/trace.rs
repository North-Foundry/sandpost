//! SMTP trace fields added at capture, independently of the client's SIZE allowance.
use mail_parser::DateTime;
use sandpost_core::Message;
use sandpost_mime::Envelope;
use std::{
    net::IpAddr,
    time::{SystemTime, UNIX_EPOCH},
};

/// Validated session information used to construct the final delivery trace.
pub(super) struct Trace<'session> {
    pub(super) client_name: &'session str,
    pub(super) server_name: &'session str,
    pub(super) peer_address: Option<IpAddr>,
    pub(super) extended: bool,
    pub(super) authenticated: bool,
    pub(super) encrypted: bool,
}

/// Generated trace fields, with their unfolded values for the searchable facts.
pub(super) struct TraceHeaders {
    received: String,
    return_path: String,
}

impl Trace<'_> {
    /// Record the trusted peer IP and the actual SMTP, AUTH and TLS state with a numeric UTC offset.
    pub(super) fn headers(&self, envelope: &Envelope) -> TraceHeaders {
        let peer = self
            .peer_address
            .map(address_literal)
            .map(|address| format!(" ({address})"))
            .unwrap_or_default();
        let protocol = if self.encrypted && self.authenticated {
            "ESMTPSA"
        } else if self.authenticated {
            "ESMTPA"
        } else if self.encrypted {
            "ESMTPS"
        } else if self.extended {
            "ESMTP"
        } else {
            "SMTP"
        };
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let date = DateTime::from_timestamp(now).to_rfc822();
        TraceHeaders {
            received: format!(
                "from {}{peer}\r\n\tby {} with {protocol};\r\n\t{date}",
                self.client_name, self.server_name
            ),
            return_path: format!(
                "<{}>",
                envelope
                    .sender
                    .as_ref()
                    .map(|sender| sender.address.as_str())
                    .unwrap_or_default()
            ),
        }
    }
}

impl TraceHeaders {
    /// Prepend the final return path and a Received field, retaining existing Received order and body bytes.
    ///
    /// Existing Return-Path fields are removed at final capture so exactly one authoritative
    /// envelope sender remains. Submitted SIZE is enforced before adding these server fields.
    pub(super) fn prepend(self, message: &mut Message) {
        let mut received_message = format!(
            "Return-Path: {}\r\nReceived: {}\r\n",
            self.return_path, self.received
        )
        .into_bytes();
        let mut skipping_return_path = false;
        let mut reading_headers = true;
        for line in message.raw_message.split_inclusive(|byte| *byte == b'\n') {
            if reading_headers {
                if line == b"\r\n" {
                    reading_headers = false;
                    skipping_return_path = false;
                } else if !line.starts_with(b" ") && !line.starts_with(b"\t") {
                    skipping_return_path = line
                        .split(|byte| *byte == b':')
                        .next()
                        .is_some_and(|name| name.eq_ignore_ascii_case(b"Return-Path"));
                }
            }
            if !skipping_return_path {
                received_message.extend_from_slice(line);
            }
        }
        message.raw_message = received_message;
        message.facts.size = message.raw_message.len() as u64;
        message
            .facts
            .headers
            .insert("return-path".to_owned(), vec![self.return_path]);
        message
            .facts
            .headers
            .entry("received".to_owned())
            .or_default()
            .insert(0, self.received.replace("\r\n\t", " "));
    }
}

/// Format a trusted IP address using SMTP's address-literal syntax.
fn address_literal(address: IpAddr) -> String {
    match address {
        IpAddr::V4(address) => format!("[{address}]"),
        IpAddr::V6(address) => format!("[IPv6:{address}]"),
    }
}
