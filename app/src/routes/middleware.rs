use crate::app::AppState;
use crate::session::{SessionToken, UserSession};
use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::IntoResponse;
use axum_extra::extract::cookie::{Cookie, SameSite};
use axum_extra::extract::CookieJar;
use std::sync::Arc;
use time::Duration;
use tokio::sync::RwLock;
use tracing::info;

const SESSION_TOKEN_KEY: &str = "session_token";

async fn create_new_session(state: &AppState, ttl: Duration) -> (SessionToken, UserSession) {
    let new_user_session = UserSession::new();
    let new_id = SessionToken::generate_session_id();
    state
        .session
        .save_session(&new_id, &new_user_session, ttl)
        .await
        .expect("Could not save session");

    (new_id, new_user_session)
}

pub async fn session_middleware(
    State(state): State<Arc<AppState>>,
    jar: CookieJar,
    mut req: Request,
    next: Next,
) -> impl IntoResponse {
    let ttl = Duration::days(state.settings.cache_settings.session_ttl_days as i64);

    let (session_token, is_new, user_session) = match jar.get(SESSION_TOKEN_KEY) {
        Some(cookie) => {
            let session_token_from_cookie = SessionToken::from(cookie.value());

            // Either get existing session or create a new one with new token
            let (token, user_session) = match state
                .session
                .get_and_refresh_session(&session_token_from_cookie, ttl)
                .await
            {
                Some(user_session) => (session_token_from_cookie.clone(), user_session),
                None => create_new_session(&state, ttl).await,
            };
            let is_new = token != session_token_from_cookie;

            (token, is_new, user_session)
        }
        None => {
            let (new_id, new_user_session) = create_new_session(&state, ttl.clone()).await;

            (new_id, true, new_user_session)
        }
    };

    let session_state_before_request = user_session;
    let user_session: Arc<RwLock<UserSession>> =
        Arc::new(RwLock::new(session_state_before_request.clone()));

    // 2. Inject the session ID into the request extensions
    // This allows downstream handlers to easily read it without reparsing cookies!
    req.extensions_mut().insert(Arc::clone(&user_session));

    // 3. Let the request continue down the chain to your handlers/routers
    let mut response = next.run(req).await;

    let read_session = user_session.read().await;
    if read_session != session_state_before_request {
        state
            .session
            .save_session(&session_token, &*read_session, ttl)
            .await
            .expect("Could not save session");
    }

    // 4. If it was a brand-new session, append the Set-Cookie header to the outbound response
    if is_new {
        let token = session_token.clone();
        let new_cookie = Cookie::build((SESSION_TOKEN_KEY, token.as_str()))
            .path("/")
            .http_only(true)
            .secure(true)
            .same_site(SameSite::Lax)
            .max_age(ttl)
            .build();

        // Add the cookie to the outbound response headers
        if let Ok(cookie_header) = new_cookie.to_string().parse() {
            response
                .headers_mut()
                .append(axum::http::header::SET_COOKIE, cookie_header);
        }
    }

    response
}
