//! Content-addressed storage — IPFS / Bitswap の最小版。
//!
//! スコープ:
//! - CID v1 (raw multihash: blake3-256) の生成 / 文字列化
//! - `ContentStore` trait: block (bytes) を CID でキー引く store
//! - `MemoryContentStore` / `FileContentStore` の 2 実装
//! - ファイル → root-CID + chunk-CID リスト (Merkle DAG 最小版) の分割 / 再組立
//! - Bitswap-lite: `BitswapRequest::Want(cid)` → `BitswapResponse::Block/NotFound`
//!   を QUIC bidi ストリーム (magic `BSW1`) でやりとりするクライアント / サーバ
//!
//! 本実装の `Cid` は `synergos_net::types::Cid(String)` を使い、内部的に
//! `blake3-<hex>` の形を採用する (完全な multiformats とは互換ではないが、
//! 最小仕様で blake3 固定)。上位層が CID を見せる場面では透過的に文字列化する。

mod bitswap;
mod block;
mod chunker;
mod session;
mod store;

pub use bitswap::{
    handle_bitswap_stream, request_block, request_many, send_cancel, BitswapRequest,
    BitswapResponse, BITSWAP_STREAM_MAGIC, MAX_BITSWAP_FRAME, MAX_WANTLIST_LEN,
};
pub use block::{cid_for, Block};
pub use chunker::{add_file, get_file, ChunkDag, ChunkerOptions};
pub use session::{BitswapSession, FetchOutcome, SESSION_WANTLIST_CHUNK};
pub use store::{ContentStore, FileContentStore, MemoryContentStore};
