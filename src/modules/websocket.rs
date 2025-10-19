use crate::modules::types::{
    AppState, BookMessage, MarketUpdate, MarketWebSocketMessages, PriceChangeMessage,
};
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tokio::time::{sleep, Duration};
use tokio_tungstenite::{connect_async, tungstenite::Message};

// -------------------- WebSocket Client --------------------
pub async fn websocket_client_task(
    ws_url: String,
    asset_id: String,
    market_tx: mpsc::Sender<MarketUpdate>,
    state: Arc<Mutex<AppState>>,
) {
    println!("Starting websocket client task...");
    // Keep a copy to filter incoming messages strictly to this asset only
    let target_asset_id = asset_id.clone();
    let mut backoff_secs = 1u64;
    loop {
        // Only break on shutdown after successful backoff iteration to avoid early exits
        if is_shutting_down(&state).await {
            println!("Market WS: shutdown requested. Exiting loop.");
            break;
        }
        println!("Connecting to websocket...");
        match connect_async(&ws_url).await {
            Ok((mut ws, _)) => {
                // Subscribe to market channel for provided asset token IDs
                println!(
                    "Subscribing to market channel for asset: {}",
                    target_asset_id
                );
                if !target_asset_id.is_empty() {
                    // Per API, market WS expects just {"assets_ids": [...]}
                    let sub_msg = json!({"assets_ids": [target_asset_id]});
                    let res = ws.send(Message::Text(sub_msg.to_string().into())).await;
                    if res.is_err() {
                        println!("Error sending market subscription: {:?}", res);
                        return;
                    }
                    println!("Sent market subscription payload: {}", sub_msg);
                }

                backoff_secs = 1;
                while let Some(msg) = ws.next().await {
                    if is_shutting_down(&state).await {
                        let _ = ws.close(None).await;
                        return;
                    }
                    match msg {
                        Ok(Message::Text(txt)) => {
                            for u in parse_update(&txt) {
                                if u.asset_id == target_asset_id {
                                    if market_tx.send(u).await.is_err() {
                                        return;
                                    }
                                }
                            }
                        }
                        Ok(Message::Ping(p)) => {
                            let _ = ws.send(Message::Pong(p)).await;
                        }
                        Ok(Message::Close(_)) => {
                            break;
                        }
                        Err(_) => {
                            break;
                        }
                        _ => {}
                    }
                }
            }
            Err(_) => {}
        }

        if is_shutting_down(&state).await {
            break;
        }
        sleep(Duration::from_secs(backoff_secs)).await;
        backoff_secs = (backoff_secs * 2).min(30);
    }
}

// -------------------- Message Parsing --------------------
pub fn parse_update(txt: &str) -> Vec<MarketUpdate> {
    let mut out = Vec::new();

    // Try array first: market WS may send batches like `[ {...}, {...} ]`
    if let Ok(msgs) = serde_json::from_str::<Vec<MarketWebSocketMessages>>(txt) {
        println!("Parsed array of MarketWebSocketMessages: {:?}", msgs);
        for msg in msgs {
            push_updates_from_msg(&mut out, msg);
        }
        return out;
    }

    // Fallback: explicit array of BookMessage
    if let Ok(books) = serde_json::from_str::<Vec<BookMessage>>(txt) {
        println!("Parsed array of BookMessage: {:?}", books);
        for book in books {
            push_updates_from_msg(&mut out, MarketWebSocketMessages::BookMessage(book));
        }
        return out;
    }

    // Fallback: explicit array of PriceChangeMessage
    if let Ok(price_msgs) = serde_json::from_str::<Vec<PriceChangeMessage>>(txt) {
        println!("Parsed array of PriceChangeMessage: {:?}", price_msgs);
        for pcm in price_msgs {
            push_updates_from_msg(&mut out, MarketWebSocketMessages::PriceChangeMessage(pcm));
        }
        return out;
    }

    // Fallback to single message
    match serde_json::from_str::<MarketWebSocketMessages>(txt) {
        Ok(msg) => {
            println!("Parsed single MarketWebSocketMessage: {:?}", msg);
            push_updates_from_msg(&mut out, msg);
        }
        Err(err) => {
            // Try single BookMessage
            if let Ok(book) = serde_json::from_str::<BookMessage>(txt) {
                println!("Parsed single BookMessage: {:?}", book);
                push_updates_from_msg(&mut out, MarketWebSocketMessages::BookMessage(book));
                return out;
            }
            // Try single PriceChangeMessage
            if let Ok(pcm) = serde_json::from_str::<PriceChangeMessage>(txt) {
                println!("Parsed single PriceChangeMessage: {:?}", pcm);
                push_updates_from_msg(&mut out, MarketWebSocketMessages::PriceChangeMessage(pcm));
                return out;
            }
            println!("Error parsing update: {}", txt);
            println!("Error: {:?}", err);
        }
    }

    out
}

fn push_updates_from_msg(out: &mut Vec<MarketUpdate>, msg: MarketWebSocketMessages) {
    match msg {
        MarketWebSocketMessages::BookMessage(book) => {
            let asset_id = book.asset_id;
            let parse = |s: &str| {
                let t = if s.starts_with('.') {
                    format!("0{}", s)
                } else {
                    s.to_string()
                };
                t.parse::<f64>().ok()
            };
            let best_bid = book
                .bids
                .last()
                .and_then(|b| parse(&b.price))
                .unwrap_or(0.0);
            let best_ask = book
                .asks
                .last()
                .and_then(|a| parse(&a.price))
                .unwrap_or(1.0);
            let ts = book.timestamp.parse::<i64>().unwrap_or(0);
            out.push(MarketUpdate {
                asset_id,
                best_bid,
                best_ask,
                ts,
            });
        }
        MarketWebSocketMessages::PriceChangeMessage(price_change_message) => {
            for change in price_change_message.price_changes {
                let asset_id = change.asset_id;
                let best_bid = change.best_bid.parse::<f64>().unwrap_or(0.0);
                let best_ask = change.best_ask.parse::<f64>().unwrap_or(1.0);
                let ts = price_change_message.timestamp.parse::<i64>().unwrap_or(0);
                out.push(MarketUpdate {
                    asset_id,
                    best_bid,
                    best_ask,
                    ts,
                });
            }
        }
        _ => {}
    }
}

// -------------------- Helper Functions --------------------
async fn is_shutting_down(state: &Arc<Mutex<AppState>>) -> bool {
    state.lock().await.shutting_down
}
