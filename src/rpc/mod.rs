use std::sync::Arc;
use jsonrpc_core::{IoHandler, Params, Value};
use jsonrpc_http_server::{ServerBuilder, hyper, RequestMiddleware, RequestMiddlewareAction};
use crate::core::chain::ChainState;
use crate::wallet::Wallet;
use crate::core::consensus::RPC_PORT;
use crate::core::hash::hash_to_hex;

fn rpc_token() -> String {
    if let Ok(t) = std::env::var("THOCOIN_RPC_TOKEN") {
        if !t.is_empty() { return t; }
    }
    use rand::RngCore;
    let mut b = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut b);
    let tok = hex::encode(b);
    eprintln!("[RPC] token tự sinh (đặt THOCOIN_RPC_TOKEN để cố định): {tok}");
    tok
}

struct AuthMiddleware { token: String }

impl RequestMiddleware for AuthMiddleware {
    fn on_request(&self, request: hyper::Request<hyper::Body>) -> RequestMiddlewareAction {
        let ok = request.headers()
            .get(hyper::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .map(|v| {
                let expected = format!("Bearer {}", self.token);
                use subtle::ConstantTimeEq;
                v.as_bytes().ct_eq(expected.as_bytes()).into()
            })
            .unwrap_or(false);
        if ok {
            request.into()
        } else {
            hyper::Response::builder()
                .status(hyper::StatusCode::UNAUTHORIZED)
                .body(hyper::Body::from("unauthorized"))
                .unwrap()
                .into()
        }
    }
}

pub fn start_rpc(chain: Arc<ChainState>, wallet: Arc<Wallet>, mempool: crate::core::mempool::Mempool) -> jsonrpc_http_server::Server {
    let mut io = IoHandler::new();

    let c = chain.clone();
    io.add_sync_method("getinfo", move |_| {
        Ok(serde_json::json!({
            "height": *c.height.read(),
            "supply": *c.supply.read(),
            "tip": hash_to_hex(&*c.tip.read()),
        }))
    });

    let w = wallet.clone();
    io.add_sync_method("getaddress", move |_| Ok(Value::String(w.address())));

    let c2 = chain.clone();
    let w2 = wallet.clone();
    io.add_sync_method("getbalance", move |_| {
        Ok(Value::String(w2.balance(&c2).to_string()))
    });

    let c3 = chain.clone();
    let w3 = wallet.clone();
    let mp = mempool.clone();
    io.add_sync_method("send", move |p: Params| {
        let arr: Vec<Value> = p.parse().map_err(|_| jsonrpc_core::Error::invalid_params("need [to, amount]"))?;
        if arr.len() < 2 {
            return Err(jsonrpc_core::Error::invalid_params("need [to, amount]"));
        }
        let to = arr[0].as_str().unwrap_or("").to_string();
        let amount = arr[1].as_u64().unwrap_or(0);

        if let Some(secret) = crate::wallet::totp::load_secret_beside(&w3.path) {
            let code = arr.get(2).and_then(|v| v.as_str()).unwrap_or("");
            if !crate::wallet::totp::verify(&secret, code) {
                return Err(jsonrpc_core::Error::invalid_params(
                    "2FA đang bật: cần mã TOTP hợp lệ làm tham số thứ 3 [to, amount, code]"));
            }
        }

        match w3.send(&c3, &to, amount, 1000) {
            Ok(tx) => {
                let txid = hash_to_hex(&tx.txid());
                // Vào mempool → vòng announce P2P sẽ relay INV. Trước đây tx bị vứt sau khi tạo.
                mp.accept(&c3, tx)
                    .map_err(|e| jsonrpc_core::Error::invalid_params(format!("mempool tu choi: {e}")))?;
                Ok(Value::String(txid))
            }
            Err(e) => Err(jsonrpc_core::Error::invalid_params(e.to_string())),
        }
    });

    let token = rpc_token();
    ServerBuilder::new(io)
        .threads(2)
        .request_middleware(AuthMiddleware { token })
        .start_http(&format!("127.0.0.1:{}", RPC_PORT).parse().unwrap())
        .expect("RPC start failed")
}
