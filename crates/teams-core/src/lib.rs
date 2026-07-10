//! teams-core: Teams v2 로컬 IndexedDB 를 읽어 대화/메시지를 뽑아내는 로직.
//! transport(MCP/stdio/http) 와 무관한 순수 데이터 레이어.

pub mod idb;
pub mod leveldb;
pub mod location;
pub mod teams;
pub mod util;
pub mod v8;
pub mod varint;

pub use teams::{Chat, Message, StoreError, TeamsStore};
