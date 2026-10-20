//! Signing in without blocking: log in, and register if the account doesn't
//! exist yet, as [`Bot::sign_in`] does, but one [`NetEvent`] at a time so a
//! game loop can drive it. It never touches the network itself.
//!
//! ```no_run
//! # use capitol_core::{bot::Account, net::*, signin::*};
//! # let net = Net::spawn(NetConfig::default()).unwrap();
//! let (mut sign_in, request) = SignIn::start(Account::numbered("flint", 1), 2);
//! net.rest(request, sign_in.tag());
//! // once per frame:
//! for event in net.drain() {
//!     match sign_in.observe(&event) {
//!         Some(Step::Send(request)) => net.rest(request, sign_in.tag()),
//!         Some(Step::SignedIn(auth)) => net.connect(&*auth),
//!         Some(Step::Failed(why)) => eprintln!("{why}"),
//!         None => {}
//!     }
//! }
//! ```
//!
//! [`Bot::sign_in`]: crate::bot::Bot::sign_in

use capitol_protocol::rest::{AuthResponse, LoginRequest, RegisterRequest};

use crate::bot::Account;
use crate::net::{NetEvent, RestError, RestRequest, RestResponse, Tag};

/// Where a sign-in is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    LoggingIn,
    Registering,
    Over,
}

#[derive(Debug, Clone)]
pub struct SignIn {
    email: String,
    /// Sent after a login's 401, when the sign-in may register. `None` for
    /// [`SignIn::log_in`], and once it has been sent.
    register: Option<RegisterRequest>,
    tag: Tag,
    phase: Phase,
}

/// What a sign-in wants next.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// Send this, with [`SignIn::tag`].
    Send(RestRequest),
    /// Done: `connect` with the token.
    SignedIn(Box<AuthResponse>),
    /// Done, without a session. Nothing more is sent.
    Failed(Failure),
}

/// Why a sign-in ended without a session.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{}: {error}", if *.registering { "register" } else { "login" })]
pub struct Failure {
    /// The register failed, not the login.
    pub registering: bool,
    pub error: RestError,
}

impl SignIn {
    /// Logs in, and registers the account if the login gets a 401: a bot's
    /// way in, where the account is made on first use. Send the request with
    /// `tag`; the answers carry it.
    pub fn start(account: Account, tag: Tag) -> (SignIn, RestRequest) {
        let (mut sign_in, request) = SignIn::log_in(&account.email, &account.password, tag);
        sign_in.register = Some(RegisterRequest {
            email: account.email,
            password: account.password,
            player_name: account.name,
            refresh: true,
        });
        (sign_in, request)
    }

    /// Logs in only. A 401 is a failure: a person who mistyped their password
    /// must not get a new account.
    pub fn log_in(email: &str, password: &str, tag: Tag) -> (SignIn, RestRequest) {
        // Every sign-in asks for a refresh token, so the IO thread can keep
        // the session going past the access token's 15 minutes (E0.6).
        let request = RestRequest::Login(LoginRequest {
            email: email.to_owned(),
            password: password.to_owned(),
            refresh: true,
        });
        let sign_in = SignIn {
            email: email.to_owned(),
            register: None,
            tag,
            phase: Phase::LoggingIn,
        };
        (sign_in, request)
    }

    /// Registers a new account. A taken email fails with a 409.
    pub fn register(account: Account, tag: Tag) -> (SignIn, RestRequest) {
        let sign_in = SignIn {
            email: account.email.clone(),
            register: None,
            tag,
            phase: Phase::Registering,
        };
        let request = RestRequest::Register(RegisterRequest {
            email: account.email,
            password: account.password,
            player_name: account.name,
            refresh: true,
        });
        (sign_in, request)
    }

    pub fn tag(&self) -> Tag {
        self.tag
    }

    pub fn email(&self) -> &str {
        &self.email
    }

    /// Whether it has got as far as registering.
    pub fn is_registering(&self) -> bool {
        self.phase == Phase::Registering
    }

    /// Feed every [`NetEvent`]; only answers carrying [`SignIn::tag`] count.
    pub fn observe(&mut self, event: &NetEvent) -> Option<Step> {
        let NetEvent::Rest(tag, result) = event else {
            return None;
        };
        if *tag != self.tag || self.phase == Phase::Over {
            return None;
        }
        let registering = self.phase == Phase::Registering;
        let step = match result {
            Ok(RestResponse::Auth(auth)) => Step::SignedIn(Box::new(auth.clone())),
            Err(e) if !registering && e.is_unauthorized() && self.register.is_some() => {
                self.phase = Phase::Registering;
                return self
                    .register
                    .take()
                    .map(|r| Step::Send(RestRequest::Register(r)));
            }
            Err(e) => Step::Failed(Failure {
                registering,
                error: e.clone(),
            }),
            Ok(other) => Step::Failed(Failure {
                registering,
                error: RestError::Decode(format!("unexpected answer {other:?}")),
            }),
        };
        self.phase = Phase::Over;
        Some(step)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::RestError;
    use capitol_protocol::rest::AccountInfo;
    use uuid::Uuid;

    const TAG: Tag = 9;

    fn auth() -> RestResponse {
        RestResponse::Auth(AuthResponse {
            token: "t0ken".into(),
            expires_in: 900,
            refresh_token: Some("r".into()),
            account: AccountInfo {
                id: Uuid::nil(),
                email: "flint1@bots.capitol.local".into(),
                tier: "free".into(),
                is_admin: false,
                kind: Default::default(),
            },
            player_id: Uuid::nil(),
        })
    }

    fn status(code: u16) -> RestError {
        RestError::Http {
            status: code,
            message: "nope".into(),
        }
    }

    fn started() -> SignIn {
        let (sign_in, request) = SignIn::start(Account::numbered("flint", 1), TAG);
        assert!(matches!(request, RestRequest::Login(l) if l.email == "flint1@bots.capitol.local"));
        sign_in
    }

    #[test]
    fn a_known_account_logs_in() {
        let mut s = started();
        let step = s.observe(&NetEvent::Rest(TAG, Ok(auth())));
        assert!(matches!(step, Some(Step::SignedIn(a)) if a.token == "t0ken"));
        assert_eq!(s.observe(&NetEvent::Rest(TAG, Ok(auth()))), None);
    }

    #[test]
    fn an_unknown_account_registers() {
        let mut s = started();
        let step = s.observe(&NetEvent::Rest(TAG, Err(status(401))));
        assert!(
            matches!(step, Some(Step::Send(RestRequest::Register(r))) if r.player_name == "Flint1")
        );
        let step = s.observe(&NetEvent::Rest(TAG, Ok(auth())));
        assert!(matches!(step, Some(Step::SignedIn(_))));
    }

    #[test]
    fn other_failures_end_it() {
        let mut s = started();
        let step = s.observe(&NetEvent::Rest(TAG, Err(status(500))));
        assert!(matches!(step, Some(Step::Failed(f)) if f.to_string().starts_with("login")));

        let mut s = started();
        s.observe(&NetEvent::Rest(TAG, Err(status(401))));
        let step = s.observe(&NetEvent::Rest(TAG, Err(status(409))));
        assert!(
            matches!(step, Some(Step::Failed(f)) if f.registering && f.error.status() == Some(409))
        );
        assert_eq!(s.observe(&NetEvent::Rest(TAG, Ok(auth()))), None);
    }

    #[test]
    fn other_tags_and_events_are_ignored() {
        let mut s = started();
        assert_eq!(s.observe(&NetEvent::Rest(TAG + 1, Ok(auth()))), None);
        assert_eq!(s.observe(&NetEvent::Resync), None);
        assert!(s.observe(&NetEvent::Rest(TAG, Ok(auth()))).is_some());
    }

    #[test]
    fn a_login_only_sign_in_never_registers() {
        let (mut s, request) = SignIn::log_in("ada@capitol.local", "hunter22", TAG);
        assert!(matches!(request, RestRequest::Login(l) if l.password == "hunter22"));
        let step = s.observe(&NetEvent::Rest(TAG, Err(status(401))));
        assert!(
            matches!(step, Some(Step::Failed(f)) if !f.registering && f.error.is_unauthorized())
        );
        assert_eq!(s.observe(&NetEvent::Rest(TAG, Ok(auth()))), None);
    }

    #[test]
    fn a_register_sign_in_registers_straight_away() {
        let account = Account {
            email: "ada@capitol.local".into(),
            password: "hunter22".into(),
            name: "Ada".into(),
        };
        let (mut s, request) = SignIn::register(account, TAG);
        assert!(matches!(request, RestRequest::Register(r) if r.player_name == "Ada"));
        assert!(s.is_registering());
        let step = s.observe(&NetEvent::Rest(TAG, Ok(auth())));
        assert!(matches!(step, Some(Step::SignedIn(_))));
    }

    #[test]
    fn an_unreachable_server_ends_it() {
        let (mut s, _) = SignIn::log_in("ada@capitol.local", "hunter22", TAG);
        let refused = RestError::Transport("connection refused".into());
        let step = s.observe(&NetEvent::Rest(TAG, Err(refused)));
        assert!(matches!(step, Some(Step::Failed(f)) if f.error.status().is_none()));
    }
}
