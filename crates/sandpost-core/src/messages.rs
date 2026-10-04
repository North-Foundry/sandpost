//! Normalized mail facts, original message bytes, and attachment metadata.
use crate::MessageIdentifier;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mailbox {
    pub address: String,
    pub domain: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageFacts {
    pub envelope_from: Option<Mailbox>,
    pub envelope_to: Vec<Mailbox>,
    pub from: Vec<Mailbox>,
    pub to: Vec<Mailbox>,
    #[serde(rename = "cc")]
    pub carbon_copy: Vec<Mailbox>,
    pub subject: String,
    pub text: String,
    #[serde(rename = "html")]
    pub markup_body: String,
    #[serde(rename = "message_id")]
    pub message_identifier: Option<String>,
    /// Unix seconds, UTC. DSL timestamp values use this same unit.
    pub received_at: i64,
    pub size: u64,
    pub attachment_count: u64,
    /// Lowercase header names; duplicate values are preserved.
    pub headers: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attachment {
    pub filename: Option<String>,
    pub content_type: String,
    pub size: u64,
    /// SHA-256 reference for a future content-addressed filesystem store.
    pub content_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    #[serde(rename = "id")]
    pub identifier: MessageIdentifier,
    pub facts: MessageFacts,
    #[serde(rename = "raw_mime")]
    pub raw_message: Vec<u8>,
    pub attachments: Vec<Attachment>,
}
