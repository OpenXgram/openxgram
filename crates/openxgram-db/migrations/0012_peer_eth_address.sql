-- peer.eth_address — envelope.from 매칭용 secp256k1 EIP-55 주소.
-- public_key 와는 다른 표현 (address = keccak256(pubkey)[12..]) 이라 별도 컬럼.
-- inbound webhook 에서 받은 envelope.from 과 직접 비교 가능.

ALTER TABLE peers ADD COLUMN eth_address TEXT;
CREATE INDEX idx_peers_eth_address ON peers(eth_address);
