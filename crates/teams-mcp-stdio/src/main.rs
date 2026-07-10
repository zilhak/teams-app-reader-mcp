//! Teams 읽기 전용 MCP — stdio 전송 바이너리.
//! 같은 `TeamsServer` 핸들러를 stdio 전송에 연결한다.

use rmcp::{transport::stdio, ServiceExt};
use teams_mcp_server::TeamsServer;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let server = TeamsServer::new()?;
    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
