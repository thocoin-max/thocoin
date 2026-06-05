# Coin222

Bitcoin-like cryptocurrency in Rust.

## Tham số
- Cap: 222,000,000 COIN (hard cap, mint dừng khi đạt)
- Reward khởi đầu: 110.109935 COIN/block
- Halving: mỗi 210,000 block
- Block time: 10 phút, PoW SHA-256d
- Difficulty adjust: mỗi 2016 block
- Address prefix: 0x32 (Base58Check)
- P2P port: 22221, RPC port: 22222

## Build
```bash
cargo build --release
```

## Run daemon
```bash
./target/release/coin222d --mine
```

## Run GUI (ví + miner tích hợp)
```bash
./target/release/coin222-gui
```

## RPC
```bash
curl -X POST http://127.0.0.1:22222 -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"getinfo","params":[]}'
```

## Lưu ý bảo mật còn thiếu (cần bổ sung trước mainnet)
- Script verify đầy đủ (hiện chỉ check P2PKH cơ bản)
- Header chain reorg & fork choice (longest-work)
- BIP32 HD wallet, mã hoá file key
- Difficulty adjust thực thi
- DoS protection, peer scoring, banning
- TLS/Noise cho P2P
- Audit độc lập
"# thocoin" 
