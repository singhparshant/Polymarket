use crate::modules::logger;
use crate::modules::types::{AppState, BotCommand, MarketUpdate, Order, OrderId, Side};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

// -------------------- Trading Logic Task --------------------
pub async fn trading_logic_task(
    mut market_rx: mpsc::Receiver<MarketUpdate>,
    cmd_tx: mpsc::Sender<BotCommand>,
    state: Arc<Mutex<AppState>>,
) {
    while let Some(update) = market_rx.recv().await {
        if is_shutting_down(&state).await {
            break;
        }

        // Update market data
        {
            let mut s = state.lock().await;
            s.last_prices.insert(
                update.asset_id.clone(),
                (update.best_bid, update.best_ask, update.ts),
            );
        }

        // Execute sophisticated market-making algorithm
        execute_market_making_strategy(&update, &cmd_tx, &state).await;
    }
}

// -------------------- Market Making Strategy --------------------
pub async fn execute_market_making_strategy(
    update: &MarketUpdate,
    cmd_tx: &mpsc::Sender<BotCommand>,
    state: &Arc<Mutex<AppState>>,
) {
    // -------------------- 0. Configuration --------------------
    // These values can be moved to environment variables for easier tuning.
    const BASE_ORDER_SIZE: f64 = 500.0; // The base size of orders to place (e.g., $10).
    const EDGE_PCT: f64 = 0.02; // Quote 2% away from current best bid/ask to avoid immediate fills on wide spreads.
    const AGGRESSIVE_SKEW: f64 = 0.01; // Only applied when imbalance dollars exceed the threshold.

    // -------------------- 1. Get State & Perform Risk Checks --------------------
    let (
        yes_token,
        // no_token,
        inventory_yes,
        // inventory_no,
        max_imbalance,
        max_position,
        open_orders,
    ) = {
        let s = state.lock().await;
        if s.risk_paused || s.shutting_down {
            return;
        }

        // Get the explicit YES token from state
        let yes_token = s.yes_token.clone().unwrap_or_default();
        if yes_token.is_empty() {
            return;
        }
        // let no_token = s.token_pairs.get(&yes_token).cloned().unwrap_or_default();
        // if no_token.is_empty() {
        //     return;
        // }

        let inventory_yes = s.inventory.get(&yes_token).copied().unwrap();
        // let inventory_no = s.inventory.get(&no_token).copied().unwrap();

        let open_orders: Vec<(OrderId, String)> = s
            .my_open_orders
            .iter()
            .filter(|(_, order)| order.asset_id == yes_token)
            // .filter(|(_, order)| order.asset_id == yes_token || order.asset_id == no_token)
            .map(|(id, order)| (id.clone(), order.asset_id.clone()))
            .collect();

        (
            yes_token,
            // no_token,
            inventory_yes,
            // inventory_no,
            s.max_inventory_imbalance,
            s.max_position_size,
            open_orders,
        )
    };

    // Risk Check 1: Pause if prices are too extreme.
    if update.best_ask >= 0.98
        || update.best_ask <= 0.02
        || update.best_bid <= 0.02
        || update.best_bid >= 0.98
    {
        logger::logln(
            "Strategy: Extreme prices detected. Pausing and canceling all orders.".to_string(),
        );
        let mut s = state.lock().await;
        s.risk_paused = true;
        let _ = cmd_tx.send(BotCommand::CancelAll).await;
        return;
    }

    // Risk Check 2: Check position size and inventory imbalance
    // Convert token quantities to dollar values using current market prices
    let current_mid_price = (update.best_bid + update.best_ask) / 2.0;
    let inventory_yes_dollars = inventory_yes * current_mid_price;
    let open_orders_size = open_orders.len();
    // let inventory_no_dollars = inventory_no * (1.0 - current_mid_price);

    // let inventory_imbalance_dollars = (inventory_yes_dollars - inventory_no_dollars).abs();
    // let total_position_dollars = inventory_yes_dollars + inventory_no_dollars;

    // // Check if we should place orders (position size check)
    // let should_place_orders = total_position_dollars <= max_position;

    // if !should_place_orders {
    //     println!(
    //         "Strategy: Max position size exceeded (${:.2} > ${:.2}). Will skip order placement but continue monitoring.",
    //         total_position_dollars, max_position
    //     );
    // }

    // // Only apply aggressive skew when the dollar imbalance breaches the threshold; otherwise no skew.
    // let signed_imbalance_dollars = inventory_yes_dollars - inventory_no_dollars;
    // let apply_skew = inventory_imbalance_dollars > max_imbalance;

    // -------------------- 2. Conditional cancel based on mid-price bucket OR skew requirements --------------------
    // Compute mid and a coarse bucket; only re-quote if the bucket changed.
    let mid_bucket = (current_mid_price * 100.0).ceil() as i32; // 1-cent buckets
    let should_requote_price = {
        let mut s = state.lock().await;
        let prev = s
            .last_mid_bucket
            .insert(update.asset_id.clone(), mid_bucket);
        match prev {
            Some(last) => last != mid_bucket,
            None => true,
        }
    };

    // Also requote if we need to apply skew (inventory imbalance exceeded threshold)
    // let should_requote_skew = apply_skew;

    // Requote if either price moved OR skew is needed
    // || should_requote_skew;

    if should_requote_price {
        logger::logln(format!(
            "Strategy: Price moved. Canceling orders for requote if open orders. Current open orders={} and current mid bucket={}",
            open_orders_size, mid_bucket
        ));
        if open_orders_size > 0 {
            logger::logln("Strategy: Price moved. Canceling orders for requote.".to_string());
            let _ = cmd_tx.send(BotCommand::CancelAll).await;
        }
    } else {
        // No material move; keep existing quotes
        let last_mid_bucket = {
            let s = state.lock().await;
            s.last_mid_bucket.get(&update.asset_id).copied()
        };
        logger::logln(format!(
            "Strategy: Price not moved. Returning. Current open orders={} and current mid bucket={}, best bid={:.2}, best ask={:.2} last mid bucket={:?}",
            open_orders_size, mid_bucket, update.best_bid, update.best_ask, last_mid_bucket
        ));
        return;
    }

    // -------------------- 3. Calculate Quotes anchored to best bid/ask with asymmetrical skew --------------------
    // Base quotes: 10% away from current best bid/ask to avoid immediate fills on wide spreads.
    let our_bid_price = (update.best_bid * (1.0 - EDGE_PCT)).max(0.01);
    let our_ask_price = (update.best_ask + EDGE_PCT * update.best_ask).min(0.99);

    // Quantize to cents: floor bid, ceil ask
    let our_bid_price = ((our_bid_price * 100.0).ceil() / 100.0).max(0.01);
    let our_ask_price = ((our_ask_price * 100.0).floor() / 100.0).min(0.99);

    logger::logln(format!(
        "Strategy: Calculated quotes: bid={:.2}, ask={:.2} and best bid={:.2}, best ask={:.2} should requote={} open orders={} yes_token={}",
        our_bid_price, our_ask_price, update.best_bid, update.best_ask, should_requote_price, open_orders_size, yes_token.clone()
    ));

    // Apply aggressive, asymmetrical skew only when imbalance exceeds threshold.
    // if apply_skew {
    //     println!(
    //         "Strategy: High inventory imbalance (${:.2} > ${:.2}). Applying asymmetrical skew.",
    //         inventory_imbalance_dollars, max_imbalance
    //     );
    //     if signed_imbalance_dollars > 0.0 {
    //         // Long YES: sell YES faster (lower ask), discourage buying more (lower bid)
    //         let ask_skew = AGGRESSIVE_SKEW;
    //         let bid_skew = AGGRESSIVE_SKEW;
    //         our_ask_price = (base_ask - ask_skew).clamp(0.01, 0.99);
    //         our_bid_price = (base_bid - bid_skew).clamp(0.01, 0.99);
    //     } else {
    //         // Short YES: buy YES faster (raise bid), discourage selling more (raise ask)
    //         let bid_skew = AGGRESSIVE_SKEW;
    //         let ask_skew = AGGRESSIVE_SKEW;
    //         our_bid_price = (base_bid + bid_skew).clamp(0.01, 0.99);
    //         our_ask_price = (base_ask + ask_skew).clamp(0.01, 0.99);
    //     }
    // }

    // // Ensure our bid is not higher than our ask after all adjustments.
    // if our_bid_price >= our_ask_price {
    //     println!("Strategy: Calculated bid is higher than ask. Skipping quote placement.");
    //     return;
    // }

    // -------------------- 4. Place New Two-Sided Quotes (if position allows) --------------------
    if should_requote_price || open_orders_size == 0 {
        // Action 1: Place the BUY order for the YES token.
        let _ = cmd_tx
            .send(BotCommand::Create(Order {
                id: None,
                asset_id:
                    "48525272152376477814181507566364840492981593695809374031340296121651261876072"
                        .to_string(), //yes_token.clone(),
                side: Side::Sell,
                price: 1.0 - our_bid_price,
                size: BASE_ORDER_SIZE,
            }))
            .await;

        let _ = cmd_tx
            .send(BotCommand::Create(Order {
                id: None,
                asset_id: yes_token.clone(),
                side: Side::Sell,
                price: our_ask_price,
                size: BASE_ORDER_SIZE,
            }))
            .await;
    } else {
        logger::logln(
            "Strategy: Position limit reached. Skew calculations completed but no orders placed."
                .to_string(),
        );
    }

    // Action 2: Place the SELL order for the YES token.
    // This is done by placing a BUY order for the complementary NO token.
    // The price is calculated as 1.0 - our desired SELL price for YES.
    // let no_token_buy_price = 1.0 - our_ask_price;
    // let _ = cmd_tx
    //     .send(BotCommand::Create(Order {
    //         id: None,
    //         asset_id: no_token,
    //         side: Side::Buy,
    //         price: no_token_buy_price,
    //         size: BASE_ORDER_SIZE,
    //     }))
    //     .await;
}

// -------------------- Helper Functions --------------------
async fn is_shutting_down(state: &Arc<Mutex<AppState>>) -> bool {
    state.lock().await.shutting_down
}
