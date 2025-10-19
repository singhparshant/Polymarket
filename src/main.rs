use dotenv::dotenv;
use polymarket_rs_client::{ClobClient, SigType};
use std::env;
use std::sync::Arc;
use tokio::signal;
use tokio::sync::{mpsc, Mutex};

mod modules;
use modules::{
    data::{fetch_current_positions, fetch_open_orders},
    execution::order_execution_task,
    monitor::monitor_task,
    persistence::{load_state, save_state},
    split_merge::{execute_split_merge, TransactionType},
    trading::trading_logic_task,
    types::{BotCommand, MarketUpdate},
    user_ws::user_ws_task,
    websocket::websocket_client_task,
};

// -------------------- Config --------------------
const WS_MARKET_URL: &str = "wss://ws-subscriptions-clob.polymarket.com/ws/market";
const HOST_PM: &str = "https://clob.polymarket.com";
const POLYGON: u64 = 137;

// Channel capacity constants for inter-task communication
// These buffer sizes are tuned for the expected message rates and API limits
const MARKET_CHANNEL_CAP: usize = 1024; // Market updates: handles burst of price changes
const COMMAND_CHANNEL_CAP: usize = 1024; // Trading commands: matches Polymarket API rate limits

/// Polymarket Market‑Making Bot — High‑Level Architecture
///
/// Components:
/// ┌─────────────────┐    MarketUpdate    ┌──────────────────┐    BotCommand    ┌─────────────────┐
/// │ WebSocket (MKTs)│ ──────────────────▶ │ Trading Logic    │ ───────────────▶ │ Order Execution │
/// │ book+price_change│                    │ (Strategy Engine)│                  │ (REST API)      │
/// └─────────────────┘                     └──────────────────┘                  └─────────────────┘
///         │                                        │                                       │
///         ▼                                        ▼                                       ▼
/// ┌─────────────────┐                     ┌──────────────────┐                  ┌─────────────────┐
/// │ User WebSocket  │  Inventory updates  │ Shared AppState  │  Periodic stats  │ Monitor Task    │
/// │ (fills/orders)  │ ───────────────────▶ │ (Arc<Mutex<>>)   │ ───────────────▶ │ (5s display)    │
/// └─────────────────┘                     └──────────────────┘                  └─────────────────┘
///
/// Data sources:
/// - Market WS (`websocket.rs`):
///   - `book` snapshots (derive best bid/ask)
///   - `price_change` (new schema) with per‑asset `best_bid`/`best_ask` for fast deltas
/// - User WS (`user_ws.rs`): order updates and fills → inventory sync
/// - REST (`execution.rs`):
///   - `/positions?user=` on startup to seed inventory (persist positions across restarts)
///   - `get_orders` for optional open‑order debugging
///
/// Strategy (trading.rs):
/// - Quotes anchor to book edges, not mid:
///   - Bid: 10% below current best bid
///   - Ask: 10% above current best ask (scaled toward 1.0)
/// - Asymmetrical skew only when dollar inventory imbalance > MAX_INVENTORY_IMBALANCE:
///   - Long YES: lower ask (sell faster), lower bid (discourage buying more YES)
///   - Short YES: raise bid (buy faster), raise ask (discourage selling more YES)
/// - Conditional replace: cancel and re‑quote only if mid‑price bucket changed
///   - Bucket = ceil(mid * 100) → reduces churn on tiny price moves
/// - Risk guards:
///   - Pause and cancel all if total position dollars > MAX_POSITION_SIZE
///   - Pause if prices are extreme (bid ≤ 0.02 or ask ≥ 0.98)
///
/// Channels:
/// - market_tx/rx (WebSocket → Logic): real‑time `MarketUpdate`
/// - cmd_tx/rx (Logic → Execution): `BotCommand::{Create,Cancel,CancelAll,Shutdown}`
///
/// Shared State (Arc<Mutex<AppState>>):
/// - my_open_orders: bot’s active orders
/// - inventory: token_id → quantity
/// - token_pairs/yes_token: explicit YES/NO mapping (1st/2nd in env `ASSETS_IDS`)
/// - last_prices: latest (bid, ask, ts) per token
/// - last_mid_bucket: ceil(mid*100) per market for conditional re‑quotes
/// - risk_paused / shutting_down: control flags
/// - max_inventory_imbalance / max_position_size: dollar risk limits
///
/// Kill switch:
/// - Ctrl+C sets `shutting_down`, issues `CancelAll`, and stops tasks cleanly.
#[tokio::main]
async fn main() {
    dotenv().ok();

    // --- Initialize Polymarket client with proxy wallet (L2 headers proxy) ---
    let private_key = env::var("PK").expect("PK environment variable not set");
    let condition_id = env::var("CONDITIONID").expect("CONDITIONID environment variable not set");
    let nonce = None;
    let client_l1 = ClobClient::with_l1_headers(HOST_PM, &private_key, POLYGON);
    let keys = client_l1
        .create_or_derive_api_key(nonce)
        .await
        .expect("derive api key failed");
    // Extract creds for user WebSocket auth before moving keys
    let ws_api_key = keys.api_key.clone();
    let ws_api_secret = keys.secret.clone();
    let ws_api_passphrase = keys.passphrase.clone();

    // Funder/proxy wallet address (the account you see on Polymarket)
    let proxy_wallet =
        env::var("PROXYWALLET").expect("Set PROXYWALLET to your proxy wallet address");
    let sig_type = SigType::PolyGnosisSafe;
    let client_pm = ClobClient::with_l2_headers_proxy(
        HOST_PM,
        &private_key,
        POLYGON,
        keys,
        Some(&proxy_wallet),
        Some(sig_type),
    );
    // (ws_* variables already captured above)
    let client_pm = Arc::new(Mutex::new(client_pm));

    // Assets to subscribe (comma-separated ASSETS_IDS in env) - should be exactly 2 tokens (yes/no pair)
    // let assets_ids: Vec<String> = env::var("ASSETS_IDS")
    //     .unwrap_or_default()
    //     .split(',')
    //     .filter_map(|s| {
    //         let t = s.trim();
    //         if t.is_empty() {
    //             None
    //         } else {
    //             Some(t.to_string())
    //         }
    //     })
    //     .collect();

    let assets_id: String = env::var("ASSETS_IDS").unwrap_or_default();
    // if assets_id.len() != 1 {
    //     panic!("ASSETS_IDS must contain exactly 1 token");
    // }

    // Initialize state with token pairs and risk parameters (load snapshot if present)
    let mut initial_state = load_state().unwrap_or_default();
    // Normalize flags on startup (avoid stale persisted shutdown/pause)
    initial_state.shutting_down = false;
    initial_state.risk_paused = false;

    // Explicit mapping: first token = YES, second token = NO
    let yes_token = assets_id.clone();
    // let no_token = assets_ids[1].clone();
    // initial_state
    //     .token_pairs
    //     .insert(yes_token.clone(), no_token.clone());
    // initial_state
    //     .token_pairs
    //     .insert(no_token.clone(), yes_token.clone());

    // Store the explicit YES token for reference (clone to avoid moving)
    initial_state.yes_token = Some(yes_token.clone());
    initial_state.max_inventory_imbalance = env::var("MAX_INVENTORY_IMBALANCE")
        .unwrap_or_else(|_| "25.0".to_string())
        .parse()
        .unwrap_or(25.0);
    initial_state.max_position_size = env::var("MAX_POSITION_SIZE")
        .unwrap_or_else(|_| "50.0".to_string())
        .parse()
        .unwrap_or(50.0);

    // Ensure inventory keys exist
    initial_state
        .inventory
        .entry(assets_id.clone())
        .or_insert(0.0);
    // initial_state
    //     .inventory
    //     .entry(assets_ids[1].clone())
    //     .or_insert(0.0);

    // Fetch current positions from Polymarket API
    println!("Fetching current positions from Polymarket...");
    match fetch_current_positions(&client_pm, &assets_id, &condition_id).await {
        Ok(positions) => {
            println!("Current positions: {:?}", positions);
            for (token_id, quantity) in positions {
                initial_state.inventory.insert(token_id, quantity);
            }
        }
        Err(e) => {
            println!("Warning: Could not fetch current positions: {:?}", e);
            println!("Starting with zero inventory. Manual position sync recommended.");
        }
    }

    // Also fetch open orders for debugging
    // println!("Fetching open orders from Polymarket...");
    // match fetch_open_orders(&client_pm, &assets_ids).await {
    //     Ok(orders) => match serde_json::to_string_pretty(&orders) {
    //         Ok(orders_str) => println!("Open orders: {}", orders_str),
    //         Err(e) => println!("Error formatting open orders: {:?}", e),
    //     },
    //     Err(e) => {
    //         println!("Warning: Could not fetch open orders: {:?}", e);
    //     }
    // }

    // Execute split/merge transaction
    let amount = "500"; // Amount in USDC
    let neg_risk = false; // Set to true if using neg risk adapter

    // split transaction
    match execute_split_merge(
        TransactionType::Split,
        amount,
        &condition_id,
        &proxy_wallet,
        neg_risk,
    )
    .await
    {
        Ok(tx_hash) => {
            println!("Split transaction successful: {:?}", tx_hash);
            // if let Some(k) = initial_state.token_pairs.get(&yes_token).cloned() {
            initial_state
                .inventory
                .insert(yes_token, amount.parse::<f64>().unwrap());
            // }
            // if let Some(k) = initial_state.token_pairs.get(&no_token).cloned() {
            //     initial_state
            //         .inventory
            //         .insert(k, amount.parse::<f64>().unwrap());
            // }
        }
        Err(e) => println!("Split transaction failed: {:?}", e),
    }

    // initial_state
    //     .inventory
    //     .insert(yes_token, amount.parse::<f64>().unwrap());

    // Example merge transaction
    // match execute_split_merge(
    //     TransactionType::Merge,
    //     amount,
    //     &condition_id,
    //     &proxy_wallet,
    //     neg_risk,
    // )
    // .await
    // {
    //     Ok(tx_hash) => println!("Merge transaction successful: {:?}", tx_hash),
    //     Err(e) => println!("Merge transaction failed: {:?}", e),
    // }

    // --- Shared state and channels ---
    let state = Arc::new(Mutex::new(initial_state));

    // Channel 1: Market Data Flow (WebSocket → Trading Logic)
    // Purpose: Streams real-time market updates from Polymarket WebSocket feed
    // Data: MarketUpdate (market_id, best_bid, best_ask, timestamp)
    // Flow: websocket_client_task → trading_logic_task
    // Capacity: 1024 messages (handles burst of market updates)
    let (market_tx, market_rx) = mpsc::channel::<MarketUpdate>(MARKET_CHANNEL_CAP);

    // Channel 2: Command Flow (Trading Logic → Order Execution)
    // Purpose: Sends trading decisions as executable commands
    // Data: BotCommand (Create/Cancel/CancelAll/OrderFilled/Shutdown)
    // Flow: trading_logic_task → order_execution_task
    // Capacity: 1024 commands (matches API rate limits)
    let (cmd_tx, cmd_rx) = mpsc::channel::<BotCommand>(COMMAND_CHANNEL_CAP);

    println!("Spawning tasks...");

    // --- Spawn tasks ---
    let ws_state = Arc::clone(&state);
    let ws_tx = market_tx.clone();
    let ws_asset_id = assets_id.clone();
    let ws_handle = tokio::spawn(async move {
        websocket_client_task(WS_MARKET_URL.to_string(), ws_asset_id, ws_tx, ws_state).await;
    });

    println!("Spawning trading logic task...");
    let logic_state = Arc::clone(&state);
    let logic_cmd_tx = cmd_tx.clone();
    let logic_handle = tokio::spawn(async move {
        trading_logic_task(market_rx, logic_cmd_tx, logic_state).await;
    });

    println!("Spawning order execution task...");
    let exec_state = Arc::clone(&state);
    let exec_client = Arc::clone(&client_pm);
    let exec_handle = tokio::spawn(async move {
        order_execution_task(cmd_rx, exec_client, exec_state).await;
    });

    // --- Monitor task ---
    println!("Spawning monitor task...");
    let monitor_state = Arc::clone(&state);
    let monitor_handle = tokio::spawn(async move {
        monitor_task(monitor_state).await;
    });
    // --- User WS task (authenticated) ---
    println!("Spawning user WS task...");
    let user_state = Arc::clone(&state);
    let user_cmd_tx = cmd_tx.clone();
    tokio::spawn(async move {
        user_ws_task(
            ws_api_key,
            ws_api_secret,
            ws_api_passphrase,
            user_state,
            user_cmd_tx,
        )
        .await;
    });

    // --- Kill switch ---
    println!("Sending kill switch...");
    signal::ctrl_c()
        .await
        .expect("install Ctrl+C handler failed");
    {
        println!("Setting shutting down and risk paused...");
        let mut s = state.lock().await;
        s.shutting_down = true;
        s.risk_paused = true;
        let _ = save_state(&s);
    }
    println!("Sending cancel all and shutdown commands...");
    let _ = cmd_tx.send(BotCommand::CancelAll).await;
    let _ = cmd_tx
        .send(BotCommand::Shutdown(condition_id, proxy_wallet))
        .await;
    drop(market_tx);
    drop(cmd_tx);

    println!("Waiting for tasks to complete...");
    let _ = ws_handle.await;
    let _ = logic_handle.await;
    let _ = exec_handle.await;
    let _ = monitor_handle.await;
    println!("Tasks completed.");
}
