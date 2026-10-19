//! Email client compatibility checks for authorized messages.

use crate::{ApplicationError, AuthorizedApplication, authorization};
use sandpost_core::MessageIdentifier;
pub use sandpost_html_check::CompatibilityReport;

impl AuthorizedApplication {
    /**
     * Check the original HTML body of an authorized message against the caniemail data.
     *
     * The check reads the captured MIME rather than the stored markup body, which may be generated
     * from plain text. A message without a `text/html` part, or whose MIME cannot be interpreted,
     * yields a report with `html_present` false. An HTML body above the check's size limit is
     * invalid input.
     */
    pub async fn check_message_html(
        &self,
        identifier: MessageIdentifier,
    ) -> Result<CompatibilityReport, ApplicationError> {
        let message = self
            .application
            .storage
            .get_user_message(self.user_identifier, identifier)
            .await?
            .ok_or(ApplicationError::NotFound)?;
        authorization::require_snapshot(
            &self.application.storage,
            self.user_identifier,
            &message.facts,
        )
        .await?;
        tokio::task::spawn_blocking(move || {
            match sandpost_mime::native_html_body(&message.raw_message) {
                Ok(Some(html)) => sandpost_html_check::check_html(&html)
                    .map_err(|error| ApplicationError::InvalidInput(error.to_string())),
                Ok(None) | Err(_) => Ok(CompatibilityReport::without_html()),
            }
        })
        .await?
    }
}
