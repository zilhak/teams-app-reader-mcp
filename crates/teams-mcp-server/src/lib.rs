//! Teams MCP 서버 핸들러. 전송(stdio/http)과 무관하게 `TeamsServer` 를 노출하고,
//! stdio/http 바이너리가 각자 전송에 연결한다.
//!
//! Teams 데이터는 **읽기만** 한다 (메시지 전송·수정 도구 없음). 전송 대신,
//! 사용자가 Teams 입력창에 붙여넣을 수 있는 형태로 클립보드에 올려주는
//! `copy_to_clipboard` 까지를 지원한다.

use std::sync::Arc;

use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler,
};
use base64::Engine;
use schemars::JsonSchema;
use serde::Deserialize;
use teams_core::TeamsStore;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReadMessagesArgs {
    /// 대상 대화. conversationId(예 "19:...@thread.v2") 정확일치 또는 대화명(topic) 부분일치.
    pub chat: String,
    /// 반환할 최근 메시지 수 (기본 30).
    pub limit: Option<usize>,
    /// 이 epoch(ms) 이전 메시지만 (과거 페이지네이션용, 선택).
    pub before_ms: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchArgs {
    /// 검색어 (content/발신자명 대상, 대소문자 무시).
    pub query: String,
    /// 반환할 최근 결과 수 (기본 20).
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FetchImageArgs {
    /// read_messages 의 images[].url (AMS 이미지 URL, `*.asyncgw.teams.microsoft.com`).
    pub url: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CopyToClipboardArgs {
    /// 클립보드에 올릴 HTML 조각. Teams 입력창에서 동작 확인된 태그는 문단 `<p>`,
    /// 강조 `<strong>`, 불릿 `<ul><li>…</li></ul>`(중첩 가능), 링크
    /// `<a href="URL">텍스트</a>`. `<html>`/`<body>` 로 감쌀 필요 없다 (자동으로 감싼다).
    pub html: String,
}

#[derive(Clone)]
pub struct TeamsServer {
    store: Arc<TeamsStore>,
    // tool_handler 매크로가 내부적으로 참조하는 필드 (직접 읽는 코드는 없음)
    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl TeamsServer {
    pub fn new() -> Result<Self, teams_core::StoreError> {
        Ok(Self {
            store: Arc::new(TeamsStore::open(None)?),
            tool_router: Self::tool_router(),
        })
    }

    #[tool(
        description = "로컬에 캐시된 Teams 대화 목록을 최근 활동순으로 반환한다. 각 항목: 대화명(topic), conversationId, 캐시된 메시지 수, 마지막 메시지 시각/발신자. 읽기 전용."
    )]
    async fn list_chats(&self) -> Result<CallToolResult, McpError> {
        let store = self.store.clone();
        let chats = tokio::task::spawn_blocking(move || store.list_chats())
            .await
            .map_err(join_err)?
            .map_err(store_err)?;
        Ok(json_result(&chats))
    }

    #[tool(
        description = "특정 Teams 대화의 캐시된 메시지를 시간 오름차순으로 반환한다. chat 은 conversationId 정확일치 또는 대화명 부분일치. 각 메시지: 발신자, 본문(HTML은 평문화), 시각(UTC), 메시지타입. 이미지가 있으면 images[]{url,width,height} 로 함께 반환하며, 실제 이미지는 그 url 을 fetch_image 에 넘겨 조회한다. 읽기 전용."
    )]
    async fn read_messages(
        &self,
        Parameters(args): Parameters<ReadMessagesArgs>,
    ) -> Result<CallToolResult, McpError> {
        let store = self.store.clone();
        let limit = args.limit.unwrap_or(30);
        let msgs = tokio::task::spawn_blocking(move || {
            store.read_messages(&args.chat, limit, args.before_ms)
        })
        .await
        .map_err(join_err)?
        .map_err(store_err)?;
        Ok(json_result(&msgs))
    }

    #[tool(
        description = "캐시된 모든 Teams 메시지에서 키워드로 검색한다(대소문자 무시, 본문/발신자명 대상). 최근순 결과를 반환. 읽기 전용."
    )]
    async fn search_messages(
        &self,
        Parameters(args): Parameters<SearchArgs>,
    ) -> Result<CallToolResult, McpError> {
        let store = self.store.clone();
        let limit = args.limit.unwrap_or(20);
        let msgs = tokio::task::spawn_blocking(move || store.search(&args.query, limit))
            .await
            .map_err(join_err)?
            .map_err(store_err)?;
        Ok(json_result(&msgs))
    }

    #[tool(
        description = "read_messages 의 images[].url 로 Teams 이미지(스크린샷 등)를 실제로 가져와 이미지로 반환한다. 로컬 Teams 쿠키를 복호화해 AMS CDN 에 인증 요청하므로, 다른 도구와 달리 네트워크를 사용한다. `*.asyncgw.teams.microsoft.com` URL 만 허용."
    )]
    async fn fetch_image(
        &self,
        Parameters(args): Parameters<FetchImageArgs>,
    ) -> Result<CallToolResult, McpError> {
        let (bytes, mime) =
            tokio::task::spawn_blocking(move || teams_core::fetch_ams_image(&args.url))
                .await
                .map_err(join_err)?
                .map_err(media_err)?;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
        Ok(CallToolResult::success(vec![ContentBlock::image(b64, mime)]))
    }

    #[tool(
        description = "Teams 입력창에 붙여넣을 리치 텍스트를 시스템 클립보드에 올린다. 이 서버는 메시지를 전송하지 않는 대신 여기까지를 지원한다 — 사용자가 Teams 창에서 붙여넣기(Cmd+V / Ctrl+V)하면 링크·불릿·강조 서식이 살아있는 상태로 입력된다. 여러 줄 업무보고나 티켓 링크가 섞인 메시지를 만들 때 쓴다. 주의: 클립보드는 사용자의 전역 상태이고 기존 복사 내용을 덮어쓰므로, 사용자가 클립보드에 넣어달라고 명시적으로 요청했을 때만 호출한다."
    )]
    async fn copy_to_clipboard(
        &self,
        Parameters(args): Parameters<CopyToClipboardArgs>,
    ) -> Result<CallToolResult, McpError> {
        tokio::task::spawn_blocking(move || teams_core::copy_html(&args.html))
            .await
            .map_err(join_err)?
            .map_err(clipboard_err)?;
        Ok(CallToolResult::success(vec![ContentBlock::text(
            "클립보드에 복사했다. Teams 입력창에서 붙여넣기(Cmd+V / Ctrl+V)하면 서식이 적용된다.",
        )]))
    }
}

#[tool_handler]
impl ServerHandler for TeamsServer {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        info.instructions = Some(
            "Microsoft Teams(v2) 가 로컬 IndexedDB 에 캐싱해둔 대화/메시지를 읽는 서버. \
             Teams 앱이 캐싱한 수개월치 히스토리를 조회할 수 있다. \
             Teams 로 메시지를 전송·수정하는 기능은 제공하지 않는다 — 대신 사용자가 Teams \
             입력창에 직접 붙여넣을 수 있도록 리치 텍스트를 클립보드에 올려주는 \
             copy_to_clipboard 까지를 지원한다(서식·링크 유지). \
             list_chats·read_messages·search_messages 는 로컬 캐시만 읽어 네트워크를 쓰지 않는다. \
             예외로 fetch_image 만, read_messages 의 images[].url 이미지를 실제로 받아오기 위해 \
             로컬 Teams 쿠키를 복호화해 AMS CDN 에 인증 요청한다."
                .into(),
        );
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        let mut impl_info = Implementation::default();
        impl_info.name = "teams-mcp".into();
        impl_info.version = env!("CARGO_PKG_VERSION").into();
        info.server_info = impl_info;
        info
    }
}

fn json_result<T: serde::Serialize>(value: &T) -> CallToolResult {
    let text =
        serde_json::to_string_pretty(value).unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"));
    CallToolResult::success(vec![ContentBlock::text(text)])
}

fn join_err(e: tokio::task::JoinError) -> McpError {
    McpError::internal_error(format!("작업 스레드 오류: {e}"), None)
}

fn store_err(e: teams_core::StoreError) -> McpError {
    McpError::internal_error(e.to_string(), None)
}

fn media_err(e: teams_core::MediaError) -> McpError {
    McpError::internal_error(e.to_string(), None)
}

fn clipboard_err(e: teams_core::ClipboardError) -> McpError {
    McpError::internal_error(e.to_string(), None)
}
