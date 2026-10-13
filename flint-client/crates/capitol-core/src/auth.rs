//! The way in for a person, as [`signin`](crate::signin) is for a bot: the
//! login form's state, from signed out to in the world and back.
//!
//! [`Auth`] does no IO and never reads the clock. The game feeds it every
//! [`NetEvent`] and calls [`Auth::tick`] once per frame, and carries out the
//! [`Effect`]s it returns: REST calls, connecting the WebSocket, and writing
//! the remembered email and token to disk.
//!
//! - **A stored token is tried first.** [`Auth::new`] connects with it at
//!   once. The server rejecting it (expired after 24 h, or replaced by a
//!   login elsewhere) returns to the form with a [`Notice`] and forgets it.
//! - **A login never registers.** A mistyped password is a
//!   [`Notice::WrongPassword`], never a new account. Registering is its own
//!   form.
//! - **The server's health is known before you submit.** While signed out,
//!   `GET /health` goes every [`HEALTH_EVERY`], so the form can say the
//!   server is down rather than wait for a login to fail.
//! - **The email is remembered once it signs in**, so a typo isn't. The
//!   password is never kept.

use std::fmt;
use std::time::{Duration, Instant};

use capitol_protocol::rest::LogoutRequest;

use crate::bot::Account;
use crate::net::{ConnectionState, NetEvent, RestError, RestRequest, Tag};
use crate::signin::{Failure, SignIn, Step};

/// How often to ask whether the server is up, while signed out.
pub const HEALTH_EVERY: Duration = Duration::from_secs(5);

/// The server's `auth_error` when another login took the account's one
/// session (`middleware/auth.rs`).
const REPLACED: &str = "Session expired";

/// Player names the form accepts, as the React client's form does. The
/// server itself checks nothing.
pub const NAME_CHARS: std::ops::RangeInclusive<usize> = 2..=32;
/// The shortest password the register form accepts (React's rule).
pub const PASSWORD_MIN: usize = 8;

/// What's kept on disk between runs.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Remembered {
    /// The last email that signed in, to fill the form with.
    pub email: Option<String>,
    /// That session's token, tried before the form is shown.
    pub token: Option<String>,
}

/// Where the player is on the way in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// No session: the form is up.
    SignedOut,
    /// A login or register is waiting on the server.
    SigningIn { registering: bool },
    /// Signed in, and the WebSocket is opening. `resumed` when the token is
    /// the stored one from an earlier run.
    Entering { resumed: bool },
    /// Connected at least once. If the connection drops it comes back on its
    /// own (P2.2); only the server rejecting the token ends it.
    InWorld,
}

/// What the game must do for [`Auth`].
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    Rest(RestRequest, Tag),
    /// `Net::connect` with this token.
    Connect(String),
    /// `Net::disconnect`.
    Disconnect,
    /// Write this to disk, replacing what was there.
    Remember(Remembered),
}

/// Why the form is showing a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    /// The form's own checks, before anything is sent.
    Invalid(&'static str),
    /// A login's 401.
    WrongPassword,
    /// A register's 409.
    EmailTaken,
    /// No answer from the server.
    Unreachable,
    /// Anything else the server said, in its words.
    Server(String),
    /// The server rejected the session's token. `elsewhere` when another
    /// login took the account.
    SessionEnded { elsewhere: bool },
}

impl fmt::Display for Notice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Notice::Invalid(why) => f.write_str(why),
            Notice::WrongPassword => f.write_str("That email and password don't match."),
            Notice::EmailTaken => {
                f.write_str("That email already has an account. Sign in instead.")
            }
            Notice::Unreachable => f.write_str("Can't reach the server."),
            Notice::Server(message) => f.write_str(message),
            Notice::SessionEnded { elsewhere: true } => {
                f.write_str("This account signed in somewhere else.")
            }
            Notice::SessionEnded { elsewhere: false } => {
                f.write_str("Your session has ended. Sign in again.")
            }
        }
    }
}

impl From<&Failure> for Notice {
    fn from(failure: &Failure) -> Notice {
        match (&failure.error, failure.registering) {
            (RestError::Transport(_), _) => Notice::Unreachable,
            (RestError::Http { status: 401, .. }, false) => Notice::WrongPassword,
            (RestError::Http { status: 409, .. }, true) => Notice::EmailTaken,
            (RestError::Http { message, .. }, _) => Notice::Server(message.clone()),
            (RestError::Decode(what), _) => Notice::Server(format!("Unexpected answer: {what}")),
        }
    }
}

#[derive(Debug)]
pub struct Auth {
    phase: Phase,
    /// The sign-in's tag; health and logout use the next two.
    tag: Tag,
    remembered: Remembered,
    sign_in: Option<SignIn>,
    notice: Option<Notice>,
    /// `None` until the server has answered, or failed to, once.
    reachable: Option<bool>,
    health_due: Option<Instant>,
    health_in_flight: bool,
    /// The WebSocket, since the last sign-in.
    connection: Option<ConnectionState>,
}

impl Auth {
    /// Starts from what was on disk: entering with the stored token if there
    /// is one, otherwise signed out. Uses tags `tag`, `tag + 1` and `tag + 2`.
    pub fn new(remembered: Remembered, tag: Tag) -> (Auth, Vec<Effect>) {
        let mut effects = Vec::new();
        let phase = match &remembered.token {
            Some(token) => {
                effects.push(Effect::Connect(token.clone()));
                Phase::Entering { resumed: true }
            }
            None => Phase::SignedOut,
        };
        let auth = Auth {
            phase,
            tag,
            remembered,
            sign_in: None,
            notice: None,
            reachable: None,
            health_due: None,
            health_in_flight: false,
            connection: None,
        };
        (auth, effects)
    }

    fn health_tag(&self) -> Tag {
        self.tag + 1
    }

    fn logout_tag(&self) -> Tag {
        self.tag + 2
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// The message for the form, if there is one.
    pub fn notice(&self) -> Option<&Notice> {
        self.notice.as_ref()
    }

    /// Whether the server answered its last health check or sign-in.
    pub fn reachable(&self) -> Option<bool> {
        self.reachable
    }

    /// The email to fill the form with.
    pub fn email(&self) -> Option<&str> {
        self.remembered.email.as_deref()
    }

    /// The WebSocket since the last sign-in; `None` before one.
    pub fn connection(&self) -> Option<&ConnectionState> {
        self.connection.as_ref()
    }

    /// The form's Sign in. Only while signed out; otherwise nothing happens.
    pub fn log_in(&mut self, email: &str, password: &str) -> Vec<Effect> {
        if self.phase != Phase::SignedOut {
            return Vec::new();
        }
        let email = email.trim();
        let checked = check_login(email, password);
        if let Err(why) = checked {
            self.notice = Some(Notice::Invalid(why));
            return Vec::new();
        }
        let (sign_in, request) = SignIn::log_in(email, password, self.tag);
        self.begin(sign_in, request)
    }

    /// The form's Create account. Only while signed out.
    pub fn register(&mut self, name: &str, email: &str, password: &str) -> Vec<Effect> {
        if self.phase != Phase::SignedOut {
            return Vec::new();
        }
        let name = name.trim();
        let email = email.trim();
        let checked = check_register(name, email, password);
        if let Err(why) = checked {
            self.notice = Some(Notice::Invalid(why));
            return Vec::new();
        }
        let account = Account {
            email: email.to_owned(),
            password: password.to_owned(),
            name: name.to_owned(),
        };
        let (sign_in, request) = SignIn::register(account, self.tag);
        self.begin(sign_in, request)
    }

    fn begin(&mut self, sign_in: SignIn, request: RestRequest) -> Vec<Effect> {
        self.phase = Phase::SigningIn {
            registering: sign_in.is_registering(),
        };
        self.sign_in = Some(sign_in);
        self.notice = None;
        vec![Effect::Rest(request, self.tag)]
    }

    /// Leaves the world, or gives up entering it: tells the server, closes
    /// the WebSocket and forgets the token. The email stays.
    pub fn log_out(&mut self) -> Vec<Effect> {
        let mut effects = Vec::new();
        if matches!(self.phase, Phase::SignedOut | Phase::SigningIn { .. }) {
            return effects;
        }
        if let Some(token) = self.remembered.token.take() {
            effects.push(Effect::Rest(
                RestRequest::Logout(LogoutRequest { token }),
                self.logout_tag(),
            ));
        }
        effects.push(Effect::Disconnect);
        effects.push(Effect::Remember(self.remembered.clone()));
        self.signed_out(None);
        effects
    }

    fn signed_out(&mut self, notice: Option<Notice>) {
        self.phase = Phase::SignedOut;
        self.sign_in = None;
        self.notice = notice;
        self.connection = None;
    }

    /// Feed every [`NetEvent`].
    pub fn observe(&mut self, event: &NetEvent) -> Vec<Effect> {
        let mut effects = Vec::new();
        match event {
            NetEvent::Rest(tag, result) if *tag == self.health_tag() => {
                self.health_in_flight = false;
                // Any answer, even an error status, means it's up.
                self.reachable = Some(!matches!(result, Err(RestError::Transport(_))));
            }
            NetEvent::Rest(tag, _) if *tag == self.tag => self.observe_sign_in(event, &mut effects),
            NetEvent::Connection(state) => self.observe_connection(state, &mut effects),
            _ => {}
        }
        effects
    }

    fn observe_sign_in(&mut self, event: &NetEvent, effects: &mut Vec<Effect>) {
        let Some(sign_in) = &mut self.sign_in else {
            return;
        };
        match sign_in.observe(event) {
            Some(Step::Send(request)) => effects.push(Effect::Rest(request, self.tag)),
            Some(Step::SignedIn(auth)) => {
                self.remembered = Remembered {
                    email: Some(sign_in.email().to_owned()),
                    token: Some(auth.token.clone()),
                };
                self.sign_in = None;
                self.reachable = Some(true);
                self.phase = Phase::Entering { resumed: false };
                effects.push(Effect::Remember(self.remembered.clone()));
                effects.push(Effect::Connect(auth.token));
            }
            Some(Step::Failed(failure)) => {
                tracing::info!("auth: {failure}");
                let notice = Notice::from(&failure);
                self.reachable = Some(notice != Notice::Unreachable);
                self.signed_out(Some(notice));
            }
            None => {}
        }
    }

    fn observe_connection(&mut self, state: &ConnectionState, effects: &mut Vec<Effect>) {
        if !matches!(self.phase, Phase::Entering { .. } | Phase::InWorld) {
            // A late echo of a session already left.
            return;
        }
        self.connection = Some(state.clone());
        match state {
            ConnectionState::Connected { .. } => {
                self.phase = Phase::InWorld;
                self.reachable = Some(true);
            }
            ConnectionState::AuthFailed { message } => {
                tracing::info!("auth: the server ended the session: {message}");
                self.remembered.token = None;
                effects.push(Effect::Remember(self.remembered.clone()));
                self.signed_out(Some(Notice::SessionEnded {
                    elsewhere: message == REPLACED,
                }));
            }
            _ => {}
        }
    }

    /// Call once per frame: the health check, while signed out or entering.
    pub fn tick(&mut self, now: Instant) -> Vec<Effect> {
        let wanted = self.phase != Phase::InWorld;
        let due = self.health_due.is_none_or(|at| now >= at);
        if !wanted || self.health_in_flight || !due {
            return Vec::new();
        }
        self.health_in_flight = true;
        self.health_due = Some(now + HEALTH_EVERY);
        vec![Effect::Rest(RestRequest::Health, self.health_tag())]
    }
}

/// The Sign in form's checks, before anything is sent.
fn check_login(email: &str, password: &str) -> Result<(), &'static str> {
    check_email(email)?;
    if password.is_empty() {
        return Err("Enter your password.");
    }
    Ok(())
}

/// The Create account form's checks, React's rules.
fn check_register(name: &str, email: &str, password: &str) -> Result<(), &'static str> {
    if !NAME_CHARS.contains(&name.chars().count()) {
        return Err("A player name is 2 to 32 characters.");
    }
    check_email(email)?;
    if password.chars().count() < PASSWORD_MIN {
        return Err("A password is at least 8 characters.");
    }
    Ok(())
}

fn check_email(email: &str) -> Result<(), &'static str> {
    if email.is_empty() {
        Err("Enter your email.")
    } else if !email.contains('@') {
        Err("That doesn't look like an email address.")
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::RestResponse;
    use capitol_protocol::rest::{AccountInfo, AuthResponse, HealthResponse};
    use uuid::Uuid;

    const TAG: Tag = 10;

    fn auth_ok(token: &str) -> NetEvent {
        NetEvent::Rest(
            TAG,
            Ok(RestResponse::Auth(AuthResponse {
                token: token.into(),
                account: AccountInfo {
                    id: Uuid::nil(),
                    email: "ada@capitol.local".into(),
                    tier: "free".into(),
                    is_admin: false,
                },
                player_id: Uuid::nil(),
            })),
        )
    }

    fn http(tag: Tag, status: u16, message: &str) -> NetEvent {
        NetEvent::Rest(
            tag,
            Err(RestError::Http {
                status,
                message: message.into(),
            }),
        )
    }

    fn refused(tag: Tag) -> NetEvent {
        NetEvent::Rest(tag, Err(RestError::Transport("connection refused".into())))
    }

    fn connected() -> NetEvent {
        NetEvent::Connection(ConnectionState::Connected {
            player_id: Uuid::nil(),
        })
    }

    fn auth_failed(message: &str) -> NetEvent {
        NetEvent::Connection(ConnectionState::AuthFailed {
            message: message.into(),
        })
    }

    fn signed_out() -> Auth {
        let (auth, effects) = Auth::new(Remembered::default(), TAG);
        assert!(effects.is_empty());
        assert_eq!(auth.phase(), Phase::SignedOut);
        auth
    }

    #[test]
    fn a_login_enters_the_world_and_remembers_the_email() {
        let mut auth = signed_out();
        let effects = auth.log_in("  ada@capitol.local ", "hunter22");
        assert!(
            matches!(&effects[..], [Effect::Rest(RestRequest::Login(l), TAG)]
            if l.email == "ada@capitol.local")
        );
        assert_eq!(auth.phase(), Phase::SigningIn { registering: false });

        let effects = auth.observe(&auth_ok("t0ken"));
        let remembered = Remembered {
            email: Some("ada@capitol.local".into()),
            token: Some("t0ken".into()),
        };
        assert_eq!(
            effects,
            vec![
                Effect::Remember(remembered),
                Effect::Connect("t0ken".into())
            ]
        );
        assert_eq!(auth.phase(), Phase::Entering { resumed: false });

        auth.observe(&connected());
        assert_eq!(auth.phase(), Phase::InWorld);
        assert_eq!(auth.email(), Some("ada@capitol.local"));
    }

    #[test]
    fn a_wrong_password_is_a_notice_not_a_new_account() {
        let mut auth = signed_out();
        auth.log_in("ada@capitol.local", "hunter2");
        let effects = auth.observe(&http(TAG, 401, "Invalid credentials"));
        assert!(effects.is_empty(), "no register after a 401: {effects:?}");
        assert_eq!(auth.phase(), Phase::SignedOut);
        assert_eq!(auth.notice(), Some(&Notice::WrongPassword));
        assert_eq!(auth.reachable(), Some(true));
        assert_eq!(auth.email(), None, "a failed login isn't remembered");
    }

    #[test]
    fn registering_a_taken_email_says_so() {
        let mut auth = signed_out();
        let effects = auth.register("Ada", "ada@capitol.local", "hunter22");
        assert!(
            matches!(&effects[..], [Effect::Rest(RestRequest::Register(r), TAG)]
            if r.player_name == "Ada")
        );
        assert_eq!(auth.phase(), Phase::SigningIn { registering: true });
        auth.observe(&http(TAG, 409, "Email already registered"));
        assert_eq!(auth.notice(), Some(&Notice::EmailTaken));
    }

    #[test]
    fn the_form_checks_before_sending() {
        let mut auth = signed_out();
        assert!(auth.log_in("", "x").is_empty());
        assert_eq!(auth.notice(), Some(&Notice::Invalid("Enter your email.")));
        assert!(auth.log_in("ada", "x").is_empty());
        assert!(auth.log_in("ada@capitol.local", "").is_empty());
        assert!(auth
            .register("A", "ada@capitol.local", "hunter22")
            .is_empty());
        assert!(auth
            .register("Ada", "ada@capitol.local", "short")
            .is_empty());
        assert_eq!(auth.phase(), Phase::SignedOut);
        // A good submit clears the last notice.
        auth.register("Ada", "ada@capitol.local", "hunter22");
        assert_eq!(auth.notice(), None);
    }

    #[test]
    fn a_stored_token_is_tried_first() {
        let remembered = Remembered {
            email: Some("ada@capitol.local".into()),
            token: Some("old".into()),
        };
        let (mut auth, effects) = Auth::new(remembered, TAG);
        assert_eq!(effects, vec![Effect::Connect("old".into())]);
        assert_eq!(auth.phase(), Phase::Entering { resumed: true });
        auth.observe(&connected());
        assert_eq!(auth.phase(), Phase::InWorld);
    }

    #[test]
    fn a_rejected_token_returns_to_the_form_and_is_forgotten() {
        let remembered = Remembered {
            email: Some("ada@capitol.local".into()),
            token: Some("old".into()),
        };
        let (mut auth, _) = Auth::new(remembered.clone(), TAG);
        let effects = auth.observe(&auth_failed("Invalid token"));
        assert_eq!(
            effects,
            vec![Effect::Remember(Remembered {
                email: remembered.email.clone(),
                token: None
            })]
        );
        assert_eq!(auth.phase(), Phase::SignedOut);
        assert_eq!(
            auth.notice(),
            Some(&Notice::SessionEnded { elsewhere: false })
        );

        // In the world, another login takes the account.
        let (mut auth, _) = Auth::new(remembered, TAG);
        auth.observe(&connected());
        auth.observe(&auth_failed(REPLACED));
        assert_eq!(
            auth.notice(),
            Some(&Notice::SessionEnded { elsewhere: true })
        );
    }

    #[test]
    fn a_dropped_connection_stays_in_the_world() {
        let (mut auth, _) = Auth::new(
            Remembered {
                email: None,
                token: Some("t".into()),
            },
            TAG,
        );
        auth.observe(&connected());
        auth.observe(&NetEvent::Connection(ConnectionState::Reconnecting {
            attempt: 1,
            delay: Duration::from_millis(500),
            reason: "reset".into(),
        }));
        assert_eq!(auth.phase(), Phase::InWorld);
        assert!(matches!(
            auth.connection(),
            Some(ConnectionState::Reconnecting { .. })
        ));
    }

    #[test]
    fn logging_out_tells_the_server_and_keeps_the_email() {
        let mut auth = signed_out();
        auth.log_in("ada@capitol.local", "hunter22");
        auth.observe(&auth_ok("t0ken"));
        auth.observe(&connected());
        let effects = auth.log_out();
        assert_eq!(
            effects,
            vec![
                Effect::Rest(
                    RestRequest::Logout(LogoutRequest {
                        token: "t0ken".into()
                    }),
                    TAG + 2
                ),
                Effect::Disconnect,
                Effect::Remember(Remembered {
                    email: Some("ada@capitol.local".into()),
                    token: None
                }),
            ]
        );
        assert_eq!(auth.phase(), Phase::SignedOut);
        assert_eq!(auth.notice(), None);
        // The session's last words don't bring it back.
        auth.observe(&connected());
        auth.observe(&NetEvent::Connection(ConnectionState::Disconnected {
            reason: "closed".into(),
        }));
        assert_eq!(auth.phase(), Phase::SignedOut);
        assert_eq!(auth.connection(), None);
    }

    #[test]
    fn an_unreachable_server_is_known_before_and_after_submitting() {
        let mut auth = signed_out();
        let t0 = Instant::now();
        let effects = auth.tick(t0);
        assert_eq!(effects, vec![Effect::Rest(RestRequest::Health, TAG + 1)]);
        assert!(auth.tick(t0 + HEALTH_EVERY * 2).is_empty(), "one in flight");
        auth.observe(&refused(TAG + 1));
        assert_eq!(auth.reachable(), Some(false));
        assert!(auth.tick(t0 + HEALTH_EVERY / 2).is_empty(), "not due yet");
        assert_eq!(auth.tick(t0 + HEALTH_EVERY).len(), 1);
        let health = HealthResponse {
            status: "healthy".into(),
            database: "ok".into(),
            redis: "ok".into(),
        };
        auth.observe(&NetEvent::Rest(TAG + 1, Ok(RestResponse::Health(health))));
        assert_eq!(auth.reachable(), Some(true));

        auth.log_in("ada@capitol.local", "hunter22");
        auth.observe(&refused(TAG));
        assert_eq!(auth.notice(), Some(&Notice::Unreachable));
        assert_eq!(auth.reachable(), Some(false));
    }

    #[test]
    fn no_health_checks_in_the_world() {
        let mut auth = signed_out();
        auth.log_in("ada@capitol.local", "hunter22");
        auth.observe(&auth_ok("t"));
        auth.observe(&connected());
        assert!(auth.tick(Instant::now()).is_empty());
    }
}
