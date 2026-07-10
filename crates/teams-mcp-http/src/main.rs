//! Teams 읽기 전용 MCP — Streamable HTTP 전송 바이너리.
//! 같은 `TeamsServer` 핸들러를 rmcp 의 Streamable HTTP 서비스에 연결하고 axum 으로 서빙한다.
//!
//! ⚠️ 남의 Teams 메시지(민감 데이터)를 네트워크로 노출하므로 **기본 localhost 바인드**.
//!   - 주소: 환경변수 `TEAMS_MCP_HTTP_ADDR` (기본 127.0.0.1:8787)
//!   - 인증: `TEAMS_MCP_TOKEN` 설정 시 `Authorization: Bearer <token>` 필수. 미설정 시
//!     경고 로그와 함께 무인증(로컬 전용 가정)으로 뜬다.

use std::sync::Arc;

use axum::{
    extract::Request,
    http::{header::AUTHORIZATION, StatusCode},
    middleware::{from_fn_with_state, Next},
    response::Response,
};
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use teams_mcp_server::TeamsServer;

const MCP_PATH: &str = "/mcp";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().init();

    let addr = std::env::var("TEAMS_MCP_HTTP_ADDR").unwrap_or_else(|_| "127.0.0.1:8787".into());
    let token = std::env::var("TEAMS_MCP_TOKEN").ok().filter(|t| !t.is_empty());

    let service: StreamableHttpService<TeamsServer, LocalSessionManager> =
        StreamableHttpService::new(
            || {
                TeamsServer::new()
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))
            },
            Arc::new(LocalSessionManager::default()),
            StreamableHttpServerConfig::default(),
        );

    let mut app = axum::Router::new().route_service(MCP_PATH, service);
    if let Some(tok) = token {
        app = app.layer(from_fn_with_state(Arc::new(tok), auth));
        tracing::info!("Bearer 토큰 인증 활성화");
    } else {
        tracing::warn!(
            "TEAMS_MCP_TOKEN 미설정 — 무인증으로 뜬다. 반드시 localhost 에서만 사용할 것."
        );
    }

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("Teams MCP (HTTP) 리스닝: http://{addr}{MCP_PATH}");
    axum::serve(listener, app).await?;
    Ok(())
}

/// `Authorization: Bearer <token>` 검증 미들웨어.
async fn auth(
    axum::extract::State(expected): axum::extract::State<Arc<String>>,
    req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let ok = req
        .headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|t| t == expected.as_str())
        .unwrap_or(false);
    if ok {
        Ok(next.run(req).await)
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}
