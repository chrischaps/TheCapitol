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
//!         Some(Step::SignedIn(auth)) => net.connect(auth.token),
//!         Some(Step::Failed(why)) => eprintln!("{why}"),
//!         None => {}
//!     }
//! }
//! ```
//!
//! [`Bot::sign_in`]: crate::bot::Bot::sign_in

use capitol_protocol::rest::{AuthResponse, LoginRequest, RegisterRequest};

use crate::bot::Account;
use crate::net::{NetEvent, RestRequest, RestResponse, Tag};

/// Where a sign-in is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    LoggingIn,
    Registering,
    Over,
}

#[derive(Debug, Clone)]
pub struct SignIn {
    account: Account,
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
    Failed(String),
}

impl SignIn {
    /// Starts with a login. Send the request with `tag`; the answers carry it.
    pub fn start(account: Account, tag: Tag) -> (SignIn, RestRequest) {
        let request = RestRequest::Login(LoginRequest {
            email: account.email.clone(),
            password: account.password.clone(),
        });
        let sign_in = SignIn {
            account,
            tag,
            phase: Phase::LoggingIn,
        };
        (sign_in, request)
    }

    pub fn tag(&self) -> Tag {
        self.tag
    }

    pub fn account(&self) -> &Account {
        &self.account
    }

    /// Feed every [`NetEvent`]; only answers carrying [`SignIn::tag`] count.
    pub fn observe(&mut self, event: &NetEvent) -> Option<Step> {
        let NetEvent::Rest(tag, result) = event else {
            return None;
        };
        if *tag != self.tag || self.phase == Phase::Over {
            return None;
        }
        let step = match (self.phase, result) {
            (_, Ok(RestResponse::Auth(auth))) => Step::SignedIn(Box::new(auth.clone())),
            (Phase::LoggingIn, Err(e)) if e.is_unauthorized() => {
                self.phase = Phase::Registering;
                return Some(Step::Send(RestRequest::Register(RegisterRequest {
                    email: self.account.email.clone(),
                    password: self.account.password.clone(),
                    player_name: self.account.name.clone(),
                })));
            }
            (Phase::LoggingIn, Err(e)) => Step::Failed(format!("login: {e}")),
            (Phase::Registering, Err(e)) => Step::Failed(format!("register: {e}")),
            (_, Ok(other)) => Step::Failed(format!("unexpected answer {other:?}")),
            (Phase::Over, _) => unreachable!("answers after the end are dropped above"),
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
            account: AccountInfo {
                id: Uuid::nil(),
                email: "flint1@bots.capitol.local".into(),
                tier: "free".into(),
                is_admin: false,
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
        assert!(matches!(step, Some(Step::Failed(why)) if why.starts_with("login")));

        let mut s = started();
        s.observe(&NetEvent::Rest(TAG, Err(status(401))));
        let step = s.observe(&NetEvent::Rest(TAG, Err(status(409))));
        assert!(matches!(step, Some(Step::Failed(why)) if why.starts_with("register")));
        assert_eq!(s.observe(&NetEvent::Rest(TAG, Ok(auth()))), None);
    }

    #[test]
    fn other_tags_and_events_are_ignored() {
        let mut s = started();
        assert_eq!(s.observe(&NetEvent::Rest(TAG + 1, Ok(auth()))), None);
        assert_eq!(s.observe(&NetEvent::Resync), None);
        assert!(s.observe(&NetEvent::Rest(TAG, Ok(auth()))).is_some());
    }
}
