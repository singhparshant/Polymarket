use crate::modules::logger;
use crate::modules::{
    split_merge::{self, TransactionType},
    types::{AppState, BotCommand, Side},
};
use polymarket_rs_client::{ClobClient, OrderArgs, OrderType, Side as PmSide};
use rust_decimal::Decimal;
// (no-op)
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

// -------------------- Order Execution Task --------------------
pub async fn order_execution_task(
    mut cmd_rx: mpsc::Receiver<BotCommand>,
    client_pm: Arc<Mutex<ClobClient>>,
    state: Arc<Mutex<AppState>>,
) {
    while let Some(cmd) = cmd_rx.recv().await {
        match cmd {
            BotCommand::Create(order) => {
                // Convert our internal Order to Polymarket OrderArgs
                let pm_side = match order.side {
                    Side::Buy => PmSide::BUY,
                    Side::Sell => PmSide::SELL,
                };

                let order_args = OrderArgs {
                    token_id: order.asset_id.clone(),
                    price: Decimal::from_f64_retain(order.price).unwrap_or_default(),
                    size: Decimal::from_f64_retain(order.size).unwrap_or_default(),
                    side: pm_side,
                };

                // Create and post order using Polymarket client
                let client = client_pm.lock().await;
                // Create signed order (proxy-safe) and then post it
                let expiration: Option<u64> = None;
                match client
                    .create_order(&order_args, expiration, None, None)
                    .await
                {
                    Ok(response) => {
                        // Post order as GTC
                        match client.post_order(response, OrderType::GTC).await {
                            Ok(posted) => {
                                logger::logln(format!("Exec: Posted order: {:?}", posted));
                                if let Some(order_id_str) = posted
                                    .get("orderID")
                                    .or_else(|| posted.get("order_id"))
                                    .and_then(|v| v.as_str())
                                {
                                    let order_id = order_id_str.to_string();
                                    let mut s = state.lock().await;
                                    let mut updated_order = order;
                                    updated_order.id = Some(order_id.clone());
                                    s.my_open_orders.insert(order_id, updated_order);
                                    logger::logln(format!(
                                        "Exec: Placed order successfully with ID: {}",
                                        order_id_str
                                    ));
                                } else {
                                    logger::logln(format!("Exec: Order did not go through - no order ID returned. Response: {:?}", posted));
                                }
                            }
                            Err(e) => {
                                logger::logln(format!("Exec: Failed to post order: {:?}", e));
                            }
                        }
                    }
                    Err(e) => {
                        logger::logln(format!("Exec: Failed to create order: {:?}", e));
                    }
                }
            }
            BotCommand::Cancel(order_id) => {
                let client = client_pm.lock().await;
                match client.cancel(&order_id).await {
                    Ok(_) => {
                        let mut s = state.lock().await;
                        s.my_open_orders.remove(&order_id);
                        logger::logln(format!("Exec: Canceled order {}", order_id));
                    }
                    Err(e) => {
                        logger::logln(format!(
                            "Exec: Failed to cancel order {}: {:?}",
                            order_id, e
                        ));
                    }
                }
            }
            BotCommand::CancelAll => {
                let client = client_pm.lock().await;
                match client.cancel_all().await {
                    Ok(_) => {
                        let mut s = state.lock().await;
                        s.my_open_orders.clear();
                        logger::logln("Exec: Canceled all orders".to_string());
                    }
                    Err(e) => {
                        logger::logln(format!("Exec: Failed to cancel all orders: {:?}", e));
                        // Fallback: clear local state even if API call failed
                        let mut s = state.lock().await;
                        s.my_open_orders.clear();
                    }
                }
            }
            BotCommand::OrderFilled(order_id, token_id, side, price, size) => {
                // Update inventory when order is filled
                let mut s = state.lock().await;
                let current_inventory = s.inventory.get(&token_id).copied().unwrap_or(0.0);
                let inventory_change = match side {
                    Side::Buy => size,   // Bought tokens, add to inventory
                    Side::Sell => -size, // Sold tokens, subtract from inventory
                };
                s.inventory
                    .insert(token_id.clone(), current_inventory + inventory_change);
                s.my_open_orders.remove(&order_id);
                logger::logln(format!(
                    "Exec: Order {} filled - {} {} {} tokens @ {:.4}",
                    order_id,
                    crate::modules::types::side_str(&side),
                    size,
                    token_id,
                    price
                ));
            }
            BotCommand::Shutdown(condition_id, proxy_wallet) => {
                logger::logln("Exec: Shutdown received - merging before exit".to_string());
                let min_f64 = {
                    let s = state.lock().await;
                    s.inventory
                        .values()
                        .copied()
                        .fold(f64::INFINITY, f64::min)
                        .abs()
                };
                let shares_to_merge_i64: i64 = min_f64.floor() as i64;
                if shares_to_merge_i64 > 0 {
                    match split_merge::execute_split_merge(
                        TransactionType::Merge,
                        shares_to_merge_i64.to_string().as_str(),
                        &condition_id,
                        &proxy_wallet,
                        false,
                    )
                    .await
                    {
                        Ok(_) => logger::logln(format!(
                            "Exec: Merged {shares_to_merge_i64} shares successfully"
                        )),
                        Err(e) => logger::logln(format!(
                            "Exec: Failed to merge {shares_to_merge_i64} shares: {:?}",
                            e
                        )),
                    }
                }
                break;
            }
        }
    }
}

// Position/Open Orders helpers moved to data.rs
