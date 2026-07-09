//! teams-mcp: macOS 접근성 API로 실행 중인 Microsoft Teams 데스크톱 앱을
//! 읽기 전용으로 노출하는 MCP 서버. 쓰기/전송 기능은 의도적으로 제공하지 않는다.

use rmcp::{
    handler::server::router::tool::ToolRouter,
    model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
    transport::stdio,
    ErrorData as McpError, ServerHandler, ServiceExt,
};

mod ax;

#[derive(Clone)]
struct TeamsServer {
    // tool_handler 매크로가 내부적으로 참조하는 필드 (직접 읽기 코드는 없어 dead_code 경고가 뜬다)
    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl TeamsServer {
    fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        description = "현재 열려 있는 Teams 창의 제목을 반환한다. 어느 채팅방/화면이 열려 있는지 확인용."
    )]
    async fn active_view(&self) -> Result<CallToolResult, McpError> {
        match ax::window_title() {
            Ok(t) => Ok(CallToolResult::success(vec![ContentBlock::text(t)])),
            Err(e) => Err(McpError::internal_error(e.to_string(), None)),
        }
    }

    #[tool(
        description = "현재 Teams 창에 보이는 모든 텍스트(메시지 본문·발신자·시간, 좌측 채팅 목록 포함)를 위에서 아래 순서로 반환한다. 읽기 전용."
    )]
    async fn read_messages(&self) -> Result<CallToolResult, McpError> {
        match ax::read_static_texts() {
            Ok(t) => Ok(CallToolResult::success(vec![ContentBlock::text(t)])),
            Err(e) => Err(McpError::internal_error(e.to_string(), None)),
        }
    }
}

#[tool_handler]
impl ServerHandler for TeamsServer {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        info.instructions = Some(
            "macOS 접근성 API로 실행 중인 Microsoft Teams 데스크톱 앱을 읽는 읽기 전용 서버. \
             메시지 전송·수정 등 쓰기 기능은 제공하지 않는다. \
             Teams 앱이 실행 중이어야 하며, 이 서버 프로세스에 손쉬운 사용(Accessibility) 권한이 필요하다."
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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let service = TeamsServer::new().serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
