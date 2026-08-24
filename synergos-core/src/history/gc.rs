//! 履歴ノードの保持ポリシー (docs/versioning-design.md §3.5)。
//!
//! 純粋関数: 索引と設定から「削ってよい (rel, version)」を決めるだけで
//! ファイル操作はしない (`HistoryStore::gc` が実行する)。

use std::collections::BTreeSet;

use synergos_net::config::HistoryConfig;

use super::index::HistoryIndex;

/// gc の実行結果。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GcReport {
    pub removed_versions: Vec<(String, u64)>,
    pub removed_objects: usize,
    pub bytes_freed: u64,
}

/// 保持ポリシーを適用して削除対象を返す。
///
/// 1. `keep` に含まれる版は決して削らない
/// 2. `max_versions_per_file > 0`: path ごとに新しい N 版を残す
/// 3. `max_age_days > 0`: `stored_at` がそれより古い版を削る
/// 4. `max_bytes > 0`: 残った版の unique object 合計がこれを超える間、
///    `stored_at` の古い順に削る
pub fn apply_retention(
    index: &HistoryIndex,
    config: &HistoryConfig,
    keep: &[(String, u64)],
    now_ms: u64,
) -> Vec<(String, u64)> {
    let protected: BTreeSet<(&str, u64)> = keep.iter().map(|(r, v)| (r.as_str(), *v)).collect();
    let mut victims: BTreeSet<(String, u64)> = BTreeSet::new();

    // 2. per-file 版数上限
    if config.max_versions_per_file > 0 {
        let keep_count = usize::try_from(config.max_versions_per_file).unwrap_or(usize::MAX);
        for (rel, versions) in &index.entries {
            let mut sorted: Vec<u64> = versions
                .iter()
                .filter(|(_, entry)| entry.offloaded.is_none())
                .map(|(version, _)| *version)
                .collect();
            sorted.sort_unstable_by(|a, b| b.cmp(a));
            for v in sorted
                .into_iter()
                .skip(keep_count)
            {
                if !protected.contains(&(rel.as_str(), v)) {
                    victims.insert((rel.clone(), v));
                }
            }
        }
    }

    // 3. 保持期間
    if config.max_age_days > 0 {
        let max_age_ms = config
            .max_age_days
            .saturating_mul(24)
            .saturating_mul(60)
            .saturating_mul(60)
            .saturating_mul(1000);
        let cutoff = now_ms.saturating_sub(max_age_ms);
        for (rel, v, entry) in index.iter_all() {
            if entry.offloaded.is_none()
                && entry.stored_at < cutoff
                && !protected.contains(&(rel, v))
            {
                victims.insert((rel.to_string(), v));
            }
        }
    }

    // 4. 総容量
    if config.max_bytes > 0 {
        let mut object_refs: std::collections::BTreeMap<&str, (usize, u64)> =
            std::collections::BTreeMap::new();
        for (rel, version, entry) in index.iter_all() {
            if entry.offloaded.is_some() || victims.contains(&(rel.to_string(), version)) {
                continue;
            }
            object_refs
                .entry(entry.hash.as_str())
                .and_modify(|(count, _)| *count += 1)
                .or_insert((1, entry.size));
        }
        let mut total = object_refs.values().fold(0u64, |sum, (_, size)| {
            sum.saturating_add(*size)
        });
        let mut oldest: Vec<(u64, String, u64)> = index
            .iter_all()
            .filter(|(rel, v, entry)| {
                entry.offloaded.is_none() && !protected.contains(&(rel, *v))
            })
            .map(|(rel, v, e)| (e.stored_at, rel.to_string(), v))
            .collect();
        oldest.sort();
        for (_, rel, v) in oldest {
            if total <= config.max_bytes {
                break;
            }
            if !victims.insert((rel.clone(), v)) {
                continue;
            }
            let Some(entry) = index.get(&rel, v) else {
                continue;
            };
            if let Some((count, size)) = object_refs.get_mut(entry.hash.as_str()) {
                *count -= 1;
                if *count == 0 {
                    total = total.saturating_sub(*size);
                }
            }
        }
    }

    victims.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::index::{IndexEntry, OffloadedRef};

    fn idx() -> HistoryIndex {
        let mut i = HistoryIndex::new("p");
        for (rel, v, hash, size, at) in [
            ("a", 1, "h1", 100, 1_000),
            ("a", 2, "h2", 100, 2_000),
            ("a", 3, "h3", 100, 3_000),
            ("b", 1, "h1", 100, 1_500), // a/1 と同じ内容 (dedup)
        ] {
            i.insert(
                rel,
                v,
                IndexEntry {
                    hash: hash.into(),
                    size,
                    crc: 0,
                    stored_at: at,
                    publisher: String::new(),
                    source: String::new(),
                    offloaded: None,
                },
            );
        }
        i
    }

    fn cfg() -> HistoryConfig {
        HistoryConfig {
            enabled: true,
            ..HistoryConfig::default()
        }
    }

    #[test]
    fn unlimited_policy_removes_nothing() {
        assert!(apply_retention(&idx(), &cfg(), &[], 10_000).is_empty());
    }

    #[test]
    fn max_versions_keeps_newest_and_protected() {
        let mut c = cfg();
        c.max_versions_per_file = 1;
        let v = apply_retention(&idx(), &c, &[("a".into(), 1)], 10_000);
        assert_eq!(v, vec![("a".to_string(), 2)]);
    }

    #[test]
    fn max_age_uses_stored_at() {
        let mut c = cfg();
        c.max_age_days = 1;
        let day = 24 * 60 * 60 * 1000;
        let v = apply_retention(&idx(), &c, &[], day + 2_500);
        assert_eq!(
            v,
            vec![
                ("a".to_string(), 1),
                ("a".to_string(), 2),
                ("b".to_string(), 1)
            ]
        );
    }

    #[test]
    fn retention_never_removes_offloaded_versions() {
        let mut index = idx();
        let mut offloaded = index.get("a", 1).unwrap().clone();
        offloaded.offloaded = Some(OffloadedRef {
            backend: "local_path".into(),
            key: format!("{}/{}", "a".repeat(64), "b".repeat(64)),
            config: None,
        });
        index.insert("a", 1, offloaded);
        let mut config = cfg();
        config.max_versions_per_file = 1;
        config.max_age_days = 1;
        config.max_bytes = 1;

        let victims = apply_retention(&index, &config, &[], 2 * 24 * 60 * 60 * 1000);
        assert!(!victims.contains(&("a".to_string(), 1)));
    }

    #[test]
    fn max_bytes_drops_oldest_until_under_limit_counting_unique_objects() {
        let mut c = cfg();
        // unique objects: h1,h2,h3 = 300 bytes。上限 200 → 一番古い a/1 (h1) を削っても
        // b/1 が h1 を参照するので容量は減らず、次に古い b/1 も削って 200 になる
        c.max_bytes = 200;
        let v = apply_retention(&idx(), &c, &[], 10_000);
        assert_eq!(v, vec![("a".to_string(), 1), ("b".to_string(), 1)]);
    }
}
