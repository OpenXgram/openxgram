//! Application-layer ratchet — 주기적 ephemeral key 회전 + NIP-44 v2 wrap (PRD-NOSTR-05).
//!
//! 설계: 매 period (default 7일) 마다 새 ephemeral keypair 생성. 옛 secret 은
//! 2 period 이상 지나면 폐기 → forward secrecy. publish 는 ratchet 전용 kind 30050
//! addressable event (d=`ratchet:{period}`) 로 peer 에게 노출.
//!
//! 본 모듈은 키 관리·encrypt·decrypt 만 담당. 회전 cron 통합은 scheduler 측 책임.

use crate::{NostrError, NostrKind, Result};
use nostr::nips::nip44::{self, Version};
use nostr::{EventBuilder, Keys, PublicKey, SecretKey, Tag};

const DEFAULT_PERIOD_SECS: u64 = 7 * 24 * 3600; // 1주
const RETAIN_PERIODS: u64 = 2; // 현재 + 직전 1 period 까지만 secret 보관

#[derive(Debug, Clone)]
pub struct RatchetKey {
    pub period: u64, // period index = unix_ts / period_secs
    pub secret: SecretKey,
    pub public: PublicKey,
}

impl RatchetKey {
    fn generate(period: u64) -> Self {
        let keys = Keys::generate();
        Self {
            period,
            secret: keys.secret_key().clone(),
            public: keys.public_key(),
        }
    }
}

#[derive(Debug)]
pub struct Ratchet {
    keys: Vec<RatchetKey>, // ascending period
    period_secs: u64,
}

impl Default for Ratchet {
    fn default() -> Self {
        Self::new(DEFAULT_PERIOD_SECS)
    }
}

impl Ratchet {
    pub fn new(period_secs: u64) -> Self {
        assert!(period_secs > 0, "period must be positive");
        Self {
            keys: Vec::new(),
            period_secs,
        }
    }

    pub fn period_secs(&self) -> u64 {
        self.period_secs
    }

    pub fn period_of(&self, unix_ts: u64) -> u64 {
        unix_ts / self.period_secs
    }

    /// 현재 period 키. 없으면 생성. 옛 키 정리.
    pub fn current(&mut self, unix_ts: u64) -> &RatchetKey {
        let p = self.period_of(unix_ts);
        if !self.keys.iter().any(|k| k.period == p) {
            self.keys.push(RatchetKey::generate(p));
            self.keys.sort_by_key(|k| k.period);
        }
        // 옛 키 폐기 — period < (current - RETAIN_PERIODS) 제거
        let cutoff = p.saturating_sub(RETAIN_PERIODS);
        self.keys.retain(|k| k.period >= cutoff);
        self.keys
            .iter()
            .find(|k| k.period == p)
            .expect("just inserted")
    }

    /// 강제 회전 — 현재 period 키 폐기 후 새로 생성. 테스트용.
    pub fn rotate_now(&mut self, unix_ts: u64) {
        let p = self.period_of(unix_ts);
        self.keys.retain(|k| k.period != p);
        let _ = self.current(unix_ts);
    }

    /// NIP-44 v2 로 content wrap. 현재 ratchet secret + recipient pubkey.
    pub fn wrap(&mut self, unix_ts: u64, recipient: &PublicKey, content: &str) -> Result<String> {
        let key = self.current(unix_ts);
        nip44::encrypt(&key.secret, recipient, content, Version::V2)
            .map_err(|e| NostrError::Nostr(e.to_string()))
    }

    /// 보유 중인 모든 ratchet secret 으로 시도. 모두 실패 시 에러 (forward secrecy).
    pub fn unwrap(&self, sender: &PublicKey, payload: &str) -> Result<String> {
        let mut last_err = None;
        for k in self.keys.iter().rev() {
            match nip44::decrypt(&k.secret, sender, payload) {
                Ok(s) => return Ok(s),
                Err(e) => last_err = Some(NostrError::Nostr(e.to_string())),
            }
        }
        Err(last_err.unwrap_or(NostrError::Nostr("no ratchet keys available".into())))
    }

    /// 현재 ratchet pubkey 를 announce 하는 kind 30050 event (master keys 로 서명).
    /// content 는 hex pubkey, d-tag 는 `ratchet:{period}`.
    pub fn build_announce(&mut self, master_keys: &Keys, unix_ts: u64) -> Result<nostr::Event> {
        let key = self.current(unix_ts);
        let d = format!("ratchet:{}", key.period);
        let pubkey_hex = key.public.to_hex();
        let builder = EventBuilder::new(nostr::Kind::from(NostrKind::RatchetKey), pubkey_hex)
            .tags(vec![Tag::identifier(d)]);
        builder
            .sign_with_keys(master_keys)
            .map_err(|e| NostrError::Nostr(e.to_string()))
    }

    pub fn retained_periods(&self) -> Vec<u64> {
        self.keys.iter().map(|k| k.period).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys() -> Keys {
        Keys::generate()
    }

    #[test]
    fn current_generates_and_caches() {
        let mut r = Ratchet::new(60);
        let p1 = r.current(60).period;
        let p2 = r.current(119).period;
        assert_eq!(p1, p2); // 같은 60s 윈도우 (period 1 = ts ∈ [60, 119])
        assert_eq!(r.retained_periods().len(), 1);
    }

    #[test]
    fn period_advance_keeps_recent_drops_old() {
        let mut r = Ratchet::new(60);
        let _ = r.current(0); // period 0
        let _ = r.current(60); // period 1
        let _ = r.current(180); // period 3 — drops 0
        let periods = r.retained_periods();
        assert!(!periods.contains(&0));
        assert!(periods.contains(&3));
    }

    #[test]
    fn wrap_unwrap_round_trip() {
        let mut sender = Ratchet::new(60);
        let mut recipient = Ratchet::new(60);
        let recv_pubkey = recipient.current(0).public;
        let payload = sender.wrap(0, &recv_pubkey, "hello").unwrap();

        // recipient 가 unwrap 하려면 sender 의 pubkey 필요
        let send_pubkey = sender.current(0).public;
        let plain = recipient.unwrap(&send_pubkey, &payload).unwrap();
        assert_eq!(plain, "hello");
    }

    #[test]
    fn forward_secrecy_old_payload_undecryptable_after_rotation() {
        let mut sender = Ratchet::new(60);
        let recipient_keys = keys();
        let recv_pubkey = recipient_keys.public_key();
        let recipient_secret = recipient_keys.secret_key().clone();

        // period 0 에서 payload 작성
        let payload = sender.wrap(0, &recv_pubkey, "secret-msg").unwrap();
        let send_pubkey_old = sender.current(0).public;

        // 2 period 진행 → sender 의 옛 secret 폐기됨
        let _ = sender.current(180); // period 3 — period 0 secret 제거
        assert!(!sender.retained_periods().contains(&0));

        // 외부 관찰자가 옛 payload 입수해도 sender 옛 secret 은 폐기 — 단방향
        // 단, 수신자(recipient) 가 sender pubkey 알고 자기 secret 이 있다면 복호화 가능
        // → forward secrecy 는 sender 측 키 폐기 시 sender 가 재생성 못하게 한다
        let plain = nip44::decrypt(&recipient_secret, &send_pubkey_old, &payload).unwrap();
        assert_eq!(plain, "secret-msg");

        // sender 가 자기 ratchet 으로 unwrap 시도 → 옛 secret 없음 → 실패
        let r = sender.unwrap(&recv_pubkey, &payload);
        assert!(
            r.is_err(),
            "forward secrecy: sender 옛 secret 폐기 후 unwrap 실패해야 함"
        );
    }

    #[test]
    fn announce_event_has_ratchet_d_tag() {
        let master = keys();
        let mut r = Ratchet::new(60);
        let event = r.build_announce(&master, 120).unwrap();
        assert_eq!(event.kind, nostr::Kind::from(30050u16));
        let has_ratchet_d = event.tags.iter().any(|t| {
            let s = t.as_slice();
            s.first().map(|x| x.as_str()) == Some("d")
                && s.get(1)
                    .map(|x| x.as_str())
                    .unwrap_or("")
                    .starts_with("ratchet:")
        });
        assert!(has_ratchet_d);
    }
}
