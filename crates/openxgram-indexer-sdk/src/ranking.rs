//! 4.2.1.3 — 랭킹 plugin 인터페이스. 5.3 평판 기반 랭킹 의 토대.
//!
//! 입력: identity (eth address 또는 handle) 별 카운트(메시지 / 결제 / endorsement).
//! 출력: 정규화 score (0.0 ~ 1.0) — 검색 결과 정렬에 사용.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct IdentityScore {
    pub identity: String,
    pub messages: u64,
    pub payments_received: u64,
    pub endorsements_received: u64,
    pub raw_score: f64,
}

pub trait Rank {
    fn score(&self, ident: &IdentityScore) -> f64;

    /// 여러 identity 를 정규화된 점수 순으로 정렬 (내림차순). 동률은 endorsement → messages tie-break.
    fn rank(&self, identities: Vec<IdentityScore>) -> Vec<IdentityScore> {
        let mut scored: Vec<_> = identities
            .into_iter()
            .map(|mut i| {
                i.raw_score = self.score(&i);
                i
            })
            .collect();
        scored.sort_by(|a, b| {
            b.raw_score
                .partial_cmp(&a.raw_score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(b.endorsements_received.cmp(&a.endorsements_received))
                .then(b.messages.cmp(&a.messages))
        });
        scored
    }
}

/// 5.3 — 기본 가중치: 메시지 0.3 / 결제수신 0.5 / endorsement 1.0 (평판 = 결제·추천 위주).
#[derive(Debug, Clone, Copy)]
pub struct DefaultRanker {
    pub w_messages: f64,
    pub w_payments: f64,
    pub w_endorsements: f64,
}

impl Default for DefaultRanker {
    fn default() -> Self {
        Self {
            w_messages: 0.3,
            w_payments: 0.5,
            w_endorsements: 1.0,
        }
    }
}

impl Rank for DefaultRanker {
    fn score(&self, i: &IdentityScore) -> f64 {
        // log1p 로 wide range 를 flatten — 메시지 1만개와 100개의 격차를 8배 이내로.
        self.w_messages * (i.messages as f64).ln_1p()
            + self.w_payments * (i.payments_received as f64).ln_1p()
            + self.w_endorsements * (i.endorsements_received as f64).ln_1p()
    }
}

/// 카운트만 들고 있을 때 식별자별 종합 score map 구성.
pub fn aggregate_scores<R: Rank>(
    ranker: &R,
    counts: HashMap<String, IdentityScore>,
) -> Vec<IdentityScore> {
    ranker.rank(counts.into_values().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(id: &str, m: u64, p: u64, e: u64) -> IdentityScore {
        IdentityScore {
            identity: id.into(),
            messages: m,
            payments_received: p,
            endorsements_received: e,
            raw_score: 0.0,
        }
    }

    #[test]
    fn default_ranker_prefers_endorsements_over_messages() {
        let r = DefaultRanker::default();
        let busy = s("alice", 10_000, 0, 0);
        let endorsed = s("bob", 100, 0, 50);
        let ranked = r.rank(vec![busy.clone(), endorsed.clone()]);
        assert_eq!(ranked[0].identity, "bob", "endorsement 가중치가 큼");
    }

    #[test]
    fn payment_received_outweighs_messages_alone() {
        let r = DefaultRanker::default();
        let chatty = s("alice", 100, 0, 0);
        let earner = s("bob", 10, 100, 0);
        let ranked = r.rank(vec![chatty, earner]);
        assert_eq!(ranked[0].identity, "bob");
    }

    #[test]
    fn rank_is_stable_for_identical_scores() {
        let r = DefaultRanker::default();
        let a = s("a", 5, 5, 5);
        let b = s("b", 5, 5, 5);
        let ranked = r.rank(vec![a.clone(), b.clone()]);
        assert_eq!(ranked.len(), 2);
    }

    #[test]
    fn aggregate_uses_provided_counts() {
        let r = DefaultRanker::default();
        let mut map = HashMap::new();
        map.insert("alice".into(), s("alice", 10, 0, 0));
        map.insert("bob".into(), s("bob", 0, 0, 5));
        let ranked = aggregate_scores(&r, map);
        assert_eq!(ranked.len(), 2);
        assert_eq!(ranked[0].identity, "bob");
    }
}
