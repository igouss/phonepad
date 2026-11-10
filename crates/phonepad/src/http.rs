use std::sync::Arc;

use axum::Router;
use axum::extract::ws::WebSocketUpgrade;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;

use crate::session::{self, SessionPorts};

const LOGIN_HEADER: &str = "tailscale-user-login";

struct Asset {
    path: &'static str,
    content_type: &'static str,
    body: &'static [u8],
}

const ASSETS: [Asset; 4] = [
    Asset {
        path: "/",
        content_type: "text/html; charset=utf-8",
        body: include_bytes!("../../../pad.html"),
    },
    Asset {
        path: "/manifest.webmanifest",
        content_type: "application/manifest+json",
        body: include_bytes!("../../../manifest.webmanifest"),
    },
    Asset {
        path: "/icon-192.png",
        content_type: "image/png",
        body: include_bytes!("../../../icon-192.png"),
    },
    Asset {
        path: "/icon-512.png",
        content_type: "image/png",
        body: include_bytes!("../../../icon-512.png"),
    },
];

pub fn router(login: Arc<str>, ports: SessionPorts) -> Router {
    ASSETS
        .iter()
        .fold(
            Router::new(),
            |router: Router<SessionPorts>, asset: &Asset| {
                let content_type: &'static str = asset.content_type;
                let body: &'static [u8] = asset.body;
                router.route(
                    asset.path,
                    get(move || async move {
                        (
                            [(CONTENT_TYPE, content_type), (CACHE_CONTROL, "no-store")],
                            body,
                        )
                    }),
                )
            },
        )
        .route("/ws", get(websocket))
        .fallback(|| async { StatusCode::NOT_FOUND })
        .layer(middleware::from_fn_with_state(login, require_login))
        .with_state(ports)
}

async fn require_login(State(login): State<Arc<str>>, request: Request, next: Next) -> Response {
    let allowed: bool = request
        .headers()
        .get(LOGIN_HEADER)
        .is_some_and(|value| value.as_bytes() == login.as_bytes());
    if allowed {
        next.run(request).await
    } else {
        StatusCode::FORBIDDEN.into_response()
    }
}

async fn websocket(State(ports): State<SessionPorts>, upgrade: WebSocketUpgrade) -> Response {
    upgrade.on_upgrade(move |socket| session::run(socket, ports))
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::HeaderValue;
    use phonepad_core::input::InputCommand;
    use tokio::sync::{broadcast, mpsc, watch};
    use tower::ServiceExt;

    use super::*;
    use crate::capture::{KeyframeRequest, Unit};
    use crate::clock::Clock;
    use crate::desk::DeskMessage;

    const LOGIN: &str = "owner@example.com";

    struct Reply {
        status: StatusCode,
        content_type: Option<HeaderValue>,
        cache_control: Option<HeaderValue>,
    }

    async fn get_path(path: &str, login: Option<&str>) -> Reply {
        let (desk, _): (mpsc::Sender<DeskMessage>, _) = mpsc::channel(1);
        let (input, _): (mpsc::Sender<InputCommand>, _) = mpsc::channel(1);
        let (view, _) = watch::channel(None);
        let (units, _): (broadcast::Sender<Unit>, broadcast::Receiver<Unit>) =
            broadcast::channel(1);
        let ports: SessionPorts = SessionPorts {
            desk,
            input,
            units,
            keyframe: KeyframeRequest::default(),
            view,
            cursor: watch::Sender::new(None),
            clock: Clock::start(),
        };
        let request: Request = login
            .into_iter()
            .fold(Request::builder().uri(path), |builder, login: &str| {
                builder.header(LOGIN_HEADER, login)
            })
            .body(Body::empty())
            .unwrap();
        let response: Response = router(Arc::from(LOGIN), ports)
            .oneshot(request)
            .await
            .unwrap();
        Reply {
            status: response.status(),
            content_type: response.headers().get(CONTENT_TYPE).cloned(),
            cache_control: response.headers().get(CACHE_CONTROL).cloned(),
        }
    }

    async fn served(path: &str) -> (StatusCode, Option<HeaderValue>, Option<HeaderValue>) {
        let reply: Reply = get_path(path, Some(LOGIN)).await;
        (reply.status, reply.content_type, reply.cache_control)
    }

    fn ok(content_type: &'static str) -> (StatusCode, Option<HeaderValue>, Option<HeaderValue>) {
        (
            StatusCode::OK,
            Some(HeaderValue::from_static(content_type)),
            Some(HeaderValue::from_static("no-store")),
        )
    }

    #[tokio::test]
    async fn page_without_login_is_forbidden() {
        assert_eq!(get_path("/", None).await.status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn page_with_wrong_login_is_forbidden() {
        assert_eq!(
            get_path("/", Some("intruder@example.com")).await.status,
            StatusCode::FORBIDDEN
        );
    }

    #[tokio::test]
    async fn manifest_without_login_is_forbidden() {
        assert_eq!(
            get_path("/manifest.webmanifest", None).await.status,
            StatusCode::FORBIDDEN
        );
    }

    #[tokio::test]
    async fn manifest_with_wrong_login_is_forbidden() {
        assert_eq!(
            get_path("/manifest.webmanifest", Some("intruder@example.com"))
                .await
                .status,
            StatusCode::FORBIDDEN
        );
    }

    #[tokio::test]
    async fn small_icon_without_login_is_forbidden() {
        assert_eq!(
            get_path("/icon-192.png", None).await.status,
            StatusCode::FORBIDDEN
        );
    }

    #[tokio::test]
    async fn small_icon_with_wrong_login_is_forbidden() {
        assert_eq!(
            get_path("/icon-192.png", Some("intruder@example.com"))
                .await
                .status,
            StatusCode::FORBIDDEN
        );
    }

    #[tokio::test]
    async fn large_icon_without_login_is_forbidden() {
        assert_eq!(
            get_path("/icon-512.png", None).await.status,
            StatusCode::FORBIDDEN
        );
    }

    #[tokio::test]
    async fn large_icon_with_wrong_login_is_forbidden() {
        assert_eq!(
            get_path("/icon-512.png", Some("intruder@example.com"))
                .await
                .status,
            StatusCode::FORBIDDEN
        );
    }

    #[tokio::test]
    async fn websocket_without_login_is_forbidden() {
        assert_eq!(get_path("/ws", None).await.status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn websocket_with_wrong_login_is_forbidden() {
        assert_eq!(
            get_path("/ws", Some("intruder@example.com")).await.status,
            StatusCode::FORBIDDEN
        );
    }

    #[tokio::test]
    async fn unknown_path_without_login_is_forbidden() {
        assert_eq!(
            get_path("/etc/passwd", None).await.status,
            StatusCode::FORBIDDEN
        );
    }

    #[tokio::test]
    async fn unknown_path_with_wrong_login_is_forbidden() {
        assert_eq!(
            get_path("/etc/passwd", Some("intruder@example.com"))
                .await
                .status,
            StatusCode::FORBIDDEN
        );
    }

    #[tokio::test]
    async fn login_prefix_is_not_the_login() {
        assert_eq!(
            get_path("/", Some("owner@example.co")).await.status,
            StatusCode::FORBIDDEN
        );
    }

    #[tokio::test]
    async fn page_is_html() {
        assert_eq!(served("/").await, ok("text/html; charset=utf-8"));
    }

    #[tokio::test]
    async fn manifest_is_a_web_manifest() {
        assert_eq!(
            served("/manifest.webmanifest").await,
            ok("application/manifest+json")
        );
    }

    #[tokio::test]
    async fn small_icon_is_png() {
        assert_eq!(served("/icon-192.png").await, ok("image/png"));
    }

    #[tokio::test]
    async fn large_icon_is_png() {
        assert_eq!(served("/icon-512.png").await, ok("image/png"));
    }

    #[tokio::test]
    async fn unknown_path_with_login_is_not_found() {
        assert_eq!(
            get_path("/etc/passwd", Some(LOGIN)).await.status,
            StatusCode::NOT_FOUND
        );
    }
}
