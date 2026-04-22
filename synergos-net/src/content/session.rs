//! BitswapSession: クライアント側の多チャンク取得を束ねる状態付きハンドル。
//!
//! ストリームは「1 WantList = 1 bidi stream」で張り、ListLen 件の Block/NotFound
//! (または Have/DontHave) を受け取り、最後に close。
//!
//! セッションはこの往復を複数回繰り返して:
//!
//! - `fetch_blocks`: 欲しい CID のうち、ローカルに無いものを WantList で一括取得し
//!   ContentStore へ書き込む。戻り値に各 CID の成否を返す。
//! - `fetch_dag`: root CID が指す `ChunkDag` を辿り、すべての chunk を取得する。
//!   root 自体がローカルに無ければ先に取りに行く。
//! - `have_query`: 相手が各 CID を持っているかのメタ問い合わせ (Have/DontHave)。
//!
//! 並列度は `max_inflight_streams` で制御する。相手の `max_concurrent_streams`
//! を超えないよう呼び出し側で調整する。

use std::sync::Arc;

use futures::stream::{FuturesUnordered, StreamExt};

use crate::error::{Result, SynergosNetError};
use crate::quic::{QuicManager, StreamType};
use crate::types::{Cid, PeerId};

use super::bitswap::{request_many, BitswapResponse};
use super::block::Block;
use super::chunker::ChunkDag;
use super::store::ContentStore;

/// 1 ストリームあたりの WantList 上限 (protocol の MAX_WANTLIST_LEN と一致させる)。
pub const SESSION_WANTLIST_CHUNK: usize = 128;

/// 取得結果: 成功なら Block の bytes は store にも投入済み、
/// `NotFound` なら相手が持っていなかったことを示す。
#[derive(Debug, Clone)]
pub enum FetchOutcome {
    Fetched,
    NotFound,
}

/// Bitswap クライアントセッション。
pub struct BitswapSession<S>
where
    S: ContentStore + ?Sized,
{
    quic: Arc<QuicManager>,
    peer: PeerId,
    store: Arc<S>,
    /// fetch_blocks / fetch_dag で同時に張る bidi stream の上限。
    max_inflight_streams: usize,
}

impl<S> BitswapSession<S>
where
    S: ContentStore + ?Sized + 'static,
{
    pub fn new(quic: Arc<QuicManager>, peer: PeerId, store: Arc<S>) -> Self {
        Self {
            quic,
            peer,
            store,
            max_inflight_streams: 4,
        }
    }

    pub fn with_max_inflight(mut self, n: usize) -> Self {
        self.max_inflight_streams = n.max(1);
        self
    }

    pub fn peer(&self) -> &PeerId {
        &self.peer
    }

    /// 相手が各 CID を持っているかだけ問い合わせる (want_have=true)。
    /// 戻り値: CID → true(Have) / false(DontHave) の組。
    pub async fn have_query(&self, cids: &[Cid]) -> Result<Vec<(Cid, bool)>> {
        if cids.is_empty() {
            return Ok(Vec::new());
        }
        let mut results: Vec<(Cid, bool)> = Vec::with_capacity(cids.len());
        for batch in cids.chunks(SESSION_WANTLIST_CHUNK) {
            let (send, recv) = self
                .quic
                .open_stream(&self.peer, StreamType::Control)
                .await?;
            let resps = request_many(send, recv, batch, true).await?;
            if resps.len() != batch.len() {
                return Err(SynergosNetError::Serialization(format!(
                    "bitswap have_query: expected {} responses, got {}",
                    batch.len(),
                    resps.len()
                )));
            }
            for (cid, r) in batch.iter().zip(resps.into_iter()) {
                let has = matches!(r, BitswapResponse::Have { .. });
                results.push((cid.clone(), has));
            }
        }
        Ok(results)
    }

    /// CID 一覧のうち、ローカルに無いものだけを相手から取り寄せる。
    /// 取得できた Block は自動的に ContentStore へ put される。
    pub async fn fetch_blocks(&self, cids: &[Cid]) -> Result<Vec<(Cid, FetchOutcome)>> {
        // ローカルに無い CID だけに絞る
        let mut missing: Vec<Cid> = Vec::new();
        let mut outcomes: Vec<(Cid, FetchOutcome)> = Vec::with_capacity(cids.len());
        for cid in cids {
            if self.store.has(cid).await {
                outcomes.push((cid.clone(), FetchOutcome::Fetched));
            } else {
                missing.push(cid.clone());
            }
        }
        if missing.is_empty() {
            return Ok(outcomes);
        }

        // 並列度を max_inflight_streams で制限しつつ、WantList ごとに 1 stream 張る。
        let batches: Vec<Vec<Cid>> = missing
            .chunks(SESSION_WANTLIST_CHUNK)
            .map(|s| s.to_vec())
            .collect();

        let mut pending: FuturesUnordered<_> = FuturesUnordered::new();
        let mut next = 0usize;

        // 初期投入
        while next < batches.len() && pending.len() < self.max_inflight_streams {
            pending.push(self.fetch_batch(batches[next].clone()));
            next += 1;
        }
        while let Some(res) = pending.next().await {
            match res {
                Ok(partial) => outcomes.extend(partial),
                Err(e) => {
                    // 1 バッチ失敗したらエラー伝播 (session 自体を壊す)
                    return Err(e);
                }
            }
            if next < batches.len() {
                pending.push(self.fetch_batch(batches[next].clone()));
                next += 1;
            }
        }
        Ok(outcomes)
    }

    async fn fetch_batch(&self, cids: Vec<Cid>) -> Result<Vec<(Cid, FetchOutcome)>> {
        let (send, recv) = self
            .quic
            .open_stream(&self.peer, StreamType::Control)
            .await?;
        let resps = request_many(send, recv, &cids, false).await?;
        if resps.len() != cids.len() {
            return Err(SynergosNetError::Serialization(format!(
                "bitswap fetch_batch: expected {} responses, got {}",
                cids.len(),
                resps.len()
            )));
        }
        let mut out = Vec::with_capacity(cids.len());
        for (cid, r) in cids.into_iter().zip(resps.into_iter()) {
            match r {
                BitswapResponse::Block {
                    cid: got_cid,
                    bytes,
                } => {
                    if got_cid != cid {
                        return Err(SynergosNetError::Serialization(format!(
                            "bitswap response cid mismatch: want {} got {}",
                            cid.0, got_cid.0
                        )));
                    }
                    let block = Block {
                        cid: got_cid.clone(),
                        bytes,
                    };
                    if !block.verify() {
                        return Err(SynergosNetError::Transfer(format!(
                            "bitswap block failed verification: {}",
                            got_cid.0
                        )));
                    }
                    self.store.put(block).await?;
                    out.push((cid, FetchOutcome::Fetched));
                }
                BitswapResponse::NotFound { cid: got_cid } => {
                    if got_cid != cid {
                        return Err(SynergosNetError::Serialization(format!(
                            "bitswap NotFound cid mismatch: want {} got {}",
                            cid.0, got_cid.0
                        )));
                    }
                    out.push((cid, FetchOutcome::NotFound));
                }
                other => {
                    return Err(SynergosNetError::Serialization(format!(
                        "bitswap fetch_batch: unexpected response variant {other:?}"
                    )));
                }
            }
        }
        Ok(out)
    }

    /// root CID の指す DAG を辿り、すべての chunk をローカルに揃える。
    /// root と子 chunk は ContentStore へ書き込まれる。すべて揃った場合のみ Ok(dag)。
    pub async fn fetch_dag(&self, root: &Cid) -> Result<ChunkDag> {
        // 1. root block を取得 (既にあればスキップ)
        let _ = self.fetch_blocks(std::slice::from_ref(root)).await?;
        let root_block = self.store.get(root).await?.ok_or_else(|| {
            SynergosNetError::Transfer(format!("root cid unavailable: {}", root.0))
        })?;

        // 2. ChunkDag として decode
        let dag: ChunkDag = rmp_serde::from_slice(&root_block.bytes)
            .map_err(|e| SynergosNetError::Serialization(format!("dag decode: {e}")))?;

        // 3. 子 chunk をまとめて取りに行く
        let outcomes = self.fetch_blocks(&dag.chunks).await?;
        let missing: Vec<String> = outcomes
            .iter()
            .filter_map(|(cid, out)| match out {
                FetchOutcome::NotFound => Some(cid.0.clone()),
                _ => None,
            })
            .collect();
        if !missing.is_empty() {
            return Err(SynergosNetError::Transfer(format!(
                "fetch_dag: {} chunk(s) NotFound (first: {})",
                missing.len(),
                missing.first().cloned().unwrap_or_default()
            )));
        }
        Ok(dag)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::store::MemoryContentStore;

    /// 何も接続していなくても have_query が空の入力で早期 return すること。
    #[tokio::test]
    async fn have_query_empty_is_noop() {
        use crate::identity::Identity;
        use std::net::Ipv4Addr;
        let id = Arc::new(Identity::generate());
        let quic = Arc::new(QuicManager::new(
            crate::config::QuicConfig {
                max_concurrent_streams: 8,
                idle_timeout_ms: 5000,
                max_udp_payload_size: 1350,
                enable_0rtt: false,
            },
            id.clone(),
        ));
        let _ = quic.bind((Ipv4Addr::LOCALHOST, 0).into()).await.unwrap();
        let store: Arc<MemoryContentStore> = Arc::new(MemoryContentStore::new());
        let session = BitswapSession::new(quic, PeerId::new("nobody"), store);
        let out = session.have_query(&[]).await.unwrap();
        assert!(out.is_empty());
    }

    /// 入力 CID がローカルに既にあれば、相手に問い合わせずに Fetched を返すこと。
    #[tokio::test]
    async fn fetch_blocks_short_circuits_when_local() {
        use crate::identity::Identity;
        use std::net::Ipv4Addr;
        let id = Arc::new(Identity::generate());
        let quic = Arc::new(QuicManager::new(
            crate::config::QuicConfig {
                max_concurrent_streams: 8,
                idle_timeout_ms: 5000,
                max_udp_payload_size: 1350,
                enable_0rtt: false,
            },
            id.clone(),
        ));
        let _ = quic.bind((Ipv4Addr::LOCALHOST, 0).into()).await.unwrap();
        let store: Arc<MemoryContentStore> = Arc::new(MemoryContentStore::new());
        let block = Block::new(b"pre-cached".to_vec());
        let cid = block.cid.clone();
        store.put(block).await.unwrap();

        let session = BitswapSession::new(quic, PeerId::new("nobody"), store);
        let out = session
            .fetch_blocks(std::slice::from_ref(&cid))
            .await
            .unwrap();
        assert_eq!(out.len(), 1);
        matches!(out[0].1, FetchOutcome::Fetched);
    }
}
