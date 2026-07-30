//! teams-core: Teams v2 로컬 IndexedDB 를 읽어 대화/메시지를 뽑아내는 로직.
//! transport(MCP/stdio/http) 와 무관한 데이터 레이어.
//!
//! 데이터 조회는 순수 로컬 파일 읽기다. 외부에 손을 뻗는 모듈은 둘뿐이다 —
//! [`media`](media) 는 이미지 조회를 위해 네트워크를, [`clipboard`](clipboard) 는
//! 붙여넣기용 리치 텍스트를 올리기 위해 OS 클립보드를 쓴다.

pub mod clipboard;
pub mod idb;
pub mod leveldb;
pub mod location;
pub mod media;
pub mod teams;
pub mod util;
pub mod v8;
pub mod varint;

pub use clipboard::{copy_html, ClipboardError};
pub use media::{fetch_ams_image, MediaError};
pub use teams::{Chat, ImageRef, Message, StoreError, TeamsStore};
