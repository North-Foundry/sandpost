//! Instance setup, native-held sessions, and authentication checks for operations and events.

use crate::NativeRuntime;
use crate::boundary::{authentication_error, instance_error, serialize};
use napi::{Error, Result, Status};
use napi_derive::napi;
use sandpost_application::{InstanceError, LoginRequest, LoginResponse, SessionUser, SetupRequest};
use serde_json::Value;

#[napi]
impl NativeRuntime {
    /// Report installation state without exposing the bootstrap secret.
    #[napi]
    pub fn instance_status(&self) -> Result<Value> {
        let status = self
            .application
            .instance()
            .status()
            .map_err(instance_error)?;
        serialize(serde_json::json!({
            "initialized": status.initialized,
            "setupKeyRequired": false,
        }))
    }

    /// Initialize the owner account with the bootstrap secret held by this trusted host.
    #[napi]
    pub async fn setup(&self, input: Value) -> Result<()> {
        self.ensure_running()?;
        let mut request: SetupRequest = serde_json::from_value(input)
            .map_err(|_| Error::new(Status::InvalidArg, "invalid setup input"))?;
        request.bootstrap_secret = self
            .application
            .instance()
            .bootstrap_secret_for_startup()
            .map_err(instance_error)?
            .ok_or_else(|| instance_error(InstanceError::AlreadyInitialized))?;
        self.application
            .instance()
            .setup(request)
            .await
            .map_err(instance_error)?;
        Ok(())
    }

    /// Authenticate and retain the token inside the native runtime.
    #[napi]
    pub async fn login(&self, input: Value) -> Result<Value> {
        self.ensure_running()?;
        let request: LoginRequest = serde_json::from_value(input)
            .map_err(|_| Error::new(Status::InvalidArg, "invalid login input"))?;
        let LoginResponse { token, user } = self
            .application
            .instance()
            .login(request)
            .await
            .map_err(|error| match error {
                InstanceError::Authentication => {
                    Error::new(Status::GenericFailure, "invalid email or password")
                }
                error => instance_error(error),
            })?;
        let previous_token = self.session_token.send_replace(Some(token));
        if let Some(previous_token) = previous_token {
            self.application
                .instance()
                .logout(&previous_token)
                .map_err(instance_error)?;
        }
        serialize(user)
    }

    /// Return the current user only after revalidating the native-held token.
    #[napi]
    pub async fn session(&self) -> Result<Value> {
        self.ensure_running()?;
        let Some(token) = self.session_token.borrow().clone() else {
            return Ok(Value::Null);
        };
        match self.application.instance().session(&token).await {
            Ok(user) => serialize(user),
            Err(InstanceError::Authentication) => {
                if self.session_token.borrow().as_deref() == Some(&token) {
                    self.session_token.send_replace(None);
                }
                Ok(Value::Null)
            }
            Err(error) => Err(instance_error(error)),
        }
    }

    /// Clear the native-held token and invalidate its application session.
    #[napi]
    pub async fn logout(&self) -> Result<()> {
        self.ensure_running()?;
        if let Some(token) = self.session_token.send_replace(None) {
            self.application
                .instance()
                .logout(&token)
                .map_err(instance_error)?;
        }
        Ok(())
    }
}

impl NativeRuntime {
    /// Require a live process session before every native inbox and message operation.
    pub(super) async fn ensure_authenticated(&self) -> Result<SessionUser> {
        self.ensure_running()?;
        let token = self
            .session_token
            .borrow()
            .clone()
            .ok_or_else(authentication_error)?;
        self.application
            .instance()
            .session(&token)
            .await
            .map_err(instance_error)
    }

    /// Return the retained token only while the shared instance still accepts it.
    pub(super) async fn valid_session_token(&self) -> Result<Option<String>> {
        let Some(token) = self.session_token.borrow().clone() else {
            return Ok(None);
        };
        match self.application.instance().session(&token).await {
            Ok(_) => Ok(Some(token)),
            Err(InstanceError::Authentication) => {
                if self.session_token.borrow().as_deref() == Some(&token) {
                    self.session_token.send_replace(None);
                }
                Ok(None)
            }
            Err(error) => Err(instance_error(error)),
        }
    }

    /// Revalidate a token immediately before returning an event to native callers.
    pub(super) async fn session_is_current(&self, token: &str) -> Result<bool> {
        if self.session_token.borrow().as_deref() != Some(token) {
            return Ok(false);
        }
        match self.application.instance().session(token).await {
            Ok(_) => Ok(self.session_token.borrow().as_deref() == Some(token)),
            Err(InstanceError::Authentication) => {
                if self.session_token.borrow().as_deref() == Some(token) {
                    self.session_token.send_replace(None);
                }
                Ok(false)
            }
            Err(error) => Err(instance_error(error)),
        }
    }
}
