use dashmap::DashMap;

use crate::types::{FileId, PeerId};

/// Want エントリ（受信者が「欲しい」と宣言）
#[derive(Debug, Clone)]
pub struct WantEntry {
    pub requester: PeerId,
    pub file_id: FileId,
    pub version: u64,
    pub requested_at: u64,
    pub state: LedgerEntryState,
}

/// Offer エントリ（送信者が「送りたい」と宣言）
#[derive(Debug, Clone)]
pub struct OfferEntry {
    pub sender: PeerId,
    pub file_id: FileId,
    pub version: u64,
    pub file_size: u64,
    pub crc: u32,
    pub offered_at: u64,
    pub state: LedgerEntryState,
}

/// 台帳エントリの状態
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LedgerEntryState {
    /// 待機中（マッチ相手を待っている）
    Pending,
    /// マッチ済み（転送中）
    Matched,
    /// 転送完了
    Fulfilled,
    /// 取り消し
    Cancelled,
}

/// マッチング結果のアクション
#[derive(Debug, Clone)]
pub enum LedgerAction {
    /// キューに追加された
    Queued,
    /// 重複のため無視
    Duplicate,
    /// マッチ成立 → 転送開始
    Match { sender: PeerId, file_size: u64 },
}

/// Want/Offer 転送台帳
///
/// ファイルID + バージョン をキーに Want と Offer をマッチングし、
/// 重複転送を防止する。
pub struct TransferLedger {
    wants: DashMap<(FileId, u64), Vec<WantEntry>>,
    offers: DashMap<(FileId, u64), OfferEntry>,
}

impl TransferLedger {
    pub fn new() -> Self {
        Self {
            wants: DashMap::new(),
            offers: DashMap::new(),
        }
    }

    /// Want を登録（重複チェック付き）
    pub fn register_want(&self, want: WantEntry) -> LedgerAction {
        let key = (want.file_id.clone(), want.version);

        // 重複チェック: 同じ requester が同じ (file, version) を既に Want していたら無視
        if let Some(existing) = self.wants.get(&key) {
            if existing
                .iter()
                .any(|w| w.requester == want.requester && w.state != LedgerEntryState::Cancelled)
            {
                return LedgerAction::Duplicate;
            }
        }

        // Offer が既にあるか確認
        if let Some(offer) = self.offers.get(&key) {
            if offer.state == LedgerEntryState::Pending {
                return LedgerAction::Match {
                    sender: offer.sender.clone(),
                    file_size: offer.file_size,
                };
            }
        }

        self.wants.entry(key).or_default().push(want);
        LedgerAction::Queued
    }

    /// Offer を登録し、待機中の Want すべてとマッチ
    pub fn register_offer(&self, offer: OfferEntry) -> Vec<LedgerAction> {
        let key = (offer.file_id.clone(), offer.version);
        let mut actions = Vec::new();

        // 待機中の Want をすべてマッチ
        if let Some(wants) = self.wants.get(&key) {
            for want in wants.iter() {
                if want.state == LedgerEntryState::Pending {
                    actions.push(LedgerAction::Match {
                        sender: offer.sender.clone(),
                        file_size: offer.file_size,
                    });
                }
            }
        }

        self.offers.insert(key, offer);

        if actions.is_empty() {
            vec![LedgerAction::Queued]
        } else {
            actions
        }
    }

    /// エントリを Fulfilled 状態にする
    pub fn mark_fulfilled(&self, file_id: &FileId, version: u64, peer: &PeerId) {
        let key = (file_id.clone(), version);
        if let Some(mut wants) = self.wants.get_mut(&key) {
            for want in wants.iter_mut() {
                if want.requester == *peer {
                    want.state = LedgerEntryState::Fulfilled;
                }
            }
        }
    }

    /// Want を取り消す
    pub fn cancel_want(&self, file_id: &FileId, version: u64, requester: &PeerId) {
        let key = (file_id.clone(), version);
        if let Some(mut wants) = self.wants.get_mut(&key) {
            for want in wants.iter_mut() {
                if want.requester == *requester {
                    want.state = LedgerEntryState::Cancelled;
                }
            }
        }
    }

    /// 指定ファイルの Pending な Want 数を取得
    pub fn pending_want_count(&self, file_id: &FileId, version: u64) -> usize {
        let key = (file_id.clone(), version);
        self.wants
            .get(&key)
            .map(|wants| {
                wants
                    .iter()
                    .filter(|w| w.state == LedgerEntryState::Pending)
                    .count()
            })
            .unwrap_or(0)
    }
}

impl Default for TransferLedger {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> u64 {
        1000
    }

    #[test]
    fn test_want_then_offer_match() {
        let ledger = TransferLedger::new();

        let want = WantEntry {
            requester: PeerId::new("B"),
            file_id: FileId::new("f1"),
            version: 3,
            requested_at: now(),
            state: LedgerEntryState::Pending,
        };
        let result = ledger.register_want(want);
        assert!(matches!(result, LedgerAction::Queued));

        let offer = OfferEntry {
            sender: PeerId::new("A"),
            file_id: FileId::new("f1"),
            version: 3,
            file_size: 1024,
            crc: 0xDEAD,
            offered_at: now(),
            state: LedgerEntryState::Pending,
        };
        let actions = ledger.register_offer(offer);
        assert_eq!(actions.len(), 1);
        assert!(matches!(actions[0], LedgerAction::Match { .. }));
    }

    #[test]
    fn test_offer_then_want_match() {
        let ledger = TransferLedger::new();

        let offer = OfferEntry {
            sender: PeerId::new("A"),
            file_id: FileId::new("f1"),
            version: 3,
            file_size: 1024,
            crc: 0xDEAD,
            offered_at: now(),
            state: LedgerEntryState::Pending,
        };
        ledger.register_offer(offer);

        let want = WantEntry {
            requester: PeerId::new("B"),
            file_id: FileId::new("f1"),
            version: 3,
            requested_at: now(),
            state: LedgerEntryState::Pending,
        };
        let result = ledger.register_want(want);
        assert!(matches!(result, LedgerAction::Match { .. }));
    }

    #[test]
    fn test_duplicate_want_blocked() {
        let ledger = TransferLedger::new();

        let want = WantEntry {
            requester: PeerId::new("B"),
            file_id: FileId::new("f1"),
            version: 3,
            requested_at: now(),
            state: LedgerEntryState::Pending,
        };
        ledger.register_want(want.clone());
        let result = ledger.register_want(want);
        assert!(matches!(result, LedgerAction::Duplicate));
    }
}
