use crate::modules::types::{AppState, BotCommand, Side, UserWebSocketMessages};
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tokio::time::{sleep, Duration};
use tokio_tungstenite::{connect_async, tungstenite::Message};

const WS_USER_URL: &str = "wss://ws-subscriptions-clob.polymarket.com/ws/user";

pub async fn user_ws_task(
    api_key: String,
    api_secret: String,
    api_passphrase: String,
    state: Arc<Mutex<AppState>>,
    cmd_tx: mpsc::Sender<BotCommand>,
) {
    let mut backoff_secs = 1u64;
    loop {
        if is_shutting_down(&state).await {
            println!("User WS: shutdown requested. Exiting loop.");
            break;
        }

        // Connect and then authenticate with message payload (apiKey/secret/passphrase)
        match connect_async(WS_USER_URL).await {
            Ok((mut ws, _)) => {
                // Send authentication payload per spec
                let sub = json!({
                    "type": "user",
                    "auth": {
                        "apiKey": api_key,
                        "secret": api_secret,
                        "passphrase": api_passphrase
                    }
                })
                .to_string();
                let _ = ws.send(Message::Text(sub.into())).await;

                backoff_secs = 1;
                while let Some(msg) = ws.next().await {
                    if is_shutting_down(&state).await {
                        let _ = ws.close(None).await;
                        return;
                    }
                    match msg {
                        Ok(Message::Text(txt)) => {
                            handle_user_event(&txt, &state, &cmd_tx).await;
                        }
                        Ok(Message::Ping(p)) => {
                            let _ = ws.send(Message::Pong(p)).await;
                        }
                        Ok(Message::Close(_)) => break,
                        Err(_) => break,
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

async fn handle_user_event(
    txt: &str,
    state: &Arc<Mutex<AppState>>,
    _cmd_tx: &mpsc::Sender<BotCommand>,
) {
    // Peek event_type
    let Ok(msg) = serde_json::from_str::<UserWebSocketMessages>(txt) else {
        return;
    };

    // Only process events for the configured YES token asset_id; ignore others
    let target_asset_id = {
        let s = state.lock().await;
        s.yes_token.clone().unwrap_or_default()
    };
    if target_asset_id.is_empty() {
        // If not configured, ignore all to avoid contaminating state
        return;
    }

    match msg {
        UserWebSocketMessages::TradeMessage(msg) => {
            if msg.asset_id != target_asset_id {
                return;
            }
            // Only act on matched/confirmed statuses to update inventory
            match msg.status.as_str() {
                crate::modules::types::STATUS_MATCHED => {
                    let size = msg.size.parse::<f64>().unwrap();
                    let side = if msg
                        .side
                        .eq_ignore_ascii_case(crate::modules::types::SIDE_BUY)
                    {
                        Side::Buy
                    } else {
                        Side::Sell
                    };
                    // Update inventory immediately
                    let mut s = state.lock().await;
                    let current = s.inventory.get(&msg.asset_id).copied().unwrap();
                    let delta = match side {
                        Side::Buy => size,
                        Side::Sell => -size,
                    };
                    s.inventory.insert(msg.asset_id.clone(), current + delta);
                    // Keep last price updated
                    let ts = msg.timestamp.parse::<i64>().unwrap();
                    let now_ms = now_millis();
                    println!("Trade {} filled {} milliseconds later", msg.id, now_ms - ts);
                    // let entry = s
                    //     .last_prices
                    //     .entry(msg.asset_id.clone())
                    //     .or_insert((price, price, now_ms));
                    // *entry = (price, price, now_ms);
                }
                crate::modules::types::STATUS_MINED => {
                    // Do nothing
                }
                crate::modules::types::STATUS_CONFIRMED => {
                    // Do nothing
                }
                _ => {}
            }
        }
        UserWebSocketMessages::OrderMessage(msg) => {
            if msg.asset_id != target_asset_id {
                return;
            }
            // Maintain my_open_orders map
            let mut s = state.lock().await;
            match msg.msg_type.as_str() {
                crate::modules::types::MSG_PLACEMENT | crate::modules::types::MSG_UPDATE => {
                    // We don't have order id mapping to our internal Order here; just track id and rough info
                    s.my_open_orders.entry(msg.id.clone()).or_insert(
                        crate::modules::types::Order {
                            id: Some(msg.id.clone()),
                            asset_id: msg.asset_id.clone(),
                            side: if msg
                                .side
                                .eq_ignore_ascii_case(crate::modules::types::SIDE_BUY)
                            {
                                Side::Buy
                            } else {
                                Side::Sell
                            },
                            price: msg.price.parse::<f64>().unwrap(),
                            size: msg.original_size.parse::<f64>().unwrap(),
                        },
                    );
                }
                // crate::modules::types::MSG_UPDATE => {
                //     if let Some(order) = s.my_open_orders.get_mut(&msg.id) {
                //         order.size = msg.original_size.parse::<f64>().unwrap_or(order.size);
                //         order.price = msg.price.parse::<f64>().unwrap_or(order.price);
                //     }
                // }
                crate::modules::types::MSG_CANCELLATION => {
                    s.my_open_orders.remove(&msg.id);
                }
                _ => {}
            }
        }
    }
}

async fn is_shutting_down(state: &Arc<Mutex<AppState>>) -> bool {
    state.lock().await.shutting_down
}

fn now_millis() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
